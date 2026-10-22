use std::time::{Duration, Instant};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use anyhow::{anyhow, Context, Result};
use glam::Vec3;
use syfrack_core::{DeSample, Graph, HdrImage, Scene};

use crate::gpu::GpuContext;
use crate::graph_wgsl::{compile, compile_texture, fingerprint, scene_fingerprint, Compiled};
use crate::pipeline_cache::{CompileStatus, CompileTicket, CompiledPipeline, PipelineCache};
use crate::session::RenderSession;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RenderConfig {
    /// Missing in old checkpoints: preserve their monolithic transport.
    #[serde(default)]
    pub integrator: ComputeIntegrator,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ComputeIntegrator {
    #[default]
    Megakernel,
    Wavefront,
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
            integrator: ComputeIntegrator::Megakernel,
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

pub(crate) fn separated_volume_layers(scene: &Scene) -> bool {
    scene.dimension == syfrack_core::Dimension::ThreeD
        && (scene.atmosphere.fog_density > 0.0
            || scene
                .phenomena
                .as_ref()
                .is_some_and(|p| p.bindings.density.is_some()))
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
pub enum CpuExportStatus {
    NotRequested,
    Preparing,
    Participating,
    Unavailable(String),
}

#[derive(Clone, Debug)]
pub struct ExportWorkers {
    pub cpu: CpuExportStatus,
    pub cpu_tiles: usize,
    pub gpu_tiles: usize,
}

#[derive(Clone, Debug)]
pub struct Progress {
    pub tiles: usize,
    pub dispatches: u32,
    pub samples_min: u32,
    pub samples_max: u32,
    pub target: u32,
    /// Pixel-weighted progress; unlike samples_min it advances as owned tiles finish.
    pub pixel_samples_done: u64,
    pub pixel_samples_total: u64,
    pub export_workers: Option<ExportWorkers>,
    pub elapsed: Duration,
    /// The slowest single dispatch so far in this session, in milliseconds.
    pub max_dispatch_ms: u32,
    /// Samples per pixel accumulated in each tile, row-major over `tile_grid`.
    pub tiles_done: Vec<u32>,
    /// Tile columns and rows.
    pub tile_grid: (u32, u32),
}

impl Progress {
    pub fn fraction_complete(&self) -> f64 {
        (self.pixel_samples_done as f64 / self.pixel_samples_total.max(1) as f64).clamp(0.0, 1.0)
    }

    pub fn estimated_remaining(&self) -> Option<Duration> {
        let fraction = self.fraction_complete();
        if fraction == 0.0 {
            return None;
        }
        Duration::try_from_secs_f64(self.elapsed.as_secs_f64() * (1.0 - fraction) / fraction).ok()
    }
}

/// Longest a single dispatch may take before the device is presumed lost.
const DISPATCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Bind group and pipeline layouts shared by every compiled graph.
pub struct Layouts {
    /// Path tracer: 0 uniforms, 1-3 accumulation/depth/aux, 4 form,
    /// 5 texture parameters, 6 panorama radiance/CDF, 7 density texture,
    /// 8 volume sampler, 9 cached volume lighting, 16-17 parameter overflow.
    pub bind_group_layout: wgpu::BindGroupLayout,
    /// Probe: 0 uniforms, 1 points, 2 results, 3 traps, 4/16 graph parameters.
    pub probe_layout: wgpu::BindGroupLayout,
    pub pathtrace: wgpu::PipelineLayout,
    pub probe: wgpu::PipelineLayout,
}

/// Size of the fast uniform prefix; larger parameter sets use overflow textures.
pub const GRAPH_PARAMS_BYTES: u64 = (crate::params::UNIFORM_SCALARS * 4) as u64;

#[derive(Clone)]
pub struct Renderer {
    pub(crate) control: Arc<Mutex<Option<Arc<crate::control::ControlEngine>>>>,
    pub(crate) ctx: Arc<GpuContext>,
    pub(crate) layouts: Arc<Layouts>,
    cache: Arc<PipelineCache>,
    cpu_constants: Option<Arc<crate::shader_source::CpuExportConstants>>,
    cpu_variants: Arc<Mutex<VecDeque<(u16, Arc<PipelineCache>)>>>,
    pub(crate) environment_cache: Arc<crate::environment::EnvironmentCache>,
    pub(crate) particle_cache: Arc<crate::particles::ParticleCache>,
    pub(crate) phenomena_cache: Arc<crate::phenomena::PhenomenaCache>,
}

impl Renderer {
    pub fn context(&self) -> &Arc<GpuContext> {
        &self.ctx
    }
    /// Persist the exact GPU-prepared sources in the canonical graph. The same
    /// frozen data is then sampled by control, final rendering and resume.
    pub fn freeze_creation_sources(&self, scene: &Scene) -> Result<Scene> {
        self.freeze_creation_sources_controlled(scene, &Default::default())
    }
    pub fn freeze_creation_sources_controlled(
        &self,
        scene: &Scene,
        control: &crate::phenomena::PreparationControl,
    ) -> Result<Scene> {
        let mut scene = scene.render_scene()?;
        let Some(prepared) = self.prepare_phenomena_controlled(&scene, control)? else {
            return Ok(scene);
        };
        let g = scene.creation.as_mut().unwrap();
        let (simulations, flames) = g.resource_nodes()?;
        for (id, grid) in simulations.into_iter().zip(&prepared.snapshots) {
            if let syfrack_core::CreationKind::Simulation(s) =
                &mut g.nodes.iter_mut().find(|n| n.id == id).unwrap().kind
            {
                s.frozen = Some(syfrack_core::scene::phenomena::FrozenSnapshot {
                    resolution: grid.domain.resolution,
                    values: grid.data.iter().map(|v| v.to_array()).collect(),
                });
            }
        }
        for (id, grid) in flames.into_iter().zip(&prepared.flames) {
            if let syfrack_core::CreationKind::Flame(s) =
                &mut g.nodes.iter_mut().find(|n| n.id == id).unwrap().kind
            {
                s.frozen = Some(grid.clone());
            }
        }
        scene.unify()?;
        Ok(scene)
    }
    pub fn prepare_phenomena_controlled(
        &self,
        scene: &Scene,
        control: &crate::phenomena::PreparationControl,
    ) -> Result<Option<Arc<crate::phenomena::PreparedPhenomena>>> {
        let scene = scene.render_scene()?;
        self.phenomena_cache.get_controlled(self, &scene, control)
    }
    pub fn prepare_phenomena(
        &self,
        scene: &Scene,
    ) -> Result<Option<Arc<crate::phenomena::PreparedPhenomena>>> {
        let scene = scene.render_scene()?;
        self.phenomena_cache.get_gpu(self, &scene)
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
        let parameter_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
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
                parameter_entry(16),
                parameter_entry(17),
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
                parameter_entry(16),
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
            control: Arc::new(Mutex::new(None)),
            ctx,
            layouts: Arc::new(Layouts {
                bind_group_layout,
                probe_layout,
                pathtrace,
                probe,
            }),
            cache: Arc::new(PipelineCache::default()),
            cpu_constants: None,
            cpu_variants: Arc::new(Mutex::new(VecDeque::new())),
            environment_cache: Arc::new(Default::default()),
            particle_cache: Arc::new(Default::default()),
            phenomena_cache: Arc::new(Default::default()),
        })
    }

    pub(crate) fn for_cpu_export(&self, scene: &Scene) -> Self {
        let mut constants = crate::shader_source::CpuExportConstants::from_scene(scene);
        // Match a software SIMD subgroup instead of serializing a full GPU
        // workgroup in the CPU JIT. Pixels have no workgroup-shared state.
        constants.1 = [self.ctx.info.subgroup_min_size.clamp(1, 8), 1];
        let mut variants = self.cpu_variants.lock().unwrap_or_else(|p| p.into_inner());
        let cache = variants
            .iter()
            .position(|(key, _)| *key == constants.0)
            .and_then(|i| variants.remove(i))
            .map(|(_, cache)| cache)
            .unwrap_or_else(|| Arc::new(PipelineCache::default()));
        variants.push_back((constants.0, cache.clone()));
        while variants.len() > 2 {
            variants.pop_front();
        }
        let mut renderer = self.clone();
        renderer.cache = cache;
        renderer.cpu_constants = Some(Arc::new(constants));
        renderer
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
        let resolved = scene.render_scene()?;
        let scene = &resolved;
        scene.validate().context("scene validation")?;
        if scene.material.texture.is_none()
            && scene.atmosphere.nebula.is_none()
            && !crate::graph_wgsl::scene_has_optics(scene)
            && crate::graph_wgsl::nature_features(scene) == 0
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
        let p = Arc::new(CompiledPipeline::build_specialized(
            &self.ctx.device,
            &self.layouts,
            form,
            texture,
            scene.atmosphere.nebula.is_some(),
            crate::graph_wgsl::scene_has_optics(scene),
            self.cpu_constants.as_deref(),
            crate::graph_wgsl::nature_features(scene),
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
        build_and_cache(
            &self.ctx.device,
            &self.layouts,
            &self.cache,
            c,
            self.cpu_constants.as_deref(),
        )
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
        let constants = self.cpu_constants.clone();
        std::thread::Builder::new()
            .name("syfrack-compile".into())
            .spawn(move || {
                let result = g
                    .validate()
                    .map_err(anyhow::Error::from)
                    .and_then(|()| compile(&g).map_err(anyhow::Error::from))
                    .and_then(|c| {
                        build_and_cache(&device, &layouts, &cache, c, constants.as_deref())
                    });
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

    pub(crate) fn graph_params_buffer(&self, values: &[f32]) -> Result<crate::params::Parameters> {
        crate::params::Parameters::new(&self.ctx.device, &self.ctx.queue, values)
    }

    /// Evaluates the GPU distance field at `points` with the scene's fractal,
    /// compiled from `scene.fractal.to_graph()`. Used by the CPU/GPU
    /// consistency harness; also handy for probing surfaces.
    pub fn probe_distances(&self, scene: &Scene, points: &[Vec3]) -> Result<Vec<DeSample>> {
        scene.validate().context("scene validation")?;
        crate::parity::probe_scene(self, scene, points)
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

    /// Final export with host assembly and bounded tile buffers. The output
    /// remains linear HDR; develop it once after the workers have finished.
    pub fn export(
        &self,
        scene: &Scene,
        cfg: &RenderConfig,
        mode: crate::ExportMode,
        progress: &mut dyn FnMut(&Progress),
    ) -> Result<RenderOutput> {
        let mut session = crate::ExportSession::new(self.clone(), scene, cfg, mode)?;
        let start = Instant::now();
        loop {
            if cfg
                .time_budget
                .is_some_and(|budget| start.elapsed() >= budget)
            {
                break;
            }
            let slice = cfg
                .time_budget
                .map_or(Duration::from_millis(250), |budget| {
                    Duration::from_millis(250).min(budget.saturating_sub(start.elapsed()))
                });
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
        profile: Option<&crate::execution_profile::ExecutionProfile>,
    ) -> Result<Duration> {
        let mut enc = self.ctx.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.dispatch_workgroups(w.div_ceil(workgroup[0]), h.div_ceil(workgroup[1]), 1);
        }
        let _activity =
            profile.map(|p| p.enter(self.ctx.info.device_type == wgpu::DeviceType::Cpu));
        let t0 = Instant::now();
        let submission = self.ctx.queue.submit([enc.finish()]);
        let polled = self.ctx.device.poll(wgpu::PollType::Wait {
            // Studio shares this queue with UI/preview submissions. Wait for
            // this dispatch, rather than unrelated work submitted afterwards.
            submission_index: Some(submission),
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
    constants: Option<&crate::shader_source::CpuExportConstants>,
) -> Result<Arc<CompiledPipeline>> {
    let p = Arc::new(CompiledPipeline::build_specialized(
        device, layouts, c, None, false, false, constants, 0,
    )?);
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
