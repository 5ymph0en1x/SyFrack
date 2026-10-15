use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use syfrack_core::{HdrImage, Scene};

use crate::pipeline_cache::CompiledPipeline;
use crate::renderer::{Progress, RenderConfig, RenderOutput, Renderer};
use crate::tiles::{Tile, TileGrid};
use crate::uniforms::Uniforms;

// Spatial growth gets half the slow-dispatch budget. Four conservative old
// tile times overestimate the benefit of occupying more GPU workgroups, while
// still leaving a factor-of-two margin before the automatic reduction.
/// First dispatches use tiles of at most this size and one sample, then ramp up.
const START_TILE: u32 = 16;
// Flat fields have no ray marching or secondary transport. Starting them with
// 16x16 tiles makes a 4K image require over 32,000 submissions for its first
// sample alone. Keep a bounded 64x64 start, with the same measured backoff.
fn initial_tile(scene: &Scene, requested: u32) -> u32 {
    let limit = if scene.dimension == syfrack_core::Dimension::TwoD {
        64
    } else {
        START_TILE
    };
    requested.min(limit)
}
const STATE_FILE: &str = "state.ron";
const STATE_FILES: [&str; 9] = [
    STATE_FILE,
    "accum.bin",
    "depth.bin",
    "normals.bin",
    "guiding.bin",
    "volume.bin",
    "volume-moments.bin",
    "surface.bin",
    "volume-components.bin",
];

/// Refuses a directory that holds anything other than state files.
pub fn check_replaceable(dir: &Path) -> Result<()> {
    if dir.file_name().is_none() {
        anyhow::bail!("checkpoint path {} has no final component", dir.display());
    }
    if dir.is_dir() {
        for e in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
            let name = e?.file_name();
            if !STATE_FILES.iter().any(|f| name == *f) {
                anyhow::bail!(
                    "{} is not a SyFrack state directory (contains {:?}); refusing to replace it",
                    dir.display(),
                    name
                );
            }
        }
    }
    Ok(())
}

/// Everything needed to resume a render except the GPU buffers, which are
/// saved as raw little-endian f32 files next to this.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionState {
    /// Absent in legacy checkpoints, which remain readable for gallery/info.
    #[serde(default)]
    pub shader_fingerprint: Option<u64>,
    /// Hash of the actual uploaded panorama pixels/CDF, zero without HDRI.
    #[serde(default)]
    pub environment_fingerprint: u64,
    #[serde(default)]
    pub particle_fingerprint: u64,
    #[serde(default)]
    pub guiding_updates: u32,
    #[serde(default)]
    pub guiding_fingerprint: Option<u64>,
    #[serde(default)]
    pub guiding_version: u32,
    pub scene: Scene,
    pub cfg: RenderConfig,
    pub tiles: Vec<Tile>,
    pub tile_size: u32,
    pub spd: u32,
    pub dispatches: u32,
    pub elapsed_secs: f64,
}

