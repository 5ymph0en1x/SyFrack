use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use glam::Vec3;
use syfrack_core::{DeSample, Graph, HdrImage, Scene};

use crate::gpu::GpuContext;
use crate::graph_wgsl::{
    compile, compile_texture, fingerprint, scene_fingerprint, Compiled, PARAM_COUNT,
};
use crate::pipeline_cache::{CompileStatus, CompileTicket, CompiledPipeline, PipelineCache};
use crate::session::RenderSession;
use crate::uniforms::Uniforms;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RenderConfig {
    pub width: u32,
    pub height: u32,
    pub samples_per_pixel: u32,
    pub max_bounces: u32,
    pub max_steps: u32,
    /// Primary hit tolerance in pixels. Smaller values retain finer geometry.
    #[serde(default = "default_pixel_tolerance")]
    pub pixel_tolerance: f32,
    pub tile_size: u32,
    /// Samples per pixel per dispatch; lowered automatically if a dispatch is slow.
    pub samples_per_dispatch: u32,
    /// Target upper dispatch latency. Export jobs can trade UI latency for
    /// larger spatial batches; this never changes the pixel sample budget.
    #[serde(default = "default_dispatch_budget_ms")]
    pub dispatch_budget_ms: u32,
    pub time_budget: Option<Duration>,
    /// Filter the developed export; keep the EXR and accumulation raw.
    #[serde(default)]
    pub denoise: bool,
    /// Learn a bounded directional GI proposal; useful for longer still renders.
    #[serde(default)]
    pub guiding: bool,
}

fn default_pixel_tolerance() -> f32 {
    1.0
}
fn default_dispatch_budget_ms() -> u32 {
    400
}

impl RenderConfig {
    pub fn new(width: u32, height: u32, samples_per_pixel: u32) -> Self {
        Self {
            width,
            height,
            samples_per_pixel,
            max_bounces: 4,
            max_steps: 256,
            pixel_tolerance: default_pixel_tolerance(),
            tile_size: 256,
            samples_per_dispatch: 8,
            dispatch_budget_ms: default_dispatch_budget_ms(),
            time_budget: None,
            denoise: false,
            guiding: false,
        }
    }
}

