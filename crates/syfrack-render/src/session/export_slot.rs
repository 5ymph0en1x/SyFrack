//! Bounded GPU work/readback slots. A slot is reused only after its map has
//! completed and the mapped view has been dropped and unmapped.
use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc,
};

pub(crate) struct TileResult {
    pub tile: Tile,
    pub output: RenderOutput,
    pub guides: Vec<(HdrImage, HdrImage)>,
    pub dispatches: u32,
    pub max_dispatch: Duration,
}

struct GuideBuffers {
    uniforms: wgpu::Buffer,
    albedo: wgpu::Buffer,
    normals: wgpu::Buffer,
    group: wgpu::BindGroup,
}

pub(crate) struct ExportSlot {
    session: RenderSession,
    staging: wgpu::Buffer,
    guides: Vec<GuideBuffers>,
    pending: Option<(
        mpsc::Receiver<std::result::Result<(), wgpu::BufferAsyncError>>,
        wgpu::SubmissionIndex,
    )>,
    tile: Tile,
    capacity: u32,
    ranges: Vec<std::ops::Range<u64>>,
    next_grid_size: u32,
    next_spd: u32,
}

impl ExportSlot {
    pub fn new(renderer: Renderer, scene: &Scene, cfg: &RenderConfig, size: u32) -> Result<Self> {
        let mut local = cfg.clone();
        local.width = size;
        local.height = size;
        local.tile_size = size;
        local.guiding = false;
        local.time_budget = None;
        let session = RenderSession::new(renderer, scene, &local)?;
        let ctx = &session.renderer.ctx;
        let capacity = u64::from(size) * u64::from(size);
        let layer_bytes = if scene.dimension == syfrack_core::Dimension::ThreeD
            && scene.atmosphere.fog_density > 0.0
        {
            96
        } else {
            0
        };
        let guide_bytes = if cfg.denoise && scene.dimension == syfrack_core::Dimension::ThreeD {
            64
        } else {
            0
        };
        let (staging, guides) = session.renderer.with_scopes(|| {
            let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pooled export readback"),
                size: capacity * (36 + layer_bytes + guide_bytes),
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut guides = Vec::new();
            if cfg.denoise && scene.dimension == syfrack_core::Dimension::ThreeD {
                for _ in 0..2 {
                    let buffer = |label, size, usage| {
                        ctx.device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some(label),
                            size,
                            usage,
                            mapped_at_creation: false,
                        })
                    };
                    let uniforms = buffer(
                        "tile guide uniforms",
                        std::mem::size_of::<Uniforms>() as u64,
                        wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    );
                    let usage = wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::COPY_SRC
                        | wgpu::BufferUsages::COPY_DST;
                    let albedo = buffer("pooled tile albedo", capacity * 16, usage);
                    let normals = buffer("pooled tile shading normals", capacity * 16, usage);
                    let entries = [
                        (0, uniforms.as_entire_binding()),
                        (1, albedo.as_entire_binding()),
                        (2, session.depth.as_entire_binding()),
                        (3, normals.as_entire_binding()),
                        (4, session.params_buf.binding()),
                        (5, session.texture_params_buf.binding()),
                        (16, session.params_buf.overflow_binding()),
                        (17, session.texture_params_buf.overflow_binding()),
                        (6, session.environment.buffer.as_entire_binding()),
                        (
                            7,
                            wgpu::BindingResource::TextureView(&session.particles.view),
                        ),
                        (
                            8,
                            wgpu::BindingResource::Sampler(&session.particles.sampler),
                        ),
                        (
                            9,
                            wgpu::BindingResource::TextureView(&session.particles.lighting),
                        ),
                        (
                            10,
                            wgpu::BindingResource::TextureView(
                                &session.particles.microstructure.view,
                            ),
                        ),
                        (
                            11,
                            wgpu::BindingResource::Sampler(
                                &session.particles.microstructure.sampler,
                            ),
                        ),
                        (12, session.volume.as_entire_binding()),
                        (13, session.volume_moments.as_entire_binding()),
                        (14, session.surface.as_entire_binding()),
                        (15, session.volume_components.as_entire_binding()),
                    ]
                    .into_iter()
                    .map(|(binding, resource)| wgpu::BindGroupEntry { binding, resource })
                    .collect::<Vec<_>>();
                    let group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("tile material guides"),
                        layout: &session.renderer.layouts.bind_group_layout,
                        entries: &entries,
                    });
                    guides.push(GuideBuffers {
                        uniforms,
                        albedo,
                        normals,
                        group,
                    });
                }
            }
            Ok((staging, guides))
        })?;
        Ok(Self {
            session,
            staging,
            guides,
            pending: None,
            tile: Tile {
                x: 0,
                y: 0,
                w: 0,
                h: 0,
                done: 0,
            },
            capacity: size,
            ranges: Vec::new(),
            next_grid_size: if cfg.integrator == crate::ComputeIntegrator::Wavefront {
                64.min(size)
            } else {
                initial_tile(scene, cfg.tile_size)
            },
            next_spd: 1,
        })
    }

    pub fn identity(&self) -> (u64, u64, u64) {
        (
            self.session.transport_identity(),
            self.session.environment.fingerprint,
            self.session.particles.fingerprint,
        )
    }

    pub fn producer_identity(&self) -> crate::export::ProducerIdentity {
        let s = &self.session;
        crate::export::ProducerIdentity::new(
            &s.renderer.ctx.info,
            s.wavefront
                .as_ref()
                .map_or(s.pipeline.implementation_fingerprint, |w| w.fingerprint()),
            if s.wavefront.is_some() {
                [64, 1]
            } else {
                s.pipeline.workgroup
            },
        )
    }

    pub fn cpu_pipeline(&self) -> (Arc<CompiledPipeline>, bool) {
        (self.session.pipeline.clone(), !self.guides.is_empty())
    }

    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }

    pub fn cost(&self) -> (u32, Duration) {
        (self.session.cfg.samples_per_pixel, self.session.elapsed)
    }

    pub fn set_budget(&mut self, milliseconds: u32) {
        self.session.cfg.dispatch_budget_ms = milliseconds;
    }

    pub fn dispatch_plan(&self) -> (u32, u32) {
        (
            self.session
                .grid
                .tiles
                .iter()
                .map(|t| t.w.max(t.h))
                .max()
                .unwrap_or(self.next_grid_size)
                .clamp(crate::tiles::MIN_TILE_SIZE, self.capacity),
            self.session.spd.clamp(1, 8),
        )
    }

    pub fn set_dispatch_plan(&mut self, (size, samples): (u32, u32)) {
        self.next_grid_size = size.clamp(crate::tiles::MIN_TILE_SIZE, self.capacity);
        self.next_spd = samples.clamp(1, self.session.cfg.samples_per_dispatch.clamp(1, 8));
    }

    pub fn set_execution_profile(
        &mut self,
        profile: Arc<crate::execution_profile::ExecutionProfile>,
    ) {
        self.session.execution_profile = Some(profile);
    }

    /// Trigger a software driver's lazy JIT before claiming shared work. This
    /// result is discarded and cannot hold the final image behind a cold JIT.
    pub fn warm_cpu(&mut self, full: [u32; 2], stop: &AtomicBool) -> Result<()> {
        let plan = (self.next_grid_size, self.next_spd);
        let target = self.session.cfg.samples_per_pixel;
        self.session.cfg.samples_per_pixel = 1;
        let w = full[0].min(self.capacity).min(8);
        let h = full[1].min(self.capacity).min(8);
        let tile = Tile {
            x: (full[0] - w) / 2,
            y: (full[1] - h) / 2,
            w,
            h,
            done: 0,
        };
        let result = self.render(tile, full, stop).and_then(|rendered| {
            if rendered {
                self.finish(true)?;
            }
            Ok(())
        });
        self.session.cfg.samples_per_pixel = target;
        (self.next_grid_size, self.next_spd) = plan;
        result
    }

    pub fn working_bytes(&self) -> u64 {
        let s = &self.session;
        self.staging.size()
            + s.wavefront.as_ref().map_or(0, |w| w.working_bytes())
            + [
                &s.accum,
                &s.depth,
                &s.aux,
                &s.volume,
                &s.volume_moments,
                &s.surface,
                &s.volume_components,
            ]
            .into_iter()
            .map(wgpu::Buffer::size)
            .sum::<u64>()
            + self
                .guides
                .iter()
                .map(|g| g.albedo.size() + g.normals.size())
                .sum::<u64>()
    }

    /// Render linear radiance, preserving global camera coordinates and RNG
    /// seeds while addressing compact tile storage with flag bit 3.
    pub fn render(&mut self, tile: Tile, full: [u32; 2], cancelled: &AtomicBool) -> Result<bool> {
        anyhow::ensure!(!self.pending(), "export slot is still mapped");
        anyhow::ensure!(
            tile.w <= self.capacity && tile.h <= self.capacity,
            "tile exceeds slot capacity"
        );
        self.tile = tile;
        let s = &mut self.session;
        s.cfg.width = tile.w;
        s.cfg.height = tile.h;
        s.uniforms.image_size = full;
        s.uniforms.camera.params[1] = full[0] as f32 / full[1] as f32;
        s.uniforms.flags |= 8;
        s.uniforms.fractal.kind[3] = tile.w;
        s.uniforms.guiding_record_round = tile.x;
        s.uniforms.guiding_training_end = tile.y;
        // Reuse this slot's measured dispatch plan. Repeating the cold ramp on
        // every tile creates many tiny submissions and starves GPU work while
        // a software CPU driver is also scheduling host threads. Each session
        // still measures dispatches and backs off for a more expensive region.
        s.grid = TileGrid::new(tile.w, tile.h, self.next_grid_size);
        s.spd = self.next_spd;
        s.want_halve = false;
        s.worst = 0.0;
        s.reset_seen();
        s.dispatches = 0;
        s.max_dispatch = Duration::ZERO;
        s.elapsed = Duration::ZERO;
        s.clear_buffers()?;
        while !s.is_done() {
            if cancelled.load(Ordering::Acquire) {
                return Ok(false);
            }
            s.step(Duration::from_millis(50))?;
        }
        let ctx = &s.renderer.ctx;
        let pixels = tile.w as u64 * tile.h as u64;
        let layers = s.scene.dimension == syfrack_core::Dimension::ThreeD
            && s.scene.atmosphere.fog_density > 0.0;
        let mut encoder = ctx.device.create_command_encoder(&Default::default());
        self.ranges.clear();
        let mut offset = 0;
        let mut copy = |encoder: &mut wgpu::CommandEncoder, buffer: &wgpu::Buffer, bytes| {
            self.ranges.push(offset..offset + bytes);
            if bytes > 0 {
                encoder.copy_buffer_to_buffer(buffer, 0, &self.staging, offset, bytes);
            }
            offset += bytes;
        };
        for (i, buffer) in [
            &s.accum,
            &s.depth,
            &s.aux,
            &s.volume,
            &s.volume_moments,
            &s.surface,
            &s.volume_components,
        ]
        .into_iter()
        .enumerate()
        {
            let bytes = if i >= 3 && !layers {
                0
            } else if i == 1 {
                pixels * 4
            } else if i == 6 {
                pixels * 48
            } else {
                pixels * 16
            };
            copy(&mut encoder, buffer, bytes);
        }
        for (i, g) in self.guides.iter().enumerate() {
            let mut uniforms = s.uniforms;
            uniforms.set_tile(tile.x, tile.y, tile.w, tile.h, 0, 1);
            if i == 1 {
                uniforms.flags |= 2;
            }
            ctx.queue
                .write_buffer(&g.uniforms, 0, bytemuck::bytes_of(&uniforms));
            encoder.copy_buffer_to_buffer(&s.aux, 0, &g.normals, 0, pixels * 16);
            {
                let pipeline = s.pipeline.export_pipeline(&ctx.device, &s.renderer.layouts);
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &g.group, &[]);
                pass.dispatch_workgroups(tile.w.div_ceil(8), tile.h.div_ceil(8), 1);
            }
            copy(&mut encoder, &g.albedo, pixels * 16);
            copy(&mut encoder, &g.normals, pixels * 16);
        }
        let submission = ctx.queue.submit([encoder.finish()]);
        let (tx, rx) = mpsc::channel();
        self.staging
            .slice(..offset)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
        self.pending = Some((rx, submission));
        Ok(true)
    }

    pub fn finish(&mut self, wait: bool) -> Result<Option<TileResult>> {
        let Some((rx, submission)) = self.pending.as_ref() else {
            return Ok(None);
        };
        let ctx = &self.session.renderer.ctx;
        ctx.device
            .poll(if wait {
                wgpu::PollType::Wait {
                    submission_index: Some(submission.clone()),
                    timeout: Some(Duration::from_secs(10)),
                }
            } else {
                wgpu::PollType::Poll
            })
            .map_err(|e| anyhow!("export readback poll: {e}"))?;
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) if !wait => return Ok(None),
            Err(e) => return Err(anyhow!("export map callback: {e}")),
        };
        self.pending = None;
        result.context("export map_async")?;
        ctx.check()?;
        let end = self.ranges.last().unwrap().end;
        let data = self
            .staging
            .slice(..end)
            .get_mapped_range()
            .context("export mapped range")?;
        let raw: Vec<_> = self
            .ranges
            .iter()
            .map(|r| &data[r.start as usize..r.end as usize])
            .collect();
        let decoded = self.session.decode_readback(&raw[..7]);
        let guides = raw[7..]
            .chunks_exact(2)
            .map(|pair| {
                (
                    HdrImage::from_data(
                        self.tile.w,
                        self.tile.h,
                        bytemuck::pod_collect_to_vec(pair[0]),
                    ),
                    HdrImage::from_data(
                        self.tile.w,
                        self.tile.h,
                        bytemuck::pod_collect_to_vec(pair[1]),
                    ),
                )
            })
            .collect();
        drop(data);
        self.staging.unmap();
        self.next_grid_size = self
            .session
            .grid
            .tiles
            .iter()
            .map(|t| t.w.max(t.h))
            .max()
            .unwrap_or(self.next_grid_size)
            .clamp(crate::tiles::MIN_TILE_SIZE, self.capacity);
        self.next_spd = self.session.spd.clamp(1, 8);
        Ok(Some(TileResult {
            tile: self.tile,
            output: decoded?,
            guides,
            dispatches: self.session.dispatches,
            max_dispatch: self.session.max_dispatch,
        }))
    }
}

impl Drop for ExportSlot {
    fn drop(&mut self) {
        // Completed readbacks were already unmapped by finish(). Destruction
        // cancels outstanding maps, including callbacks that failed before
        // collection; unmapping an idle buffer would poison the shared device.
        if self.pending.is_some() {
            self.staging.destroy();
        }
    }
}