/// A render in progress: one scene, one image size, GPU buffers, and the
/// tile schedule. Advance it with `step`, inspect with `progress`, read the
/// image with `readback`, persist with `save` / `resume`, restart with `reset`.
struct FlameWork {
    receiver: std::sync::mpsc::Receiver<(crate::flame::FlameAccumulator, HdrImage)>,
    cancelled: Arc<std::sync::atomic::AtomicBool>,
    started: Instant,
}
impl Drop for FlameWork {
    fn drop(&mut self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

pub struct RenderSession {
    flame: Option<crate::flame::FlameAccumulator>,
    flame_work: Option<FlameWork>,
    renderer: Renderer,
    scene: Scene,
    cfg: RenderConfig,
    uniforms: Uniforms,
    uniform_buf: wgpu::Buffer,
    /// The compiled distance estimator of the scene's graph topology.
    pipeline: Arc<CompiledPipeline>,
    guiding_pipelines: Option<Arc<crate::pipeline_cache::GuidingPipelines>>,
    /// Graph parameter values (binding 4), laid out by `pipeline.layout`.
    params_buf: wgpu::Buffer,
    /// Independent material graph values (binding 5).
    texture_params_buf: wgpu::Buffer,
    environment: Arc<crate::environment::PreparedEnvironment>,
    particles: Arc<crate::particles::PreparedParticles>,
    accum: wgpu::Buffer,
    depth: wgpu::Buffer,
    aux: wgpu::Buffer,
    volume: wgpu::Buffer,
    volume_moments: wgpu::Buffer,
    surface: wgpu::Buffer,
    volume_components: wgpu::Buffer,
    volume_components_enabled: bool,
    bind_group: wgpu::BindGroup,
    grid: TileGrid,
    spd: u32,
    want_halve: bool,
    /// Worst observed tile time per sample at the current tile size. Using
    /// absolute tile cost also covers clipped edges without extrapolating a
    /// tiny edge dispatch's fixed submission overhead to a full square.
    worst: f64,
    /// Which tiles have been dispatched since the last change of `spd` or tile size.
    seen: Vec<bool>,
    /// Measured seconds per sample for each tile in the current layout.
    tile_cost: Vec<f64>,
    export_guides_cache: std::sync::Mutex<[Option<(HdrImage, HdrImage)>; 2]>,
    dispatches: u32,
    elapsed: Duration,
    max_dispatch: Duration,
    guiding_updates: u32,
    guiding_active_cells: usize,
    guiding_spatial_leaves: usize,
    guiding_training_records: usize,
    guiding_trusted_cells: usize,
    guiding_profile: crate::guiding::GuidingProfile,
}

impl RenderSession {
    pub fn new(renderer: Renderer, scene: &Scene, cfg: &RenderConfig) -> Result<Self> {
        scene.validate().context("scene validation")?;
        let mut scene = scene.clone();
        scene.resolve_asset_paths(Path::new("."))?;
        let scene = &scene;
        let environment = renderer.environment_for(scene)?;
        let particles = renderer.particles_for(scene)?;
        let device = &renderer.ctx.device;
        cfg.validate(&device.limits())
            .context("render configuration")?;
        let pixels = (cfg.width as u64) * (cfg.height as u64);
        if let Some(flame) = &scene.canvas.flame {
            if pixels
                .checked_mul(cfg.samples_per_pixel as u64)
                .and_then(|v| v.checked_mul(flame.points_per_sample as u64))
                .and_then(|v| v.checked_mul(65536))
                .is_none()
            {
                anyhow::bail!("flame point budget exceeds the exact integer accumulator capacity");
            }
        }
        let volume_components_enabled = pixels * 48
            <= u64::from(device.limits().max_storage_buffer_binding_size)
            && pixels * 48 <= device.limits().max_buffer_size;
        let mut uniforms =
            Uniforms::from_scene(scene, cfg.width, cfg.height, cfg.max_bounces, cfg.max_steps);
        uniforms.fractal.p0[0] = cfg.pixel_tolerance;
        if volume_components_enabled {
            uniforms.flags |= 4;
        }
        environment.apply(&mut uniforms);
        let graph = scene.fractal.to_graph();
        let pipeline = renderer.pipeline_for_scene(scene)?;
        let guiding_pipelines = if cfg.guiding {
            Some(pipeline.guiding_pipelines(device, &renderer.layouts)?)
        } else {
            None
        };
        let (
            uniform_buf,
            params_buf,
            texture_params_buf,
            accum,
            depth,
            aux,
            volume,
            volume_moments,
            surface,
            volume_components,
            bind_group,
        ) = renderer.with_scopes(|| {
            let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("uniforms"),
                size: std::mem::size_of::<Uniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let storage = |label: &str, size: u64| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size,
                    usage: wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::COPY_SRC
                        | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            };
            let params_buf = renderer.graph_params_buffer(&pipeline.layout.pack(&graph));
            let texture_values = match (&pipeline.texture_layout, &scene.material.texture) {
                (Some(layout), Some(texture)) => layout.pack(&texture.graph),
                _ => [0.0; crate::graph_wgsl::PARAM_COUNT],
            };
            let texture_params_buf = renderer.graph_params_buffer(&texture_values);
            let accum = storage("accum", pixels * 16);
            let depth = storage(
                "depth + guiding",
                pixels * 4
                    + if cfg.guiding {
                        crate::guiding::BYTES
                    } else {
                        0
                    },
            );
            let aux = storage("aux", pixels * 16);
            let volume = storage("volume radiance", pixels * 16);
            let volume_moments = storage("volume second moments", pixels * 16);
            let surface = storage("surface radiance", pixels * 16);
            let volume_components = storage(
                "volume components and dispersion",
                if volume_components_enabled {
                    pixels * 48
                } else {
                    48
                },
            );
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("pathtrace"),
                layout: &renderer.layouts.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: accum.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: depth.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: aux.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 12,
                        resource: volume.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 13,
                        resource: volume_moments.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 14,
                        resource: surface.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 15,
                        resource: volume_components.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: params_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: texture_params_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: environment.buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: wgpu::BindingResource::TextureView(&particles.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 8,
                        resource: wgpu::BindingResource::Sampler(&particles.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 9,
                        resource: wgpu::BindingResource::TextureView(&particles.lighting),
                    },
                    wgpu::BindGroupEntry {
                        binding: 10,
                        resource: wgpu::BindingResource::TextureView(
                            &particles.microstructure.view,
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 11,
                        resource: wgpu::BindingResource::Sampler(&particles.microstructure.sampler),
                    },
                ],
            });
            Ok((
                uniform_buf,
                params_buf,
                texture_params_buf,
                accum,
                depth,
                aux,
                volume,
                volume_moments,
                surface,
                volume_components,
                bind_group,
            ))
        })?;
        let mut session = Self {
            flame_work: None,
            flame: if scene.canvas.flame.is_some() {
                Some(crate::flame::FlameAccumulator::new(
                    &scene.canvas,
                    scene.seed,
                    cfg.width,
                    cfg.height,
                )?)
            } else {
                None
            },
            renderer,
            scene: scene.clone(),
            cfg: cfg.clone(),
            uniforms,
            uniform_buf,
            pipeline,
            guiding_pipelines,
            params_buf,
            texture_params_buf,
            environment,
            particles,
            accum,
            depth,
            aux,
            volume,
            volume_moments,
            surface,
            volume_components,
            volume_components_enabled,
            bind_group,
            grid: TileGrid::new(cfg.width, cfg.height, initial_tile(scene, cfg.tile_size)),
            spd: 1,
            want_halve: false,
            worst: 0.0,
            seen: Vec::new(),
            tile_cost: Vec::new(),
            export_guides_cache: std::sync::Mutex::new([None, None]),
            dispatches: 0,
            elapsed: Duration::ZERO,
            max_dispatch: Duration::ZERO,
            guiding_updates: 0,
            guiding_active_cells: 0,
            guiding_spatial_leaves: 0,
            guiding_training_records: 0,
            guiding_trusted_cells: 0,
            guiding_profile: Default::default(),
        };
        session.update_guiding_uniforms();
        session.reset_seen();
        session.clear_buffers()?;
        Ok(session)
    }

    fn step_flame(&mut self, budget: Duration) -> Result<bool> {
        if self.is_done() {
            return Ok(true);
        }
        if self.flame_work.is_none() {
            let mut flame = self.flame.take().unwrap();
            let count = (self.cfg.samples_per_pixel - flame.passes)
                .min(self.cfg.samples_per_dispatch.max(1));
            let (sender, receiver) = std::sync::mpsc::channel();
            let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let token = cancelled.clone();
            std::thread::Builder::new()
                .name("syfrack-orbits".into())
                .spawn(move || {
                    flame.advance_with_cancel(count, &token);
                    if token.load(std::sync::atomic::Ordering::Relaxed) {
                        return;
                    }
                    let image = flame.reconstruct();
                    let _ = sender.send((flame, image));
                })?;
            self.flame_work = Some(FlameWork {
                receiver,
                cancelled,
                started: Instant::now(),
            });
        }
        let (flame, image) = match self
            .flame_work
            .as_ref()
            .unwrap()
            .receiver
            .recv_timeout(budget.min(Duration::from_millis(50)))
        {
            Ok(result) => result,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return Ok(false),
            Err(error) => return Err(anyhow!("flame worker: {error}")),
        };
        let work = self.flame_work.take().unwrap();
        let samples = flame.passes;
        self.flame = Some(flame);
        let sums: Vec<[f32; 4]> = image
            .data
            .iter()
            .map(|p| {
                [
                    p[0] * samples as f32,
                    p[1] * samples as f32,
                    p[2] * samples as f32,
                    samples as f32,
                ]
            })
            .collect();
        self.renderer
            .ctx
            .queue
            .write_buffer(&self.accum, 0, bytemuck::cast_slice(&sums));
        self.renderer
            .ctx
            .queue
            .write_buffer(&self.surface, 0, bytemuck::cast_slice(&sums));
        for tile in &mut self.grid.tiles {
            tile.done = samples;
        }
        self.dispatches += 1;
        self.elapsed += work.started.elapsed();
        self.max_dispatch = self.max_dispatch.max(work.started.elapsed());
        self.renderer.ctx.check()?;
        Ok(self.is_done())
    }