impl RenderConfig {
    /// Rejects configurations that would crash, hang or exceed device limits.
    pub fn validate(&self, limits: &wgpu::Limits) -> Result<()> {
        if self.width < 1 {
            anyhow::bail!("width must be at least 1, got {}", self.width);
        }
        if self.height < 1 {
            anyhow::bail!("height must be at least 1, got {}", self.height);
        }
        if self.samples_per_pixel < 1 {
            anyhow::bail!(
                "samples_per_pixel must be at least 1, got {}",
                self.samples_per_pixel
            );
        }
        if self.max_bounces > 32 {
            anyhow::bail!("max_bounces must be at most 32, got {}", self.max_bounces);
        }
        if self.max_steps < 1 {
            anyhow::bail!("max_steps must be at least 1, got {}", self.max_steps);
        }
        if !(50..=800).contains(&self.dispatch_budget_ms) {
            anyhow::bail!(
                "dispatch_budget_ms must be between 50 and 800, got {}",
                self.dispatch_budget_ms
            );
        }
        if !self.pixel_tolerance.is_finite() || !(0.125..=1.0).contains(&self.pixel_tolerance) {
            anyhow::bail!(
                "pixel_tolerance must be between 0.125 and 1.0, got {}",
                self.pixel_tolerance
            );
        }
        let pixels = (self.width as u64) * (self.height as u64);
        let accum_bytes = (pixels * 16).max(
            pixels * 4
                + if self.guiding {
                    crate::guiding::BYTES
                } else {
                    0
                },
        );
        let storage_limit = limits.max_storage_buffer_binding_size;
        if accum_bytes > storage_limit {
            anyhow::bail!(
                "width x height ({}x{}) needs {accum_bytes} accumulation bytes, above the device max_storage_buffer_binding_size of {storage_limit}",
                self.width,
                self.height
            );
        }
        if accum_bytes > limits.max_buffer_size {
            anyhow::bail!(
                "width x height ({}x{}) needs a {accum_bytes}-byte buffer, above the device max_buffer_size of {}",
                self.width,
                self.height,
                limits.max_buffer_size
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct RenderOutput {
    /// Mean radiance per pixel, linear. Alpha is 1.
    pub color: HdrImage,
    /// Additive surface and participating-medium radiance, before reconstruction.
    pub layers: Option<RadianceLayers>,
    /// First-hit distance per pixel, -1 for a miss.
    pub depth: Vec<f32>,
    /// First-hit auxiliary data per pixel: `[nx, ny, nz, steps]` with a unit
    /// normal and the march step count, or `[0, 0, 0, -1]` for a miss.
    pub normals: Vec<[f32; 4]>,
    pub samples_min: u32,
    pub samples_max: u32,
    pub elapsed: Duration,
}

#[derive(Clone, Debug)]
pub struct RadianceLayers {
    pub surface: HdrImage,
    pub volume: HdrImage,
    /// Per-channel dispersion of the volume mean. A conservative reconstruction
    /// guide for randomized quasi-Monte Carlo, not a confidence interval.
    pub volume_variance: Vec<[f32; 3]>,
    /// Primary segment integral, kept sharp independently of noisy continuations.
    /// Optional when the packed 48-byte component buffer fits device limits.
    /// Larger images can still render and reconstruct the total volume layer.
    pub components: Option<VolumeRadianceComponents>,
}

#[derive(Clone, Debug)]
pub struct VolumeRadianceComponents {
    pub direct: HdrImage,
    pub direct_variance: Vec<[f32; 3]>,
    pub indirect_variance: Vec<[f32; 3]>,
}

#[derive(Clone, Debug)]
pub struct Progress {
    pub tiles: usize,
    pub dispatches: u32,
    pub samples_min: u32,
    pub samples_max: u32,
    pub target: u32,
    pub elapsed: Duration,
    /// The slowest single dispatch so far in this session, in milliseconds.
    pub max_dispatch_ms: u32,
    /// Samples per pixel accumulated in each tile, row-major over `tile_grid`.
    pub tiles_done: Vec<u32>,
    /// Tile columns and rows.
    pub tile_grid: (u32, u32),
}

/// Longest a single dispatch may take before the device is presumed lost.
const DISPATCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Bind group and pipeline layouts shared by every compiled graph.
pub struct Layouts {
    /// Path tracer: 0 uniforms, 1-3 accumulation/depth/aux, 4 form,
    /// 5 texture parameters, 6 panorama radiance/CDF, 7 density texture,
    /// 8 volume sampler, 9 cached volume lighting.
    pub bind_group_layout: wgpu::BindGroupLayout,
    /// Probe: 0 uniforms, 1 points, 2 results, 3 traps, 4 graph parameters.
    pub probe_layout: wgpu::BindGroupLayout,
    pub pathtrace: wgpu::PipelineLayout,
    pub probe: wgpu::PipelineLayout,
}

/// Size of the graph parameter uniform buffer (binding 4).
pub const GRAPH_PARAMS_BYTES: u64 = (PARAM_COUNT * 4) as u64;

#[derive(Clone)]
pub struct Renderer {
    pub(crate) ctx: Arc<GpuContext>,
    pub(crate) layouts: Arc<Layouts>,
    cache: Arc<PipelineCache>,
    pub(crate) environment_cache: Arc<crate::environment::EnvironmentCache>,
    pub(crate) particle_cache: Arc<crate::particles::ParticleCache>,
}

impl Renderer {
    pub fn context(&self) -> &Arc<GpuContext> {
        &self.ctx
    }

    pub fn layouts(&self) -> &Arc<Layouts> {
        &self.layouts
    }

    /// Creates the layouts only; shaders are compiled per graph topology by
    /// `pipeline_for`.
    pub fn new(ctx: Arc<GpuContext>) -> Result<Self> {
        let device = &ctx.device;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let buffer_entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let rw = wgpu::BufferBindingType::Storage { read_only: false };
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pathtrace"),
            entries: &[
                buffer_entry(0, wgpu::BufferBindingType::Uniform),
                buffer_entry(1, rw),
                buffer_entry(2, rw),
                buffer_entry(3, rw),
                buffer_entry(4, wgpu::BufferBindingType::Uniform),
                buffer_entry(5, wgpu::BufferBindingType::Uniform),
                buffer_entry(6, wgpu::BufferBindingType::Storage { read_only: true }),
                buffer_entry(12, rw),
                buffer_entry(13, rw),
                buffer_entry(14, rw),
                buffer_entry(15, rw),
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pathtrace = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pathtrace"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            ..Default::default()
        });
        let probe_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("de_probe"),
            entries: &[
                buffer_entry(0, wgpu::BufferBindingType::Uniform),
                buffer_entry(1, wgpu::BufferBindingType::Storage { read_only: true }),
                buffer_entry(2, rw),
                buffer_entry(3, rw),
                buffer_entry(4, wgpu::BufferBindingType::Uniform),
            ],
        });
        let probe = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("de_probe"),
            bind_group_layouts: &[Some(&probe_layout)],
            ..Default::default()
        });
        if let Some(err) = pollster::block_on(scope.pop()) {
            return Err(anyhow!("layout validation failed: {err}"));
        }
        Ok(Self {
            ctx,
            layouts: Arc::new(Layouts {
                bind_group_layout,
                probe_layout,
                pathtrace,
                probe,
            }),
            cache: Arc::new(PipelineCache::default()),
            environment_cache: Arc::new(Default::default()),
            particle_cache: Arc::new(Default::default()),
        })
    }

    /// The pipelines of `g`'s topology: from the cache, or compiled now
    /// (synchronously) and cached. Scalar values never change the result.
    /// The graph is validated first, cache hit or not: an invalid graph
    /// sharing a cached topology (e.g. a zero scale) is refused.
    pub fn pipeline_for(&self, g: &Graph) -> Result<Arc<CompiledPipeline>> {
        g.validate().map_err(anyhow::Error::from)?;
        let fp = fingerprint(g);
        if let Some(p) = self.cache.get(fp) {
            return Ok(p);
        }
        let c = compile(g).map_err(anyhow::Error::from)?;
        self.build_and_cache(c)
    }

    /// Compiles the form and its optional independent material graph together.
    pub fn pipeline_for_scene(&self, scene: &Scene) -> Result<Arc<CompiledPipeline>> {
        scene.validate().context("scene validation")?;
        if scene.material.texture.is_none()
            && scene.atmosphere.nebula.is_none()
            && !crate::graph_wgsl::scene_has_optics(scene)
        {
            return self.pipeline_for(scene.fractal.graph());
        }
        let fp = scene_fingerprint(scene);
        if let Some(p) = self.cache.get(fp) {
            return Ok(p);
        }
        let form = compile(scene.fractal.graph())?;
        let texture = scene
            .material
            .texture
            .as_ref()
            .map(|t| compile_texture(&t.graph))
            .transpose()?;
        let p = Arc::new(CompiledPipeline::build_with_features(
            &self.ctx.device,
            &self.layouts,
            form,
            texture,
            scene.atmosphere.nebula.is_some(),
            crate::graph_wgsl::scene_has_optics(scene),
        )?);
        self.cache.insert(p.clone());
        Ok(p)
    }

    /// Background compilation of both graphs for the studio preview.
    pub fn request_scene(&self, scene: &Scene) -> CompileTicket {
        let fp = crate::environment::scene_resource_fingerprint(scene);
        let (tx, rx) = std::sync::mpsc::channel();
        if scene.validate().is_ok() && self.environment_ready(scene) {
            if let Some(p) = self.cache.get(scene_fingerprint(scene)) {
                let _ = tx.send(Ok(p));
                return CompileTicket {
                    fingerprint: fp,
                    rx,
                    done: Default::default(),
                };
            }
        }
        let renderer = self.clone();
        let scene = scene.clone();
        std::thread::Builder::new()
            .name("syfrack-texture-compile".into())
            .spawn(move || {
                let result = renderer
                    .environment_for(&scene)
                    .and_then(|_| renderer.particles_for(&scene))
                    .and_then(|_| renderer.pipeline_for_scene(&scene));
                let _ = tx.send(result);
            })
            .expect("spawn compile thread");
        CompileTicket {
            fingerprint: fp,
            rx,
            done: Default::default(),
        }
    }

    /// Builds the pipelines of an already generated shader and caches them
    /// on success only: a failed build leaves the cache untouched. Public
    /// so tests can inject a broken shader.
    #[doc(hidden)]
    pub fn build_and_cache(&self, c: Compiled) -> Result<Arc<CompiledPipeline>> {
        build_and_cache(&self.ctx.device, &self.layouts, &self.cache, c)
    }

    /// Starts compiling `g`'s pipelines on a background thread. A cached
    /// topology is ready immediately; the result is cached on success.
    pub fn request(&self, g: &Graph) -> CompileTicket {
        let fp = fingerprint(g);
        let (tx, rx) = std::sync::mpsc::channel();
        if g.validate().is_ok() {
            if let Some(p) = self.cache.get(fp) {
                let _ = tx.send(Ok(p));
                return CompileTicket {
                    fingerprint: fp,
                    rx,
                    done: Default::default(),
                };
            }
        }
        let (device, layouts, cache, g) = (
            self.ctx.device.clone(),
            self.layouts.clone(),
            self.cache.clone(),
            g.clone(),
        );
        std::thread::Builder::new()
            .name("syfrack-compile".into())
            .spawn(move || {
                let result = g
                    .validate()
                    .map_err(anyhow::Error::from)
                    .and_then(|()| compile(&g).map_err(anyhow::Error::from))
                    .and_then(|c| build_and_cache(&device, &layouts, &cache, c));
                let _ = tx.send(result);
            })
            .expect("spawn compile thread");
        CompileTicket {
            fingerprint: fp,
            rx,
            done: Default::default(),
        }
    }

    /// Non-blocking check of a requested compilation.
    pub fn poll(ticket: &CompileTicket) -> CompileStatus {
        if let Some(status) = ticket.done.borrow().clone() {
            return status;
        }
        let status = match ticket.rx.try_recv() {
            Ok(Ok(p)) => CompileStatus::Ready(p),
            Ok(Err(e)) => CompileStatus::Failed(format!("{e:#}")),
            Err(std::sync::mpsc::TryRecvError::Empty) => return CompileStatus::Pending,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                CompileStatus::Failed("compilation thread died".into())
            }
        };
        *ticket.done.borrow_mut() = Some(status.clone());
        status
    }

    /// Number of compiled topologies held by the cache.
    pub fn cached_pipelines(&self) -> usize {
        self.cache.len()
    }

    /// A uniform buffer for binding 4 holding `values`.
    pub(crate) fn graph_params_buffer(&self, values: &[f32; PARAM_COUNT]) -> wgpu::Buffer {
        let buf = self.ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("graph params"),
            size: GRAPH_PARAMS_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.ctx
            .queue
            .write_buffer(&buf, 0, bytemuck::cast_slice(values));
        buf
    }

    /// Evaluates the GPU distance field at `points` with the scene's fractal,
    /// compiled from `scene.fractal.to_graph()`. Used by the CPU/GPU
    /// consistency harness; also handy for probing surfaces.
    pub fn probe_distances(&self, scene: &Scene, points: &[Vec3]) -> Result<Vec<DeSample>> {
        scene.validate().context("scene validation")?;
        if points.is_empty() {
            return Ok(vec![]);
        }
        let g = scene.fractal.to_graph();
        let pipeline = self.pipeline_for(&g)?;
        let device = &self.ctx.device;
        let oom = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let result = self.probe_inner(scene, &g, &pipeline, points);
        let validation_err = pollster::block_on(validation.pop());
        let oom_err = pollster::block_on(oom.pop());
        if let Some(err) = validation_err {
            return Err(anyhow!("GPU validation error: {err}"));
        }
        if let Some(err) = oom_err {
            return Err(anyhow!("GPU out-of-memory error: {err}"));
        }
        result
    }

    fn probe_inner(
        &self,
        scene: &Scene,
        g: &Graph,
        pipeline: &CompiledPipeline,
        points: &[Vec3],
    ) -> Result<Vec<DeSample>> {
        let device = &self.ctx.device;
        let bytes = (points.len() * 16) as u64;
        let uniforms = Uniforms::from_scene(scene, 1, 1, 1, 1);
        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("probe uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.ctx
            .queue
            .write_buffer(&uniform_buf, 0, bytemuck::bytes_of(&uniforms));
        let params_buf = self.graph_params_buffer(&pipeline.layout.pack(g));
        let input: Vec<[f32; 4]> = points.iter().map(|p| [p.x, p.y, p.z, 0.0]).collect();
        let in_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("probe points"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.ctx
            .queue
            .write_buffer(&in_buf, 0, bytemuck::cast_slice(&input));
        let output = |label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let out_buf = output("probe results");
        let traps_buf = output("probe traps");
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("de_probe"),
            layout: &self.layouts.probe_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: in_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: out_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: traps_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: params_buf.as_entire_binding(),
                },
            ],
        });
        let mut enc = device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline.probe_pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups((points.len() as u32).div_ceil(64), 1, 1);
        }
        self.ctx.queue.submit([enc.finish()]);
        let raw = self.ctx.read_buffer(&out_buf, bytes)?;
        let raw_traps = self.ctx.read_buffer(&traps_buf, bytes)?;
        let rows: Vec<[f32; 4]> = bytemuck::pod_collect_to_vec(&raw);
        let traps: Vec<[f32; 4]> = bytemuck::pod_collect_to_vec(&raw_traps);
        Ok(rows
            .iter()
            .zip(&traps)
            .map(|(r, t)| DeSample {
                material: r[3] as u32,
                distance: r[0],
                traps: *t,
                iterations: r[2] as u32,
            })
            .collect())
    }

    /// Runs `f` inside OutOfMemory and Validation error scopes; a captured GPU
    /// error becomes an `Err`, unless `f` itself already failed.
    pub(crate) fn with_scopes<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        let device = &self.ctx.device;
        let oom = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let result = f();
        let validation_err = pollster::block_on(validation.pop());
        let oom_err = pollster::block_on(oom.pop());
        let value = result?;
        if let Some(err) = validation_err {
            return Err(anyhow!("GPU validation error: {err}"));
        }
        if let Some(err) = oom_err {
            return Err(anyhow!("GPU out-of-memory error: {err}"));
        }
        Ok(value)
    }

    pub fn render(
        &self,
        scene: &Scene,
        cfg: &RenderConfig,
        progress: &mut dyn FnMut(&Progress),
    ) -> Result<RenderOutput> {
        let mut session = RenderSession::new(self.clone(), scene, cfg)?;
        let start = Instant::now();
        loop {
            if let Some(budget) = cfg.time_budget {
                if start.elapsed() >= budget {
                    break;
                }
            }
            let mut slice = Duration::from_millis(250);
            if let Some(budget) = cfg.time_budget {
                slice = slice.min(budget.saturating_sub(start.elapsed()));
            }
            let done = session.step(slice)?;
            progress(&session.progress());
            if done {
                break;
            }
        }
        session.readback()
    }

    /// Submits one tile dispatch and blocks until the GPU finishes, returning
    /// the wall time so the caller can police the TDR limit.
    pub(crate) fn dispatch(
        &self,
        pipeline: &wgpu::ComputePipeline,
        bind_group: &wgpu::BindGroup,
        w: u32,
        h: u32,
        workgroup: [u32; 2],
    ) -> Result<Duration> {
        let mut enc = self.ctx.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.dispatch_workgroups(w.div_ceil(workgroup[0]), h.div_ceil(workgroup[1]), 1);
        }
        let t0 = Instant::now();
        self.ctx.queue.submit([enc.finish()]);
        let polled = self.ctx.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(DISPATCH_TIMEOUT),
        });
        match polled {
            Ok(_) => {}
            Err(wgpu::PollError::Timeout) => {
                let mut msg = format!(
                    "GPU dispatch did not complete within {} s (device lost after a TDR?)",
                    DISPATCH_TIMEOUT.as_secs()
                );
                if let Err(e) = self.ctx.check() {
                    msg.push_str(&format!("; {e}"));
                }
                return Err(anyhow!(msg));
            }
            Err(e) => return Err(anyhow!("device poll failed (device lost?): {e:?}")),
        }
        self.ctx.check()?;
        Ok(t0.elapsed())
    }
}