    fn reset_seen(&mut self) {
        self.seen = vec![false; self.grid.tiles.len()];
        self.tile_cost = vec![0.0; self.grid.tiles.len()];
    }

    fn clear_buffers(&self) -> Result<()> {
        let pixels = (self.cfg.width as usize) * (self.cfg.height as usize);
        let ctx = &self.renderer.ctx;
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        enc.clear_buffer(&self.accum, 0, None);
        enc.clear_buffer(&self.depth, 0, None);
        enc.clear_buffer(&self.volume, 0, None);
        enc.clear_buffer(&self.volume_moments, 0, None);
        enc.clear_buffer(&self.volume_components, 0, None);
        enc.clear_buffer(&self.surface, 0, None);
        ctx.queue.submit([enc.finish()]);
        let miss: Vec<[f32; 4]> = vec![[0.0, 0.0, 0.0, -1.0]; pixels];
        ctx.queue
            .write_buffer(&self.aux, 0, bytemuck::cast_slice(&miss));
        let depth: Vec<f32> = vec![-1.0; pixels];
        ctx.queue
            .write_buffer(&self.depth, 0, bytemuck::cast_slice(&depth));
        ctx.check()
    }

    pub fn is_done(&self) -> bool {
        self.grid.next_pending(self.cfg.samples_per_pixel).is_none()
    }

    /// The f32 RGBA accumulation buffer (alpha counts samples). The studio's
    /// display pass reads it directly; never write to it from outside.
    pub fn accumulation_buffer(&self) -> &wgpu::Buffer {
        &self.accum
    }

    pub fn image_size(&self) -> (u32, u32) {
        (self.cfg.width, self.cfg.height)
    }

    pub fn scene(&self) -> &Scene {
        &self.scene
    }
    pub fn config(&self) -> &RenderConfig {
        &self.cfg
    }

    /// Change only scheduling, including after resuming a saved image.
    pub fn set_dispatch_budget_ms(&mut self, milliseconds: u32) -> Result<()> {
        let mut cfg = self.cfg.clone();
        cfg.dispatch_budget_ms = milliseconds;
        cfg.validate(&self.renderer.ctx.device.limits())?;
        if milliseconds < self.cfg.dispatch_budget_ms {
            self.grid.halve();
        }
        self.cfg = cfg;
        self.spd = 1;
        self.want_halve = false;
        self.worst = 0.0;
        self.reset_seen();
        Ok(())
    }

    pub fn progress(&self) -> Progress {
        Progress {
            tiles: self.grid.tiles.len(),
            dispatches: self.dispatches,
            samples_min: self.grid.min_done(),
            samples_max: self.grid.max_done(),
            target: self.cfg.samples_per_pixel,
            elapsed: self.elapsed,
            max_dispatch_ms: self.max_dispatch.as_millis() as u32,
            tiles_done: self.grid.progress_cells(),
            tile_grid: (
                self.grid.width.div_ceil(self.grid.tile_size),
                self.grid.height.div_ceil(self.grid.tile_size),
            ),
        }
    }

    /// Runs dispatches until `budget` has elapsed or the render is complete.
    /// Always runs at least one dispatch when work remains.
    pub fn step(&mut self, budget: Duration) -> Result<bool> {
        if self.flame.is_some() || self.flame_work.is_some() {
            return self.step_flame(budget);
        }
        let slow_dispatch = Duration::from_millis(self.cfg.dispatch_budget_ms as u64);
        let fast_dispatch = slow_dispatch.mul_f64(0.2);
        let grow_dispatch = slow_dispatch.mul_f64(0.5);
        let start = Instant::now();
        let target = self.cfg.samples_per_pixel;
        loop {
            if self.cfg.guiding {
                if let Some(&boundary) =
                    crate::guiding::BOUNDARIES.get(self.guiding_updates as usize)
                {
                    if self.grid.min_done() == boundary {
                        self.freeze_guiding()?;
                    }
                }
            }
            if self.want_halve && self.grid.at_pass_boundary() {
                self.grid.halve();
                self.want_halve = false;
                self.worst = 0.0;
                self.reset_seen();
            }
            let Some(ti) = self.grid.next_pending(target) else {
                break;
            };
            let tile = self.grid.tiles[ti];
            let mut n = self.spd.min(target - tile.done);
            let mut training = false;
            if self.cfg.guiding {
                if let Some(&boundary) =
                    crate::guiding::BOUNDARIES.get(self.guiding_updates as usize)
                {
                    n = n.min(boundary - tile.done);
                    let first = crate::guiding::training_start(
                        self.cfg.width as u64 * self.cfg.height as u64,
                        boundary,
                    );
                    if tile.done < first {
                        n = n.min(first - tile.done);
                    } else {
                        training = true;
                    }
                }
            }
            self.uniforms
                .set_tile(tile.x, tile.y, tile.w, tile.h, tile.done, n);
            let ctx = &self.renderer.ctx;
            let dispatch_start = Instant::now();
            ctx.queue
                .write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&self.uniforms));
            let dt = self.renderer.with_scopes(|| {
                let pipeline = if training {
                    &self
                        .guiding_pipelines
                        .as_ref()
                        .expect("guided session variants")
                        .training
                } else if self.guiding_trusted_cells > 0 {
                    &self
                        .guiding_pipelines
                        .as_ref()
                        .expect("guided session variants")
                        .sampling
                } else {
                    &self.pipeline.pipeline
                };
                self.renderer.dispatch(
                    pipeline,
                    &self.bind_group,
                    tile.w,
                    tile.h,
                    self.pipeline.workgroup,
                )
            })?;
            ctx.check()?;
            let dispatch_wall = dispatch_start.elapsed();
            let stage = &mut self.guiding_profile.stages[self.guiding_updates as usize];
            stage.dispatches += 1;
            stage.pixel_samples += n as u64 * tile.w as u64 * tile.h as u64;
            stage.wall += dispatch_wall;
            let variant = &mut self.guiding_profile.variants[if training {
                2
            } else if self.guiding_trusted_cells > 0 {
                1
            } else {
                0
            }];
            variant.dispatches += 1;
            variant.pixel_samples += n as u64 * tile.w as u64 * tile.h as u64;
            variant.wall += dispatch_wall;
            self.grid.tiles[ti].done += n;
            // Commit a complete training round even when this dispatch uses
            // the whole time budget or finishes at an update boundary.
            if self.cfg.guiding {
                if let Some(&boundary) =
                    crate::guiding::BOUNDARIES.get(self.guiding_updates as usize)
                {
                    if self.grid.min_done() == boundary {
                        self.freeze_guiding()?;
                    }
                }
            }
            self.dispatches += 1;
            self.max_dispatch = self.max_dispatch.max(dt);
            let rate = dt.as_secs_f64() / n as f64;
            self.worst = self.worst.max(rate);
            self.tile_cost[ti] = self.tile_cost[ti].max(rate);
            self.seen[ti] = true;
            if dt > slow_dispatch {
                if self.spd > 1 {
                    self.spd = (self.spd / 2).max(1);
                    self.reset_seen();
                } else {
                    self.want_halve = true;
                }
            } else if self.seen.iter().all(|&s| s) {
                // Grow inexpensive regions independently after observing a
                // complete round. Spatial work exposes independent workgroups;
                // additional samples loop serially in the shader.
                let fast = fast_dispatch.as_secs_f64();
                let cap = self.cfg.samples_per_dispatch.max(1);
                if self.grid.merge_fast(
                    &self.tile_cost,
                    self.cfg.tile_size,
                    grow_dispatch.as_secs_f64() / self.spd as f64,
                ) {
                    self.worst = 0.0;
                    self.reset_seen();
                } else if self.spd < cap && self.worst * ((self.spd * 2) as f64) < fast {
                    self.spd = (self.spd * 2).min(cap);
                    self.reset_seen();
                } else if self.grid.at_pass_boundary() {
                    // A cold driver submission or an unusually costly sample
                    // must not lock the entire remaining render at startup
                    // settings. Re-observe every tile in the next complete
                    // pass before allowing another ramp attempt.
                    self.worst = 0.0;
                    self.reset_seen();
                }
            }
            if start.elapsed() >= budget {
                break;
            }
        }
        self.elapsed += start.elapsed();
        Ok(self.is_done())
    }

    pub fn readback(&self) -> Result<RenderOutput> {
        let ctx = &self.renderer.ctx;
        ctx.check()?;
        let pixels = (self.cfg.width as u64) * (self.cfg.height as u64);
        let (raw, raw_depth, raw_aux) = self.renderer.with_scopes(|| {
            Ok((
                ctx.read_buffer(&self.accum, pixels * 16)?,
                ctx.read_buffer(&self.depth, pixels * 4)?,
                ctx.read_buffer(&self.aux, pixels * 16)?,
            ))
        })?;
        let sums: Vec<[f32; 4]> = bytemuck::pod_collect_to_vec(&raw);
        let data = sums
            .iter()
            .map(|p| {
                let n = p[3].max(1.0);
                [p[0] / n, p[1] / n, p[2] / n, 1.0]
            })
            .collect();
        let layers = if self.scene.dimension == syfrack_core::Dimension::ThreeD
            && self.scene.atmosphere.fog_density > 0.0
        {
            let raw_layers = self.renderer.with_scopes(|| {
                Ok([
                    ctx.read_buffer(&self.volume, pixels * 16)?,
                    ctx.read_buffer(&self.volume_moments, pixels * 16)?,
                    ctx.read_buffer(&self.surface, pixels * 16)?,
                    ctx.read_buffer(
                        &self.volume_components,
                        if self.volume_components_enabled {
                            pixels * 48
                        } else {
                            48
                        },
                    )?,
                ])
            })?;
            let volume: Vec<[f32; 4]> = bytemuck::pod_collect_to_vec(&raw_layers[0]);
            let moments: Vec<[f32; 4]> = bytemuck::pod_collect_to_vec(&raw_layers[1]);
            let surface: Vec<[f32; 4]> = bytemuck::pod_collect_to_vec(&raw_layers[2]);
            let components: Vec<[[f32; 4]; 3]> = bytemuck::pod_collect_to_vec(&raw_layers[3]);
            let mean = |values: &[[f32; 4]]| {
                HdrImage::from_data(
                    self.cfg.width,
                    self.cfg.height,
                    values
                        .iter()
                        .map(|p| {
                            let n = p[3].max(1.0);
                            [p[0] / n, p[1] / n, p[2] / n, 1.0]
                        })
                        .collect(),
                )
            };
            let volume_variance = volume
                .iter()
                .zip(&moments)
                .map(|(v, m)| {
                    let n = v[3];
                    std::array::from_fn(|c| {
                        if n > 1.0 {
                            (m[c] / (n * (n - 1.0))).max(0.0)
                        } else {
                            v[c] * v[c]
                        }
                    })
                })
                .collect();
            let component_variance = |part: usize| {
                components
                    .iter()
                    .zip(&volume)
                    .map(|(p, v)| {
                        let n = p[0][3];
                        std::array::from_fn(|c| {
                            if n > 1.0 {
                                (p[part][c] / (n * (n - 1.0))).max(0.0)
                            } else if part == 1 {
                                p[0][c] * p[0][c]
                            } else {
                                (v[c] - p[0][c]).powi(2)
                            }
                        })
                    })
                    .collect()
            };
            let components = self.volume_components_enabled.then(|| {
                let direct: Vec<_> = components.iter().map(|p| p[0]).collect();
                crate::renderer::VolumeRadianceComponents {
                    direct: mean(&direct),
                    direct_variance: component_variance(1),
                    indirect_variance: component_variance(2),
                }
            });
            Some(crate::renderer::RadianceLayers {
                surface: mean(&surface),
                volume: mean(&volume),
                volume_variance,
                components,
            })
        } else {
            None
        };
        Ok(RenderOutput {
            color: HdrImage::from_data(self.cfg.width, self.cfg.height, data),
            layers,
            depth: bytemuck::pod_collect_to_vec(&raw_depth),
            normals: bytemuck::pod_collect_to_vec(&raw_aux),
            samples_min: self.grid.min_done(),
            samples_max: self.grid.max_done(),
            elapsed: self.elapsed,
        })
    }

    /// Albedo and shading-normal guides for opaque HDR export. Temporary GPU
    /// buffers protect the real samples and geometric guides from modification.
    pub fn export_guides(&self) -> Result<(HdrImage, HdrImage)> {
        self.export_layer_guides(false)
    }

    /// Material features for the separate surface layer, without fog blending.
    pub fn export_surface_guides(&self) -> Result<(HdrImage, HdrImage)> {
        self.export_layer_guides(true)
    }

    fn export_layer_guides(&self, surface_only: bool) -> Result<(HdrImage, HdrImage)> {
        anyhow::ensure!(
            self.grid.min_done() > 0,
            "all pixels need a first sample for export guides"
        );
        let mut cached = self
            .export_guides_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let slot = usize::from(surface_only);
        if let Some(guides) = cached[slot].as_ref() {
            return Ok(guides.clone());
        }
        let ctx = &self.renderer.ctx;
        let bytes = u64::from(self.cfg.width) * u64::from(self.cfg.height) * 16;
        let guides = self.renderer.with_scopes(|| {
            let buffer = |label| {
                ctx.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: bytes,
                    usage: wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::COPY_SRC
                        | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            };
            let albedo = buffer("export albedo");
            let normals = buffer("export shading normals");
            let mut encoder = ctx.device.create_command_encoder(&Default::default());
            encoder.copy_buffer_to_buffer(&self.aux, 0, &normals, 0, bytes);
            ctx.queue.submit([encoder.finish()]);
            let entries = [
                (0, self.uniform_buf.as_entire_binding()),
                (1, albedo.as_entire_binding()),
                (2, self.depth.as_entire_binding()),
                (3, normals.as_entire_binding()),
                (12, self.volume.as_entire_binding()),
                (13, self.volume_moments.as_entire_binding()),
                (14, self.surface.as_entire_binding()),
                (15, self.volume_components.as_entire_binding()),
                (4, self.params_buf.as_entire_binding()),
                (5, self.texture_params_buf.as_entire_binding()),
                (6, self.environment.buffer.as_entire_binding()),
                (7, wgpu::BindingResource::TextureView(&self.particles.view)),
                (8, wgpu::BindingResource::Sampler(&self.particles.sampler)),
                (
                    9,
                    wgpu::BindingResource::TextureView(&self.particles.lighting),
                ),
                (
                    10,
                    wgpu::BindingResource::TextureView(&self.particles.microstructure.view),
                ),
                (
                    11,
                    wgpu::BindingResource::Sampler(&self.particles.microstructure.sampler),
                ),
            ]
            .into_iter()
            .map(|(binding, resource)| wgpu::BindGroupEntry { binding, resource })
            .collect::<Vec<_>>();
            let group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("material export guides"),
                layout: &self.renderer.layouts.bind_group_layout,
                entries: &entries,
            });
            let pipeline = self
                .pipeline
                .export_pipeline(&ctx.device, &self.renderer.layouts);
            let mut uniforms = self.uniforms;
            if surface_only {
                uniforms.flags |= 2;
            }
            let dispatched = (|| -> Result<()> {
                for tile in TileGrid::new(self.cfg.width, self.cfg.height, 64).tiles {
                    uniforms.set_tile(tile.x, tile.y, tile.w, tile.h, 0, 1);
                    ctx.queue
                        .write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&uniforms));
                    self.renderer
                        .dispatch(&pipeline, &group, tile.w, tile.h, [8, 8])?;
                }
                Ok(())
            })();
            ctx.queue
                .write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&self.uniforms));
            dispatched?;
            let a = bytemuck::pod_collect_to_vec(&ctx.read_buffer(&albedo, bytes)?);
            let n = bytemuck::pod_collect_to_vec(&ctx.read_buffer(&normals, bytes)?);
            Ok((
                HdrImage::from_data(self.cfg.width, self.cfg.height, a),
                HdrImage::from_data(self.cfg.width, self.cfg.height, n),
            ))
        })?;
        cached[slot] = Some(guides.clone());
        Ok(guides)
    }

    fn guiding_offset(&self) -> u64 {
        self.cfg.width as u64 * self.cfg.height as u64 * 4
    }

    fn update_guiding_uniforms(&mut self) {
        self.uniforms.guiding_flags = crate::guiding::flags(self.cfg.guiding, self.guiding_updates);
        if self.guiding_trusted_cells == 0 {
            self.uniforms.guiding_flags &= !2;
        }
        self.uniforms.guiding_record_round = self.guiding_updates;
        self.uniforms.guiding_training_end = crate::guiding::BOUNDARIES
            .get(self.guiding_updates as usize)
            .copied()
            .unwrap_or(0);
    }

    fn freeze_guiding(&mut self) -> Result<()> {
        let ctx = &self.renderer.ctx;
        let bank_bytes = (crate::guiding::BANK_WORDS * 4) as u64;
        let populated_bytes = ((self.guiding_updates + 1) as usize
            * crate::guiding::RECORDS
            * crate::guiding::RECORD_ROW
            * 4) as u64;
        let t = Instant::now();
        let bytes = self.renderer.with_scopes(|| {
            ctx.read_buffer_range(&self.depth, self.guiding_offset(), populated_bytes)
        })?;
        let readback = t.elapsed();
        let t = Instant::now();
        let records: Vec<u32> = bytemuck::pod_collect_to_vec(&bytes);
        let decode = t.elapsed();
        let t = Instant::now();
        let model = crate::guiding::build_model(&records);
        let fit = t.elapsed();
        let t = Instant::now();
        ctx.queue.write_buffer(
            &self.depth,
            self.guiding_offset() + bank_bytes,
            bytemuck::cast_slice(&model.words),
        );
        ctx.check()?;
        self.guiding_profile.updates[self.guiding_updates as usize] =
            crate::guiding::UpdateProfile {
                readback,
                decode,
                fit,
                upload: t.elapsed(),
                readback_bytes: populated_bytes,
                upload_bytes: (model.words.len() * 4) as u64,
            };
        self.guiding_updates += 1;
        self.guiding_active_cells = model.active;
        self.guiding_spatial_leaves = model.leaves;
        self.guiding_training_records = model.records;
        self.guiding_trusted_cells = model.trusted;
        self.update_guiding_uniforms();
        Ok(())
    }

    pub fn guiding_stats(&self) -> crate::guiding::GuidingStats {
        crate::guiding::GuidingStats {
            updates: self.guiding_updates,
            active_cells: self.guiding_active_cells,
            spatial_leaves: self.guiding_spatial_leaves,
            training_records: self.guiding_training_records,
            trusted_cells: self.guiding_trusted_cells,
            memory_bytes: if self.cfg.guiding {
                crate::guiding::BYTES
            } else {
                0
            },
        }
    }

    pub fn guiding_profile(&self) -> crate::guiding::GuidingProfile {
        self.guiding_profile
    }

    /// Writes the checkpoint into a sibling temp directory, then swaps it in.
    /// Windows cannot rename onto an existing directory, so the old checkpoint
    /// is renamed aside first and deleted only once the new one is in place:
    /// one complete checkpoint always exists. A directory holding anything
    /// other than state files is never replaced.
    pub fn save(&self, dir: &Path) -> Result<()> {
        check_replaceable(dir)?;
        let ctx = &self.renderer.ctx;
        ctx.check()?;
        let pixels = (self.cfg.width as u64) * (self.cfg.height as u64);
        let mut tmp_name = dir
            .file_name()
            .ok_or_else(|| anyhow!("checkpoint path {} has no final component", dir.display()))?
            .to_os_string();
        tmp_name.push(format!(".tmp-{}", std::process::id()));
        let tmp = dir.with_file_name(tmp_name);
        match std::fs::remove_dir_all(&tmp) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("clearing {}", tmp.display())),
        }
        std::fs::create_dir_all(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        let written = (|| -> Result<()> {
            let buffers = self.renderer.with_scopes(|| {
                Ok([
                    ctx.read_buffer(&self.accum, pixels * 16)?,
                    ctx.read_buffer(&self.depth, pixels * 4)?,
                    ctx.read_buffer(&self.aux, pixels * 16)?,
                    ctx.read_buffer(&self.volume, pixels * 16)?,
                    ctx.read_buffer(&self.volume_moments, pixels * 16)?,
                    ctx.read_buffer(&self.surface, pixels * 16)?,
                    ctx.read_buffer(
                        &self.volume_components,
                        if self.volume_components_enabled {
                            pixels * 48
                        } else {
                            48
                        },
                    )?,
                ])
            })?;
            for (name, bytes) in [
                "accum.bin",
                "depth.bin",
                "normals.bin",
                "volume.bin",
                "volume-moments.bin",
                "surface.bin",
                "volume-components.bin",
            ]
            .into_iter()
            .zip(buffers)
            {
                std::fs::write(tmp.join(name), bytes).with_context(|| format!("writing {name}"))?;
            }
            let guiding_fingerprint = if self.cfg.guiding {
                let bytes = self.renderer.with_scopes(|| {
                    ctx.read_buffer_range(&self.depth, self.guiding_offset(), crate::guiding::BYTES)
                })?;
                let hash = crate::guiding::fingerprint(&bytes);
                std::fs::write(tmp.join("guiding.bin"), bytes).context("writing guiding.bin")?;
                Some(hash)
            } else {
                None
            };
            let state = SessionState {
                shader_fingerprint: Some(self.pipeline.shader_fingerprint),
                environment_fingerprint: self.environment.fingerprint,
                particle_fingerprint: self.particles.fingerprint,
                guiding_updates: self.guiding_updates,
                guiding_fingerprint,
                guiding_version: if self.cfg.guiding {
                    crate::guiding::VERSION
                } else {
                    0
                },
                scene: self.scene.clone(),
                cfg: self.cfg.clone(),
                tiles: self.grid.tiles.clone(),
                tile_size: self.grid.tile_size,
                spd: self.spd,
                dispatches: self.dispatches,
                elapsed_secs: self.elapsed.as_secs_f64(),
            };
            let text =
                ron::ser::to_string_pretty(&state, ron::ser::PrettyConfig::new().depth_limit(6))?;
            std::fs::write(tmp.join(STATE_FILE), text).context("writing state.ron")?;
            Ok(())
        })();
        if let Err(e) = written {
            std::fs::remove_dir_all(&tmp).ok();
            return Err(e);
        }
        let mut aside_name = dir.file_name().expect("checked above").to_os_string();
        aside_name.push(format!(".old-{}", std::process::id()));
        let aside = dir.with_file_name(aside_name);
        match std::fs::remove_dir_all(&aside) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                std::fs::remove_dir_all(&tmp).ok();
                return Err(e).with_context(|| format!("clearing {}", aside.display()));
            }
        }
        let had_old = dir.exists();
        if had_old {
            if let Err(e) = std::fs::rename(dir, &aside) {
                std::fs::remove_dir_all(&tmp).ok();
                return Err(e).with_context(|| format!("moving {} aside", dir.display()));
            }
        }
        if let Err(e) = std::fs::rename(&tmp, dir) {
            if had_old {
                std::fs::rename(&aside, dir).ok();
            }
            std::fs::remove_dir_all(&tmp).ok();
            return Err(e).with_context(|| format!("renaming into {}", dir.display()));
        }
        if had_old {
            std::fs::remove_dir_all(&aside).ok();
        }
        Ok(())
    }

    pub fn resume(renderer: Renderer, dir: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(dir.join(STATE_FILE))
            .with_context(|| format!("reading {}", dir.join(STATE_FILE).display()))?;
        let state: SessionState = ron::from_str(&text).map_err(|e| anyhow!("state.ron: {e}"))?;
        // Metadata is only a preview cache hint; resume hashes fresh pixels.
        renderer.invalidate_environment();
        let mut session = Self::new(renderer, &state.scene, &state.cfg)?;
        if state.tiles.iter().any(|tile| tile.done > 0)
            && state.shader_fingerprint != Some(session.pipeline.shader_fingerprint)
        {
            anyhow::bail!(
                "this checkpoint was rendered with a different or legacy lighting engine; open its scene and start a new render (the saved files have not been changed)"
            );
        }
        if state.tiles.iter().any(|tile| tile.done > 0)
            && state.environment_fingerprint != session.environment.fingerprint
        {
            anyhow::bail!("HDRI contents have changed since this checkpoint; start a new render (the saved files have not been changed)");
        }
        let pixels = (state.cfg.width as usize) * (state.cfg.height as usize);
        if state.tiles.iter().any(|tile| tile.done > 0)
            && state.particle_fingerprint != session.particles.fingerprint
        {
            anyhow::bail!("particle simulation or volume lighting changed since this checkpoint; start a new render (the saved files have not been changed)");
        }
        let ctx = &session.renderer.ctx;
        for (name, buf, size) in [
            ("accum.bin", &session.accum, pixels * 16),
            ("depth.bin", &session.depth, pixels * 4),
            ("normals.bin", &session.aux, pixels * 16),
            ("volume.bin", &session.volume, pixels * 16),
            ("volume-moments.bin", &session.volume_moments, pixels * 16),
            ("surface.bin", &session.surface, pixels * 16),
            (
                "volume-components.bin",
                &session.volume_components,
                if session.volume_components_enabled {
                    pixels * 48
                } else {
                    48
                },
            ),
        ] {
            let bytes = std::fs::read(dir.join(name)).with_context(|| format!("reading {name}"))?;
            if bytes.len() != size {
                anyhow::bail!("{name}: expected {size} bytes, found {}", bytes.len());
            }
            ctx.queue.write_buffer(buf, 0, &bytes);
        }
        session.grid = TileGrid::restore(
            state.cfg.width,
            state.cfg.height,
            state.tile_size,
            state.tiles,
            state.cfg.samples_per_pixel,
        )
        .map_err(|e| anyhow!("state.ron: {e}"))?;
        if state.cfg.guiding {
            if state.guiding_version != crate::guiding::VERSION {
                anyhow::bail!("state.ron: incompatible guiding algorithm version; saved files have not been changed");
            }
            let updates = state.guiding_updates as usize;
            if updates > crate::guiding::BOUNDARIES.len() {
                anyhow::bail!("state.ron: invalid guiding update count");
            }
            let previous = updates
                .checked_sub(1)
                .map_or(0, |i| crate::guiding::BOUNDARIES[i]);
            let next = crate::guiding::BOUNDARIES
                .get(updates)
                .copied()
                .unwrap_or(u32::MAX);
            if session.grid.min_done() < previous || session.grid.max_done() > next {
                anyhow::bail!("state.ron: guiding stage does not match tile progress");
            }
            let bytes = std::fs::read(dir.join("guiding.bin")).context("reading guiding.bin")?;
            if bytes.len() != crate::guiding::BYTES as usize
                || Some(crate::guiding::fingerprint(&bytes)) != state.guiding_fingerprint
            {
                anyhow::bail!(
                    "guiding.bin is incomplete or corrupted; saved files have not been changed"
                );
            }
            ctx.queue
                .write_buffer(&session.depth, session.guiding_offset(), &bytes);
            session.guiding_updates = state.guiding_updates;
            let words: Vec<u32> = bytemuck::pod_collect_to_vec(&bytes);
            session.guiding_spatial_leaves = words[crate::guiding::BANK_WORDS + 7] as usize;
            session.guiding_training_records = words[crate::guiding::BANK_WORDS + 8] as usize;
            session.guiding_active_cells = words[crate::guiding::PROPOSAL_OFFSET..]
                .chunks_exact(crate::guiding::PROPOSAL_ROW)
                .filter(|row| row[1] != 0)
                .count();
            session.guiding_trusted_cells = words[crate::guiding::PROPOSAL_OFFSET..]
                .chunks_exact(crate::guiding::PROPOSAL_ROW)
                .filter(|row| row[0] != 0)
                .count();
        }
        if let Some(flame) = &mut session.flame {
            if session.grid.min_done() != session.grid.max_done() {
                anyhow::bail!("flame checkpoint has inconsistent pass counts");
            }
            // Rebuild immutable orbit streams; no extra checkpoint format or lossy histogram.
            flame.advance(session.grid.min_done());
        }
        // The ramp restarts: chunking does not change the image.
        session.spd = 1;
        session.dispatches = state.dispatches;
        session.elapsed = Duration::try_from_secs_f64(state.elapsed_secs)
            .map_err(|e| anyhow!("state.ron: elapsed_secs: {e}"))?;
        ctx.check()?;
        session.reset_seen();
        session.update_guiding_uniforms();
        Ok(session)
    }

    /// Topology fingerprint of the graph being rendered.
    pub fn fingerprint(&self) -> u64 {
        self.pipeline.fingerprint
    }

    pub fn resource_fingerprint(&self) -> u64 {
        let key = crate::particles::resource_key(
            &self.scene,
            self.environment
                .resource_fingerprint(self.pipeline.fingerprint),
        );
        if self.scene.canvas.flame.is_some() {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            key.hash(&mut h);
            self.scene.seed.hash(&mut h);
            ron::ser::to_string(&self.scene.canvas)
                .unwrap()
                .hash(&mut h);
            include_str!("flame.rs").hash(&mut h);
            h.finish()
        } else {
            key
        }
    }

    /// Swaps the scene and restarts from zero, like `reset`. Returns true when
    /// the graph keeps its topology (only the parameter buffer is rewritten),
    /// false when the pipeline was recompiled (or fetched from the cache).
    pub fn set_scene_params(&mut self, scene: &Scene) -> Result<bool> {
        scene.validate().context("scene validation")?;
        let same = crate::graph_wgsl::scene_fingerprint(scene) == self.pipeline.fingerprint;
        self.reset(scene)?;
        Ok(same)
    }

    /// Adopts an already compiled pipeline (from `Renderer::request`) for
    /// `scene`, whose graph must have the pipeline's topology, and restarts
    /// from zero.
    pub fn swap_pipeline(&mut self, pipeline: Arc<CompiledPipeline>, scene: &Scene) -> Result<()> {
        scene.validate().context("scene validation")?;
        if crate::graph_wgsl::scene_fingerprint(scene) != pipeline.fingerprint {
            anyhow::bail!("the pipeline does not match the scene's graph topology");
        }
        let previous = std::mem::replace(&mut self.pipeline, pipeline);
        if let Err(error) = self.reset(scene) {
            self.pipeline = previous;
            return Err(error);
        }
        Ok(())
    }

    /// Points the session at `scene`'s graph: switches pipeline when the
    /// topology changed, then rewrites the parameter buffer.
    fn load_graph(&mut self, scene: &Scene) -> Result<()> {
        let g = scene.fractal.to_graph();
        if crate::graph_wgsl::scene_fingerprint(scene) != self.pipeline.fingerprint {
            self.pipeline = self.renderer.pipeline_for_scene(scene)?;
        }
        self.guiding_pipelines = if self.cfg.guiding {
            Some(
                self.pipeline
                    .guiding_pipelines(&self.renderer.ctx.device, &self.renderer.layouts)?,
            )
        } else {
            None
        };
        self.renderer.ctx.queue.write_buffer(
            &self.params_buf,
            0,
            bytemuck::cast_slice(&self.pipeline.layout.pack(&g)),
        );
        if let (Some(layout), Some(texture)) =
            (&self.pipeline.texture_layout, &scene.material.texture)
        {
            self.renderer.ctx.queue.write_buffer(
                &self.texture_params_buf,
                0,
                bytemuck::cast_slice(&layout.pack(&texture.graph)),
            );
        }
        Ok(())
    }

    /// Keeps the image size and config, swaps the scene, and restarts from zero.
    pub fn reset(&mut self, scene: &Scene) -> Result<()> {
        scene.validate().context("scene validation")?;
        let mut scene = scene.clone();
        scene.resolve_asset_paths(Path::new("."))?;
        let scene = &scene;
        let environment = self.renderer.environment_for(scene)?;
        let particles = self.renderer.particles_for(scene)?;
        let bind_group = if !Arc::ptr_eq(&self.environment, &environment)
            || !Arc::ptr_eq(&self.particles, &particles)
        {
            Some(self.renderer.with_scopes(|| {
                Ok(self
                    .renderer
                    .ctx
                    .device
                    .create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("pathtrace"),
                        layout: &self.renderer.layouts.bind_group_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: self.uniform_buf.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: self.accum.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: self.depth.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: self.aux.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 12,
                                resource: self.volume.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 13,
                                resource: self.volume_moments.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 14,
                                resource: self.surface.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 15,
                                resource: self.volume_components.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 4,
                                resource: self.params_buf.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 5,
                                resource: self.texture_params_buf.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 6,
                                resource: environment.buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 7,
                                resource: wgpu::BindingResource::TextureView(&particles.view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 8,
                                resource: wgpu::BindingResource::Sampler(&particles.sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 9,
                                resource: wgpu::BindingResource::TextureView(&particles.lighting),
                            },
                            wgpu::BindGroupEntry {
                                binding: 10,
                                resource: wgpu::BindingResource::TextureView(
                                    &particles.microstructure.view,
                                ),
                            },
                            wgpu::BindGroupEntry {
                                binding: 11,
                                resource: wgpu::BindingResource::Sampler(
                                    &particles.microstructure.sampler,
                                ),
                            },
                        ],
                    }))
            })?)
        } else {
            None
        };
        self.load_graph(scene)?;
        if let Some(bind_group) = bind_group {
            self.bind_group = bind_group;
        }
        self.environment = environment;
        self.particles = particles;
        self.scene = scene.clone();
        self.flame_work = None;
        self.flame = if scene.canvas.flame.is_some() {
            Some(crate::flame::FlameAccumulator::new(
                &scene.canvas,
                scene.seed,
                self.cfg.width,
                self.cfg.height,
            )?)
        } else {
            None
        };
        self.uniforms = Uniforms::from_scene(
            scene,
            self.cfg.width,
            self.cfg.height,
            self.cfg.max_bounces,
            self.cfg.max_steps,
        );
        self.environment.apply(&mut self.uniforms);
        self.uniforms.fractal.p0[0] = self.cfg.pixel_tolerance;
        if self.volume_components_enabled {
            self.uniforms.flags |= 4;
        }
        self.guiding_updates = 0;
        self.guiding_active_cells = 0;
        self.guiding_spatial_leaves = 0;
        self.guiding_training_records = 0;
        self.guiding_trusted_cells = 0;
        self.guiding_profile = Default::default();
        self.update_guiding_uniforms();
        self.grid = TileGrid::new(
            self.cfg.width,
            self.cfg.height,
            initial_tile(scene, self.cfg.tile_size),
        );
        self.reset_seen();
        self.spd = 1;
        self.worst = 0.0;
        self.max_dispatch = Duration::ZERO;
        self.want_halve = false;
        self.dispatches = 0;
        self.elapsed = Duration::ZERO;
        *self
            .export_guides_cache
            .get_mut()
            .unwrap_or_else(|p| p.into_inner()) = [None, None];
        self.clear_buffers()
    }
}