/// Builds `c` and inserts it into `cache` only when the build succeeds.
fn build_and_cache(
    device: &wgpu::Device,
    layouts: &Layouts,
    cache: &PipelineCache,
    c: Compiled,
) -> Result<Arc<CompiledPipeline>> {
    let p = Arc::new(CompiledPipeline::build(device, layouts, c)?);
    cache.insert(p.clone());
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_validation_names_the_field() {
        let limits = wgpu::Limits::default();
        assert!(RenderConfig::new(64, 64, 1).validate(&limits).is_ok());
        let msg = |c: RenderConfig| c.validate(&limits).unwrap_err().to_string();
        assert!(msg(RenderConfig::new(0, 64, 1)).contains("width"));
        assert!(msg(RenderConfig::new(64, 0, 1)).contains("height"));
        assert!(msg(RenderConfig::new(64, 64, 0)).contains("samples_per_pixel"));
        let mut c = RenderConfig::new(64, 64, 1);
        c.max_bounces = 33;
        assert!(msg(c).contains("max_bounces"));
        let mut c = RenderConfig::new(64, 64, 1);
        c.max_steps = 0;
        assert!(msg(c).contains("max_steps"));
        for value in [f32::NAN, f32::INFINITY, 0.0, 1.1] {
            let mut c = RenderConfig::new(64, 64, 1);
            c.pixel_tolerance = value;
            assert!(msg(c).contains("pixel_tolerance"));
        }
        for value in [0, 49, 801, u32::MAX] {
            let mut c = RenderConfig::new(64, 64, 1);
            c.dispatch_budget_ms = value;
            assert!(msg(c).contains("dispatch_budget_ms"));
        }
        assert!(msg(RenderConfig::new(100_000, 100_000, 1)).contains("max_"));
    }

    #[test]
    fn old_states_default_to_the_legacy_tolerance() {
        let mut c = RenderConfig::new(32, 24, 8);
        c.pixel_tolerance = 0.25;
        let text = ron::to_string(&c).unwrap();
        assert_eq!(ron::from_str::<RenderConfig>(&text).unwrap(), c);
        let old = text.replace("pixel_tolerance:0.25,", "");
        assert_eq!(
            ron::from_str::<RenderConfig>(&old).unwrap().pixel_tolerance,
            1.0
        );
    }

    #[test]
    fn legacy_configs_keep_the_interactive_dispatch_budget() {
        let text="(width:32,height:24,samples_per_pixel:8,max_bounces:4,max_steps:256,pixel_tolerance:1.0,tile_size:256,samples_per_dispatch:8,time_budget:None,denoise:false,guiding:false)";
        let cfg: RenderConfig = ron::from_str(text).unwrap();
        assert_eq!(cfg.dispatch_budget_ms, 400);
        assert_eq!(cfg, RenderConfig::new(32, 24, 8));
    }
}
