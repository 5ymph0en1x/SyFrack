//! Host-assembled exports with bounded GPU residency and heterogeneous workers.
//! No worker touches the host image. Complete tiles cross a bounded channel;
//! only the collector commits them, so snapshots cannot contain half a tile.
use crate::export_queue::TileWorkQueue;
use crate::session::{ExportSlot, TileResult};
use crate::{
    GpuContext, Progress, RadianceLayers, RenderConfig, RenderOutput, RenderSession, Renderer,
    SessionState, Tile, TileGrid, VolumeRadianceComponents,
};
use anyhow::{anyhow, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use syfrack_core::{HdrImage, Scene};

/// Auto uses bounded GPU tiles. Software CPU rendering requires an explicit
/// diagnostic `Cpu` or `Hybrid` request; ordinary exports never start its JIT.
/// Features with global adaptive state (guiding, flame histograms) retain their
/// resident implementation. Explicit tiled/hybrid requests reject those features.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExportMode {
    #[default]
    Auto,
    Gpu,
    Tiled,
    Wavefront,
    Hybrid,
    Cpu,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExportCheckpoint {
    version: u32,
    mode: ExportMode,
    layers: bool,
    components: bool,
    guides: usize,
    bytes: u64,
    hash: u64,
    #[serde(default)]
    producers: Vec<ProducerIdentity>,
    #[serde(default)]
    previous_producers: Vec<ProducerIdentity>,
}

/// Native floating point is an implementation contract, not a WGSL source
/// contract. Record both the driver and the compiled arithmetic variant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ProducerIdentity {
    adapter: String,
    backend: String,
    device_type: String,
    vendor: u32,
    device: u32,
    driver: String,
    driver_info: String,
    module: u64,
    workgroup: [u32; 2],
}

impl ProducerIdentity {
    pub(crate) fn new(info: &wgpu::AdapterInfo, module: u64, workgroup: [u32; 2]) -> Self {
        Self {
            adapter: info.name.clone(),
            backend: format!("{:?}", info.backend),
            device_type: format!("{:?}", info.device_type),
            vendor: info.vendor,
            device: info.device,
            driver: info.driver.clone(),
            driver_info: info.driver_info.clone(),
            module,
            workgroup,
        }
    }
}

fn verify_producers(saved: &[ProducerIdentity], current: &[ProducerIdentity]) -> Result<()> {
    // Existing version-1 states predate this metadata and remain readable;
    // they cannot retrospectively certify their native arithmetic producer.
    ensure!(saved.is_empty() || saved == current,
        "checkpoint arithmetic producer changed (adapter, driver, workgroup or CPU specialization); native cross-device bit-exact parity is not certified; start a new render (saved files are unchanged)");
    Ok(())
}

/// Common export API, preserving the existing interactive engine and legacy
/// checkpoints. Development/tonemapping belongs after `readback`, globally.
pub enum ExportSession {
    Resident(RenderSession),
    Tiled(Box<TiledExport>),
}

impl From<RenderSession> for ExportSession {
    fn from(s: RenderSession) -> Self {
        Self::Resident(s)
    }
}

impl ExportSession {
    pub fn new(
        renderer: Renderer,
        scene: &Scene,
        cfg: &RenderConfig,
        mode: ExportMode,
    ) -> Result<Self> {
        let resolved = scene.render_scene()?;
        let scene = &resolved;
        if mode == ExportMode::Cpu {
            return Self::cpu(scene, cfg);
        }
        if mode == ExportMode::Gpu
            || (mode == ExportMode::Auto && (cfg.guiding || scene.canvas.flame.is_some()))
        {
            return Ok(RenderSession::new(renderer, scene, cfg)?.into());
        }
        ensure!(
            !cfg.guiding,
            "tiled export does not yet support global path guiding; use --backend gpu or auto"
        );
        ensure!(
            scene.canvas.flame.is_none(),
            "flame histograms need the resident renderer; use --backend gpu or auto"
        );
        Ok(Self::Tiled(Box::new(TiledExport::new(
            renderer, scene, cfg, mode, None, false,
        )?)))
    }

    pub fn resume(renderer: Renderer, dir: &Path) -> Result<Self> {
        Self::resume_impl(renderer, dir, None)
    }

    /// Run the common integrator on the software CPU adapter, with no hardware
    /// GPU requirement. This is not a second Rust transport implementation.
    pub fn cpu(scene: &Scene, cfg: &RenderConfig) -> Result<Self> {
        Ok(Self::Tiled(Box::new(TiledExport::new(
            software_renderer()?.for_cpu_export(scene),
            scene,
            cfg,
            ExportMode::Cpu,
            None,
            false,
        )?)))
    }

    /// An explicit backend switch accepts native floating-point differences.
    /// Completed tiles are retained; their producer history is preserved.
    pub fn resume_with_backend(renderer: Renderer, dir: &Path, mode: ExportMode) -> Result<Self> {
        Self::resume_impl(renderer, dir, Some(mode))
    }

    pub fn resume_cpu(dir: &Path) -> Result<Self> {
        Self::resume_with_backend(software_renderer()?, dir, ExportMode::Cpu)
    }

    fn resume_impl(renderer: Renderer, dir: &Path, requested: Option<ExportMode>) -> Result<Self> {
        let state: SessionState = ron::from_str(&std::fs::read_to_string(dir.join("state.ron"))?)?;
        if state.export.is_none() {
            ensure!(requested.is_none() || requested == Some(ExportMode::Gpu),
                "resident checkpoints cannot be converted to tiled/CPU accumulators; resume without --backend or use gpu");
            return Ok(RenderSession::resume(renderer, dir)?.into());
        }
        let mode = requested.unwrap_or(state.export.as_ref().unwrap().mode);
        let mode = if mode == ExportMode::Gpu {
            ExportMode::Tiled
        } else {
            mode
        };
        let renderer = if mode == ExportMode::Cpu {
            software_renderer()?.for_cpu_export(&state.scene)
        } else {
            renderer
        };
        Ok(Self::Tiled(Box::new(TiledExport::new(
            renderer,
            &state.scene,
            &state.cfg,
            mode,
            Some((&state, dir)),
            requested.is_some(),
        )?)))
    }

    pub fn scene(&self) -> &Scene {
        match self {
            Self::Resident(s) => s.scene(),
            Self::Tiled(s) => &s.scene,
        }
    }
    pub fn config(&self) -> &RenderConfig {
        match self {
            Self::Resident(s) => s.config(),
            Self::Tiled(s) => &s.cfg,
        }
    }
    pub fn is_done(&self) -> bool {
        match self {
            Self::Resident(s) => s.is_done(),
            Self::Tiled(s) => s.grid.next_pending(s.cfg.samples_per_pixel).is_none(),
        }
    }
    pub fn step(&mut self, budget: Duration) -> Result<bool> {
        match self {
            Self::Resident(s) => s.step(budget),
            Self::Tiled(s) => s.step(budget),
        }
    }
    pub fn progress(&self) -> Progress {
        match self {
            Self::Resident(s) => s.progress(),
            Self::Tiled(s) => s.progress(),
        }
    }
    pub fn readback(&self) -> Result<RenderOutput> {
        match self {
            Self::Resident(s) => s.readback(),
            Self::Tiled(s) => {
                let mut out = s.output.clone();
                out.samples_min = s.grid.min_done();
                out.samples_max = s.grid.max_done();
                out.elapsed = s.elapsed();
                Ok(out)
            }
        }
    }
    pub fn save(&self, dir: &Path) -> Result<()> {
        match self {
            Self::Resident(s) => s.save(dir),
            Self::Tiled(s) => s.save(dir),
        }
    }
    /// Quiesce tiled workers before publishing a terminal UI event. Completed
    /// host tiles remain readable/saveable; uncommitted work is discarded.
    pub fn stop_workers(&mut self) {
        if let Self::Tiled(session) = self {
            session.stop_workers();
        }
    }
    pub fn set_dispatch_budget_ms(&mut self, milliseconds: u32) -> Result<()> {
        match self {
            Self::Resident(s) => s.set_dispatch_budget_ms(milliseconds),
            Self::Tiled(s) => {
                ensure!(
                    (50..=800).contains(&milliseconds),
                    "dispatch budget must be between 50 and 800 ms"
                );
                s.cfg.dispatch_budget_ms = milliseconds;
                s.dispatch_budget
                    .store(milliseconds as u64, Ordering::Release);
                Ok(())
            }
        }
    }
    pub fn export_guides(&self) -> Result<(HdrImage, HdrImage)> {
        self.guides(false)
    }
    pub fn export_surface_guides(&self) -> Result<(HdrImage, HdrImage)> {
        self.guides(true)
    }
    fn guides(&self, surface: bool) -> Result<(HdrImage, HdrImage)> {
        match self {
            Self::Resident(s) => {
                if surface {
                    s.export_surface_guides()
                } else {
                    s.export_guides()
                }
            }
            Self::Tiled(s) => {
                ensure!(
                    s.grid.min_done() > 0,
                    "all pixels need a first sample for export guides"
                );
                s.guides
                    .get(usize::from(surface))
                    .cloned()
                    .context("material guides were not requested; enable denoise before rendering")
            }
        }
    }
}

enum WorkerEvent {
    ProducerReady(ProducerIdentity),
    CpuUnavailable(String),
    Tile(Box<TileResult>, bool),
    Failed(String),
    Finished,
}

pub struct TiledExport {
    scene: Scene,
    cfg: RenderConfig,
    mode: ExportMode,
    grid: TileGrid,
    output: RenderOutput,
    guides: Vec<(HdrImage, HdrImage)>,
    identity: (u64, u64, u64),
    producers: Vec<ProducerIdentity>,
    previous_producers: Vec<ProducerIdentity>,
    results: Option<mpsc::Receiver<WorkerEvent>>,
    workers: Vec<JoinHandle<()>>,
    cpu_preparing: Option<Arc<AtomicBool>>,
    cpu_unavailable: Option<String>,
    stopped: Arc<AtomicBool>,
    dispatch_budget: Arc<AtomicU64>,
    started: Instant,
    previous_elapsed: Duration,
    dispatches: u32,
    max_dispatch: Duration,
    finished_workers: usize,
    cpu_tiles: usize,
    gpu_tiles: usize,
    last_tile: [Duration; 2],
    execution_profile: Arc<crate::execution_profile::ExecutionProfile>,
    device_buffer_bytes: u64,
    failure: Option<String>,
}

fn plane<T: Clone>(count: usize, value: T) -> Result<Vec<T>> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .context("insufficient host memory for export")?;
    values.resize(count, value);
    Ok(values)
}

fn image(w: u32, h: u32) -> Result<HdrImage> {
    Ok(HdrImage::from_data(
        w,
        h,
        plane(w as usize * h as usize, [0.0, 0.0, 0.0, 1.0])?,
    ))
}

fn blank(cfg: &RenderConfig, layers: bool, components: bool) -> Result<RenderOutput> {
    let (w, h) = (cfg.width, cfg.height);
    let n = w as usize * h as usize;
    Ok(RenderOutput {
        color: image(w, h)?,
        depth: plane(n, -1.0)?,
        normals: plane(n, [0.0, 0.0, 0.0, -1.0])?,
        layers: if layers {
            Some(RadianceLayers {
                surface: image(w, h)?,
                volume: image(w, h)?,
                volume_variance: plane(n, [0.0; 3])?,
                components: if components {
                    Some(VolumeRadianceComponents {
                        direct: image(w, h)?,
                        direct_variance: plane(n, [0.0; 3])?,
                        indirect_variance: plane(n, [0.0; 3])?,
                    })
                } else {
                    None
                },
            })
        } else {
            None
        },
        samples_min: 0,
        samples_max: 0,
        elapsed: Duration::ZERO,
    })
}

fn software_renderer() -> Result<Renderer> {
    static CPU: OnceLock<std::result::Result<Renderer, String>> = OnceLock::new();
    let renderer = CPU.get_or_init(|| {
        GpuContext::new_cpu()
            .map(Arc::new)
            .and_then(Renderer::new)
            .map_err(|e| format!("{e:#}"))
    });
    match renderer {
        Ok(renderer) => Ok(renderer.clone()),
        Err(e) => Err(anyhow!("{e}")),
    }
}

impl TiledExport {
    fn new(
        renderer: Renderer,
        scene: &Scene,
        cfg: &RenderConfig,
        mode: ExportMode,
        restore: Option<(&SessionState, &Path)>,
        allow_native_changes: bool,
    ) -> Result<Self> {
        let resolved = scene.render_scene()?;
        let scene = &resolved;
        scene.validate()?;
        let mut effective_cfg = cfg.clone();
        if mode == ExportMode::Wavefront
            || (mode == ExportMode::Auto
                && restore.is_none()
                && scene.dimension == syfrack_core::Dimension::ThreeD)
        {
            effective_cfg.integrator = crate::ComputeIntegrator::Wavefront;
        }
        let cfg = &effective_cfg;
        let mut scene = scene.clone();
        scene.resolve_asset_paths(Path::new("."))?;
        ensure!(
            !cfg.guiding && scene.canvas.flame.is_none(),
            "invalid tiled checkpoint features"
        );
        // Global coordinates stay u32 in WGSL; validate before any allocation.
        let pixels = u64::from(cfg.width) * u64::from(cfg.height);
        ensure!(
            cfg.width > 0 && cfg.height > 0 && pixels <= u32::MAX as u64,
            "invalid tiled image dimensions"
        );
        let layers = crate::renderer::separated_volume_layers(&scene);
        let guides = cfg.denoise && scene.dimension == syfrack_core::Dimension::ThreeD;
        // Budget for assembled host planes. Readback and denoising need
        // additional working copies, as documented for large exports.
        let host_bytes = pixels * (36 + if layers { 100 } else { 0 } + if guides { 64 } else { 0 });
        ensure!(host_bytes <= 8*1024*1024*1024u64, "export host planes require {host_bytes} bytes, above the 8 GiB plane budget; reduce resolution or disable denoise");
        let size = restore.map_or(
            cfg.tile_size.clamp(crate::tiles::MIN_TILE_SIZE, 16),
            |(state, _)| state.tile_size,
        );
        ensure!(
            (crate::tiles::MIN_TILE_SIZE..=256).contains(&size),
            "invalid tiled checkpoint tile size"
        );
        let mut local = cfg.clone();
        local.width = size;
        local.height = size;
        local.validate(&renderer.context().device.limits())?;
        let mut primary = Vec::new();
        let primary_cpu = renderer.context().info.device_type == wgpu::DeviceType::Cpu;
        // Larger images amortize readback/submission over 128x128 blocks.
        // Within a block the existing measured dispatch backoff still applies.
        let gpu_block_size = if pixels >= 256 * 256 {
            if cfg.integrator == crate::ComputeIntegrator::Wavefront {
                256
            } else {
                128
            }
        } else {
            64
        };
        for _ in 0..if primary_cpu { 1 } else { 2 } {
            primary.push(ExportSlot::new(
                renderer.clone(),
                &scene,
                cfg,
                if primary_cpu {
                    size
                } else {
                    size.max(gpu_block_size)
                },
            )?);
        }
        let identity = primary[0].identity();
        let mut producers = vec![primary[0].producer_identity()];
        let device_buffer_bytes = primary.iter().map(ExportSlot::working_bytes).sum();
        let mut secondary = None;
        let mut cpu_unavailable = None;
        if mode == ExportMode::Hybrid {
            if renderer.context().info.device_type == wgpu::DeviceType::Cpu {
                ensure!(
                    mode != ExportMode::Hybrid,
                    "hybrid export requires a hardware GPU and a distinct software CPU adapter"
                );
            } else {
                let prepared = software_renderer().map(|cpu| cpu.for_cpu_export(&scene));
                match prepared {
                    Ok(value) => {
                        secondary = Some(value);
                    }
                    Err(error) if mode == ExportMode::Auto => {
                        eprintln!("CPU export worker unavailable; using tiled GPU: {error:#}");
                        cpu_unavailable = Some(format!("{error:#}"));
                    }
                    Err(error) => return Err(error).context("hybrid CPU worker"),
                }
            }
        }
        let mut grid = TileGrid::new(cfg.width, cfg.height, size);
        let mut output = blank(cfg, layers, layers)?;
        let mut guide_images = Vec::new();
        if guides {
            for _ in 0..2 {
                guide_images.push((image(cfg.width, cfg.height)?, image(cfg.width, cfg.height)?));
            }
        }
        let mut previous_elapsed = Duration::ZERO;
        let mut previous_producers = Vec::new();
        let mut dispatches = 0;
        if let Some((state, dir)) = restore {
            let metadata = state
                .export
                .as_ref()
                .context("missing tiled checkpoint metadata")?;
            ensure!(
                (1..=2).contains(&metadata.version),
                "unsupported tiled checkpoint version"
            );
            ensure!(
                metadata.version == 1 || !metadata.producers.is_empty(),
                "version-2 tiled checkpoint is missing arithmetic producer identities"
            );
            previous_producers = metadata.previous_producers.clone();
            if allow_native_changes && metadata.producers != producers {
                for producer in &metadata.producers {
                    if !previous_producers.contains(producer) {
                        previous_producers.push(producer.clone());
                    }
                }
                eprintln!("Resuming on the requested backend; native CPU/GPU floating-point differences are accepted. Previous producers remain recorded in the checkpoint.");
            } else {
                let primary_saved: Vec<_> = metadata
                    .producers
                    .iter()
                    .filter(|p| (p.device_type == "Cpu") == primary_cpu)
                    .cloned()
                    .collect();
                verify_producers(&primary_saved, &producers)?;
                // CPU preparation is asynchronous. Retain its saved identity
                // and verify it on that worker before it can claim a tile.
                if secondary.is_some() {
                    producers.extend(
                        metadata
                            .producers
                            .iter()
                            .filter(|p| p.device_type == "Cpu")
                            .cloned(),
                    );
                } else {
                    for producer in &metadata.producers {
                        if !producers.contains(producer) && !previous_producers.contains(producer) {
                            previous_producers.push(producer.clone());
                        }
                    }
                }
            }
            ensure!(state.shader_fingerprint==Some(identity.0) && state.environment_fingerprint==identity.1 && state.particle_fingerprint==identity.2,
                "checkpoint transport or scene resources changed; start a new render (saved files are unchanged)");
            ensure!(
                metadata.layers == layers
                    && metadata.components == layers
                    && metadata.guides == guide_images.len(),
                "checkpoint plane layout differs from configuration"
            );
            grid = TileGrid::restore(
                cfg.width,
                cfg.height,
                size,
                state.tiles.clone(),
                cfg.samples_per_pixel,
            )
            .map_err(|e| anyhow!("state.ron: {e}"))?;
            let canonical = TileGrid::new(cfg.width, cfg.height, size);
            ensure!(
                grid.tiles.len() == canonical.tiles.len()
                    && grid
                        .tiles
                        .iter()
                        .zip(&canonical.tiles)
                        .all(|(a, b)| (a.x, a.y, a.w, a.h) == (b.x, b.y, b.w, b.h)),
                "tiled checkpoint has a noncanonical layout"
            );
            ensure!(
                grid.tiles
                    .iter()
                    .all(|t| t.done == 0 || t.done == cfg.samples_per_pixel),
                "tiled checkpoints only contain complete tiles"
            );
            read_payload(dir, &mut output, &mut guide_images, metadata)?;
            previous_elapsed = Duration::try_from_secs_f64(state.elapsed_secs)?;
            dispatches = state.dispatches;
        }
        let queue = Arc::new(TileWorkQueue::with_gpu_block_size(
            grid.tiles
                .iter()
                .copied()
                .filter(|t| t.done < cfg.samples_per_pixel)
                .collect(),
            gpu_block_size,
        ));
        let stopped = Arc::new(AtomicBool::new(false));
        let dispatch_budget = Arc::new(AtomicU64::new(cfg.dispatch_budget_ms as u64));
        let (tx, rx) = mpsc::sync_channel(3);
        let mut workers = Vec::new();
        let mut cpu_preparing = None;
        let started = Instant::now();
        let execution_profile = Arc::new(crate::execution_profile::ExecutionProfile::new(started));
        let mut spawn = |renderer, slots, cpu| -> Result<()> {
            let (queue, stopped, tx, budget) = (
                queue.clone(),
                stopped.clone(),
                tx.clone(),
                dispatch_budget.clone(),
            );
            let full = [cfg.width, cfg.height];
            let profile = execution_profile.clone();
            workers.push(
                std::thread::Builder::new()
                    .name(
                        if cpu {
                            "syfrack-export-cpu"
                        } else {
                            "syfrack-export-gpu"
                        }
                        .into(),
                    )
                    .spawn(move || {
                        worker(
                            renderer, slots, queue, stopped, tx, budget, full, cpu, profile,
                        )
                    })?,
            );
            Ok(())
        };
        let spawned = (|| -> Result<()> {
            spawn(renderer, primary, primary_cpu)?;
            Ok(())
        })();
        if let Err(error) = spawned {
            stopped.store(true, Ordering::Release);
            drop(tx);
            drop(rx);
            for worker in workers {
                let _ = worker.join();
            }
            return Err(error).context("starting export workers");
        }
        if let Some(cpu) = secondary {
            let profile = execution_profile.clone();
            let stop_on_failure = stopped.clone();
            let preparing = Arc::new(AtomicBool::new(true));
            let (queue, stopped, tx, budget, preparing_worker) = (
                queue.clone(),
                stopped.clone(),
                tx.clone(),
                dispatch_budget.clone(),
                preparing.clone(),
            );
            let (scene, cfg) = (scene.clone(), cfg.clone());
            let expected: Vec<_> = if allow_native_changes {
                vec![]
            } else {
                producers
                    .iter()
                    .filter(|p| p.device_type == "Cpu")
                    .cloned()
                    .collect()
            };
            let spawned = std::thread::Builder::new()
                .name("syfrack-export-cpu".into())
                .spawn(move || {
                    let prepared = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                        || -> Result<Option<ExportSlot>> {
                            let Some(_permit) = cpu_permit(&stopped) else {
                                return Ok(None);
                            };
                            if stopped.load(Ordering::Acquire) {
                                return Ok(None);
                            }
                            let mut slot = ExportSlot::new(cpu.clone(), &scene, &cfg, size)?;
                            ensure!(
                                slot.identity() == identity,
                                "CPU and GPU prepared different transport resources"
                            );
                            let producer = slot.producer_identity();
                            verify_producers(&expected, std::slice::from_ref(&producer))?;
                            warm_cpu(&mut slot, [cfg.width, cfg.height], &stopped)?;
                            if stopped.load(Ordering::Acquire) {
                                return Ok(None);
                            }
                            if tx.send(WorkerEvent::ProducerReady(producer)).is_err() {
                                return Ok(None);
                            }
                            Ok(Some(slot))
                        },
                    ));
                    // RMW pairs with stop_workers: if stop wins this handoff,
                    // its cancellation store happens-before the worker can
                    // claim real work. If preparation wins, stop joins it.
                    preparing_worker.swap(false, Ordering::AcqRel);
                    match prepared {
                        Ok(Ok(Some(slot))) => worker(
                            cpu,
                            vec![slot],
                            queue,
                            stopped,
                            tx,
                            budget,
                            [cfg.width, cfg.height],
                            true,
                            profile,
                        ),
                        result => {
                            let error = match result {
                                Ok(Err(e)) => Some(format!("CPU export preparation: {e:#}")),
                                Err(_) => Some("CPU export preparation panicked".into()),
                                _ => None,
                            };
                            if let Some(error) = error {
                                if mode == ExportMode::Auto && expected.is_empty() {
                                    eprintln!("CPU worker unavailable; continuing on GPU: {error}");
                                    let _ = tx.send(WorkerEvent::CpuUnavailable(error));
                                } else {
                                    let _ = tx.send(WorkerEvent::Failed(error));
                                }
                            }
                            let _ = tx.send(WorkerEvent::Finished);
                        }
                    }
                });
            match spawned {
                Ok(handle) => {
                    workers.push(handle);
                    cpu_preparing = Some(preparing);
                }
                Err(error) => {
                    stop_on_failure.store(true, Ordering::Release);
                    drop(rx);
                    for worker in workers {
                        let _ = worker.join();
                    }
                    return Err(error).context("starting CPU export worker");
                }
            }
        }
        drop(tx);
        Ok(Self {
            scene,
            cfg: cfg.clone(),
            mode,
            grid,
            output,
            guides: guide_images,
            identity,
            producers,
            previous_producers,
            results: Some(rx),
            workers,
            cpu_preparing,
            cpu_unavailable,
            stopped,
            dispatch_budget,
            started,
            previous_elapsed,
            dispatches,
            max_dispatch: Duration::ZERO,
            finished_workers: 0,
            cpu_tiles: 0,
            gpu_tiles: 0,
            last_tile: [Duration::ZERO; 2],
            execution_profile,
            device_buffer_bytes,
            failure: None,
        })
    }

    fn elapsed(&self) -> Duration {
        self.previous_elapsed + self.started.elapsed()
    }
    pub fn worker_tiles(&self) -> (usize, usize) {
        (self.cpu_tiles, self.gpu_tiles)
    }

    pub fn work_statistics(&self) -> crate::hybrid_benchmark::WorkStatistics {
        crate::hybrid_benchmark::WorkStatistics {
            producers: self
                .producers
                .iter()
                .map(|p| {
                    format!(
                        "{} ({}/{}, driver {} {})",
                        p.adapter, p.backend, p.device_type, p.driver, p.driver_info
                    )
                })
                .collect(),
            cpu_tiles: self.cpu_tiles,
            gpu_tiles: self.gpu_tiles,
            tile_size: self.grid.tile_size,
            cpu_last_tile_ms: self.last_tile[0].as_secs_f64() * 1000.0,
            gpu_last_tile_ms: self.last_tile[1].as_secs_f64() * 1000.0,
            dispatch_timing: self.execution_profile.timing(),
        }
    }
    /// Actual output and staging buffers on the primary device, excluding
    /// shared scene resources and the separate software adapter's host memory.
    pub fn device_buffer_bytes(&self) -> u64 {
        self.device_buffer_bytes
    }

    fn progress(&self) -> Progress {
        Progress {
            tiles: self.grid.tiles.len(),
            dispatches: self.dispatches,
            samples_min: self.grid.min_done(),
            samples_max: self.grid.max_done(),
            target: self.cfg.samples_per_pixel,
            pixel_samples_done: self.grid.pixel_samples_done(),
            export_workers: Some(crate::renderer::ExportWorkers {
                cpu: if let Some(reason) = &self.cpu_unavailable {
                    crate::renderer::CpuExportStatus::Unavailable(reason.clone())
                } else if self
                    .cpu_preparing
                    .as_ref()
                    .is_some_and(|p| p.load(Ordering::Acquire))
                {
                    crate::renderer::CpuExportStatus::Preparing
                } else if self.producers.iter().any(|p| p.device_type == "Cpu") {
                    crate::renderer::CpuExportStatus::Participating
                } else {
                    crate::renderer::CpuExportStatus::NotRequested
                },
                cpu_tiles: self.cpu_tiles,
                gpu_tiles: self.gpu_tiles,
            }),
            pixel_samples_total: u64::from(self.cfg.width)
                * u64::from(self.cfg.height)
                * u64::from(self.cfg.samples_per_pixel),
            elapsed: self.elapsed(),
            max_dispatch_ms: self.max_dispatch.as_millis().min(u32::MAX as u128) as u32,
            tiles_done: self.grid.progress_cells(),
            tile_grid: (
                self.cfg.width.div_ceil(self.grid.tile_size),
                self.cfg.height.div_ceil(self.grid.tile_size),
            ),
        }
    }

    fn step(&mut self, budget: Duration) -> Result<bool> {
        if let Some(error) = &self.failure {
            return Err(anyhow!("{error}"));
        }
        let start = Instant::now();
        loop {
            if self.grid.next_pending(self.cfg.samples_per_pixel).is_none() {
                return Ok(true);
            }
            let result = self
                .results
                .as_ref()
                .context("tiled export workers have stopped")?
                .recv_timeout(budget.saturating_sub(start.elapsed()));
            match result {
                Ok(WorkerEvent::ProducerReady(producer)) => {
                    if !self.producers.contains(&producer) {
                        self.producers.push(producer);
                    }
                }
                Ok(WorkerEvent::CpuUnavailable(reason)) => self.cpu_unavailable = Some(reason),
                Ok(WorkerEvent::Tile(result, cpu)) => {
                    let indices = self.validate_result(result.tile)?;
                    commit(&mut self.output, &mut self.guides, &result);
                    for &index in &indices {
                        self.grid.tiles[index].done = self.cfg.samples_per_pixel;
                    }
                    self.dispatches = self.dispatches.saturating_add(result.dispatches);
                    self.max_dispatch = self.max_dispatch.max(result.max_dispatch);
                    self.last_tile[usize::from(!cpu)] = self.started.elapsed();
                    if cpu {
                        self.cpu_tiles += indices.len();
                    } else {
                        self.gpu_tiles += indices.len();
                    }
                }
                Ok(WorkerEvent::Failed(error)) => {
                    self.stopped.store(true, Ordering::Release);
                    self.failure = Some(error.clone());
                    return Err(anyhow!("{error}"));
                }
                Ok(WorkerEvent::Finished) => self.finished_workers += 1,
                Err(mpsc::RecvTimeoutError::Timeout) => return Ok(false),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(anyhow!("export workers exited before completing the image"))
                }
            }
            if self.finished_workers == self.workers.len()
                && self.grid.next_pending(self.cfg.samples_per_pixel).is_some()
            {
                return Err(anyhow!("export workers finished with missing tiles"));
            }
            if start.elapsed() >= budget {
                return Ok(self.grid.next_pending(self.cfg.samples_per_pixel).is_none());
            }
        }
    }

    fn save(&self, dir: &Path) -> Result<()> {
        atomic_checkpoint(dir, |temp| {
            let (bytes, hash) = write_payload(temp, &self.output, &self.guides)?;
            let state = SessionState {
                export: Some(ExportCheckpoint {
                    version: 2,
                    mode: self.mode,
                    layers: self.output.layers.is_some(),
                    components: self
                        .output
                        .layers
                        .as_ref()
                        .is_some_and(|l| l.components.is_some()),
                    guides: self.guides.len(),
                    bytes,
                    hash,
                    producers: self.producers.clone(),
                    previous_producers: self.previous_producers.clone(),
                }),
                shader_fingerprint: Some(self.identity.0),
                environment_fingerprint: self.identity.1,
                particle_fingerprint: self.identity.2,
                guiding_updates: 0,
                guiding_fingerprint: None,
                guiding_version: 0,
                scene: self.scene.clone(),
                cfg: self.cfg.clone(),
                tiles: self.grid.tiles.clone(),
                tile_size: self.grid.tile_size,
                spd: 1,
                dispatches: self.dispatches,
                elapsed_secs: self.elapsed().as_secs_f64(),
            };
            std::fs::write(
                temp.join("state.ron"),
                ron::ser::to_string_pretty(&state, ron::ser::PrettyConfig::new().depth_limit(6))?,
            )?;
            Ok(())
        })
    }

    fn validate_result(&self, tile: Tile) -> Result<Vec<usize>> {
        let size = self.grid.tile_size;
        ensure!(
            tile.done == 0
                && tile.w > 0
                && tile.h > 0
                && tile.x % size == 0
                && tile.y % size == 0
                && tile
                    .x
                    .checked_add(tile.w)
                    .is_some_and(|r| r <= self.cfg.width)
                && tile
                    .y
                    .checked_add(tile.h)
                    .is_some_and(|b| b <= self.cfg.height),
            "invalid export rectangle"
        );
        let cols = self.cfg.width.div_ceil(size);
        let mut indices = Vec::new();
        let mut pixels = 0u64;
        for y in (tile.y..tile.y + tile.h).step_by(size as usize) {
            for x in (tile.x..tile.x + tile.w).step_by(size as usize) {
                let index = (y / size * cols + x / size) as usize;
                let t = self
                    .grid
                    .tiles
                    .get(index)
                    .context("invalid export tile index")?;
                ensure!(
                    t.done == 0
                        && t.x == x
                        && t.y == y
                        && t.x + t.w <= tile.x + tile.w
                        && t.y + t.h <= tile.y + tile.h,
                    "duplicate or invalid export tile"
                );
                pixels += u64::from(t.w) * u64::from(t.h);
                indices.push(index);
            }
        }
        ensure!(
            pixels == u64::from(tile.w) * u64::from(tile.h),
            "export rectangle has holes"
        );
        Ok(indices)
    }

    fn stop_workers(&mut self) {
        self.stopped.store(true, Ordering::Release);
        self.results.take();
        for (index, worker) in self.workers.drain(..).enumerate() {
            if index == 1
                && self
                    .cpu_preparing
                    .as_ref()
                    .is_some_and(|p| p.swap(false, Ordering::AcqRel))
            {
                // A native compiler cannot be interrupted safely. It owns no
                // real tile, and only shared immutable resources; let it finish
                // in the background. The permit bounds this to one CPU JIT.
                drop(worker);
            } else {
                let _ = worker.join();
            }
        }
    }
}

impl Drop for TiledExport {
    fn drop(&mut self) {
        // Drop the receiver before joining: a worker may be blocked on the
        // bounded result channel when the user cancels or closes the app.
        self.stop_workers();
    }
}

static CPU_EXECUTION: Mutex<()> = Mutex::new(());

fn cpu_permit(stop: &AtomicBool) -> Option<std::sync::MutexGuard<'static, ()>> {
    loop {
        if stop.load(Ordering::Acquire) {
            return None;
        }
        match CPU_EXECUTION.try_lock() {
            Ok(guard) => return Some(guard),
            Err(std::sync::TryLockError::Poisoned(error)) => return Some(error.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => {
                std::thread::park_timeout(Duration::from_millis(5))
            }
        }
    }
}

fn warm_cpu(slot: &mut ExportSlot, full: [u32; 2], stop: &AtomicBool) -> Result<()> {
    static WARM: Mutex<
        Vec<(
            std::sync::Weak<crate::pipeline_cache::CompiledPipeline>,
            bool,
        )>,
    > = Mutex::new(Vec::new());
    let (pipeline, guides) = slot.cpu_pipeline();
    if WARM
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .any(|(p, warmed_guides)| {
            (!guides || *warmed_guides) && p.upgrade().is_some_and(|p| Arc::ptr_eq(&p, &pipeline))
        })
    {
        return Ok(());
    }
    slot.warm_cpu(full, stop)?;
    if !stop.load(Ordering::Acquire) {
        let mut warm = WARM.lock().unwrap_or_else(|e| e.into_inner());
        warm.retain(|(p, _)| p.strong_count() > 0);
        // A colour-only export does not warm the separate material-guide
        // pipelines. Enabling denoising later must warm those before claiming
        // a real CPU tile, or their lazy JIT can hold up the final export.
        if let Some((_, warmed_guides)) = warm
            .iter_mut()
            .find(|(p, _)| p.upgrade().is_some_and(|p| Arc::ptr_eq(&p, &pipeline)))
        {
            *warmed_guides |= guides;
        } else {
            if warm.len() >= 8 {
                warm.remove(0);
            }
            warm.push((Arc::downgrade(&pipeline), guides));
        }
    }
    Ok(())
}

fn worker(
    renderer: Renderer,
    mut slots: Vec<ExportSlot>,
    queue: Arc<TileWorkQueue>,
    stop: Arc<AtomicBool>,
    tx: mpsc::SyncSender<WorkerEvent>,
    budget: Arc<AtomicU64>,
    full: [u32; 2],
    cpu: bool,
    profile: Arc<crate::execution_profile::ExecutionProfile>,
) {
    if !cpu {
        prioritize_gpu_submission();
    }
    let run = || -> Result<()> {
        let _permit = if cpu {
            let Some(permit) = cpu_permit(&stop) else {
                return Ok(());
            };
            Some(permit)
        } else {
            None
        };
        if cpu {
            for slot in &mut slots {
                warm_cpu(slot, full, &stop)?;
            }
        }
        for slot in &mut slots {
            slot.set_execution_profile(profile.clone());
        }
        // Readback slots share one device and integrator. Reuse the measured
        // plan immediately, rather than cold-ramping each slot independently.
        let mut dispatch_plan = None;
        loop {
            if stop.load(Ordering::Acquire) {
                return Ok(());
            }
            let mut claimed = false;
            for slot in &mut slots {
                if let Some(result) = slot.finish(false)? {
                    if tx.send(WorkerEvent::Tile(Box::new(result), cpu)).is_err() {
                        return Ok(());
                    }
                }
                if !slot.pending() {
                    let tile = if cpu {
                        queue.claim(true)
                    } else {
                        queue.claim_gpu()
                    };
                    if let Some(tile) = tile {
                        claimed = true;
                        slot.set_budget(budget.load(Ordering::Acquire) as u32);
                        if let Some(plan) = dispatch_plan {
                            slot.set_dispatch_plan(plan);
                        }
                        if !slot.render(tile, full, &stop)? {
                            return Ok(());
                        }
                        dispatch_plan = Some(slot.dispatch_plan());
                        let (samples, elapsed) = slot.cost();
                        queue.observe(cpu, u64::from(tile.w) * u64::from(tile.h), samples, elapsed);
                    }
                }
            }
            if !claimed {
                let mut pending = false;
                for slot in &mut slots {
                    if slot.pending() {
                        pending = true;
                        if let Some(result) = slot.finish(true)? {
                            if tx.send(WorkerEvent::Tile(Box::new(result), cpu)).is_err() {
                                return Ok(());
                            }
                        }
                    }
                }
                if !pending {
                    break;
                }
            }
        }
        renderer.context().check()
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            let _ = tx.send(WorkerEvent::Failed(format!(
                "{} export worker: {error:#}",
                if cpu { "CPU" } else { "GPU" }
            )));
        }
        Err(_) => {
            let _ = tx.send(WorkerEvent::Failed("export worker panicked".into()));
        }
    }
    let _ = tx.send(WorkerEvent::Finished);
}

/// Keep the short GPU submission/readback thread runnable when the software
/// driver's pool occupies every logical CPU. This changes only this worker's
/// CPU scheduling priority, for its lifetime; the process and UI are unchanged.
fn prioritize_gpu_submission() -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL,
        };
        // SAFETY: GetCurrentThread supplies the current thread's valid pseudo
        // handle. It is not owned and must not be closed. The priority is a
        // documented non-realtime value. Failure leaves normal scheduling.
        unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL) != 0 }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn blit<T: Copy>(dst: &mut [T], src: &[T], width: u32, t: Tile) {
    for row in 0..t.h as usize {
        let to = (t.y as usize + row) * width as usize + t.x as usize;
        let from = row * t.w as usize;
        dst[to..to + t.w as usize].copy_from_slice(&src[from..from + t.w as usize]);
    }
}

fn commit(dst: &mut RenderOutput, guides: &mut [(HdrImage, HdrImage)], result: &TileResult) {
    let w = dst.color.width;
    let t = result.tile;
    let src = &result.output;
    blit(&mut dst.color.data, &src.color.data, w, t);
    blit(&mut dst.depth, &src.depth, w, t);
    blit(&mut dst.normals, &src.normals, w, t);
    if let (Some(a), Some(b)) = (&mut dst.layers, &src.layers) {
        blit(&mut a.surface.data, &b.surface.data, w, t);
        blit(&mut a.volume.data, &b.volume.data, w, t);
        blit(&mut a.volume_variance, &b.volume_variance, w, t);
        if let (Some(a), Some(b)) = (&mut a.components, &b.components) {
            blit(&mut a.direct.data, &b.direct.data, w, t);
            blit(&mut a.direct_variance, &b.direct_variance, w, t);
            blit(&mut a.indirect_variance, &b.indirect_variance, w, t);
        }
    }
    for ((a, n), (b, m)) in guides.iter_mut().zip(&result.guides) {
        blit(&mut a.data, &b.data, w, t);
        blit(&mut n.data, &m.data, w, t);
    }
}

// Checkpoints stream their planes directly to disk, avoiding another full HDR
// allocation. Fixed little-endian layout, exact byte count and checksum.
fn planes(
    output: &RenderOutput,
    guides: &[(HdrImage, HdrImage)],
    mut f: impl FnMut(&[u8]) -> Result<()>,
) -> Result<()> {
    f(bytemuck::cast_slice(&output.color.data))?;
    f(bytemuck::cast_slice(&output.depth))?;
    f(bytemuck::cast_slice(&output.normals))?;
    if let Some(l) = &output.layers {
        f(bytemuck::cast_slice(&l.surface.data))?;
        f(bytemuck::cast_slice(&l.volume.data))?;
        f(bytemuck::cast_slice(&l.volume_variance))?;
        if let Some(c) = &l.components {
            f(bytemuck::cast_slice(&c.direct.data))?;
            f(bytemuck::cast_slice(&c.direct_variance))?;
            f(bytemuck::cast_slice(&c.indirect_variance))?;
        }
    }
    for (a, n) in guides {
        f(bytemuck::cast_slice(&a.data))?;
        f(bytemuck::cast_slice(&n.data))?;
    }
    Ok(())
}
fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for b in bytes {
        *hash = (*hash ^ u64::from(*b)).wrapping_mul(0x100000001b3);
    }
}
fn write_payload(
    dir: &Path,
    output: &RenderOutput,
    guides: &[(HdrImage, HdrImage)],
) -> Result<(u64, u64)> {
    ensure!(
        cfg!(target_endian = "little"),
        "tiled checkpoint encoding requires a little-endian host"
    );
    let mut file = std::io::BufWriter::new(std::fs::File::create(dir.join("export.bin"))?);
    let (mut bytes, mut hash) = (0, 0xcbf29ce484222325);
    planes(output, guides, |plane| {
        file.write_all(plane)?;
        bytes += plane.len() as u64;
        hash_bytes(&mut hash, plane);
        Ok(())
    })?;
    file.flush()?;
    file.get_ref().sync_all()?;
    Ok((bytes, hash))
}
fn read_payload(
    dir: &Path,
    output: &mut RenderOutput,
    guides: &mut [(HdrImage, HdrImage)],
    metadata: &ExportCheckpoint,
) -> Result<()> {
    ensure!(
        cfg!(target_endian = "little"),
        "tiled checkpoint decoding requires a little-endian host"
    );
    let mut expected = 0;
    planes(output, guides, |p| {
        expected += p.len() as u64;
        Ok(())
    })?;
    let file = std::fs::File::open(dir.join("export.bin"))?;
    ensure!(
        expected == metadata.bytes && file.metadata()?.len() == expected,
        "export.bin has an invalid byte count"
    );
    let mut reader = std::io::BufReader::new(file);
    let mut hash = 0xcbf29ce484222325;
    let mut read = |bytes: &mut [u8]| -> Result<()> {
        reader.read_exact(bytes)?;
        hash_bytes(&mut hash, bytes);
        Ok(())
    };
    read(bytemuck::cast_slice_mut(&mut output.color.data))?;
    read(bytemuck::cast_slice_mut(&mut output.depth))?;
    read(bytemuck::cast_slice_mut(&mut output.normals))?;
    if let Some(l) = &mut output.layers {
        read(bytemuck::cast_slice_mut(&mut l.surface.data))?;
        read(bytemuck::cast_slice_mut(&mut l.volume.data))?;
        read(bytemuck::cast_slice_mut(&mut l.volume_variance))?;
        if let Some(c) = &mut l.components {
            read(bytemuck::cast_slice_mut(&mut c.direct.data))?;
            read(bytemuck::cast_slice_mut(&mut c.direct_variance))?;
            read(bytemuck::cast_slice_mut(&mut c.indirect_variance))?;
        }
    }
    for (a, n) in guides {
        read(bytemuck::cast_slice_mut(&mut a.data))?;
        read(bytemuck::cast_slice_mut(&mut n.data))?;
    }
    ensure!(hash == metadata.hash, "export.bin checksum mismatch");
    Ok(())
}

fn atomic_checkpoint(dir: &Path, write: impl FnOnce(&Path) -> Result<()>) -> Result<()> {
    crate::session::check_replaceable(dir)?;
    ensure!(
        !std::fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_symlink()),
        "checkpoint cannot replace a symbolic link"
    );
    let parent = dir
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
    let suffix = format!(".export-{}-{serial}", std::process::id());
    let mut name = dir
        .file_name()
        .context("checkpoint needs a final path component")?
        .to_os_string();
    name.push(&suffix);
    let temp = dir.with_file_name(&name);
    name.push(".old");
    let aside = dir.with_file_name(&name);
    ensure!(!aside.exists(), "checkpoint backup path already exists");
    std::fs::create_dir(&temp)?;
    if let Err(error) = write(&temp) {
        let _ = std::fs::remove_dir_all(&temp);
        return Err(error);
    }
    let had_old = dir.exists();
    if had_old {
        if let Err(error) = std::fs::rename(dir, &aside) {
            let _ = std::fs::remove_dir_all(&temp);
            return Err(error.into());
        }
    }
    if let Err(error) = std::fs::rename(&temp, dir) {
        if had_old {
            let _ = std::fs::rename(&aside, dir);
        }
        let _ = std::fs::remove_dir_all(&temp);
        return Err(error.into());
    }
    if had_old {
        std::fs::remove_dir_all(&aside).context("removing previous export checkpoint")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn gpu_submission_priority_is_local_to_the_worker_thread() {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, GetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL,
        };
        // SAFETY: both pseudo handles refer to the calling live thread.
        let original = unsafe { GetThreadPriority(GetCurrentThread()) };
        std::thread::spawn(|| {
            assert!(
                prioritize_gpu_submission(),
                "Windows refused the worker priority"
            );
            assert_eq!(
                unsafe { GetThreadPriority(GetCurrentThread()) },
                THREAD_PRIORITY_ABOVE_NORMAL
            );
        })
        .join()
        .unwrap();
        assert_eq!(unsafe { GetThreadPriority(GetCurrentThread()) }, original);
    }
    #[test]
    fn resume_refuses_changes_to_the_native_arithmetic_implementation() {
        let producer = ProducerIdentity {
            adapter: "test GPU".into(),
            backend: "Vulkan".into(),
            device_type: "DiscreteGpu".into(),
            vendor: 1,
            device: 2,
            driver: "test".into(),
            driver_info: "1.0".into(),
            module: 17,
            workgroup: [8, 8],
        };
        assert!(verify_producers(&[producer.clone()], &[producer.clone()]).is_ok());
        for field in 0..4 {
            let mut changed = producer.clone();
            match field {
                0 => changed.driver_info = "2.0".into(),
                1 => changed.module += 1,
                2 => changed.workgroup = [16, 4],
                _ => changed.device += 1,
            }
            assert!(verify_producers(&[producer.clone()], &[changed])
                .unwrap_err()
                .to_string()
                .contains("arithmetic producer changed"));
        }
        assert!(verify_producers(&[producer.clone()], &[]).is_err());
        assert!(verify_producers(&[], &[producer]).is_ok()); // legacy state
    }

    fn synthetic() -> TiledExport {
        let cfg = RenderConfig::new(37, 23, 3);
        let grid = TileGrid::new(cfg.width, cfg.height, 8);
        let (tx, rx) = mpsc::sync_channel(3);
        let stopped = Arc::new(AtomicBool::new(false));
        let sent = Arc::new(AtomicU64::new(0));
        let (ready_tx, ready_rx) = mpsc::channel();
        let mut workers = Vec::new();
        // Force non-row-major delivery from two independent producers. HDR
        // values identify every global coordinate, including clipped edges.
        for cpu in [false, true] {
            let tiles: Vec<_> = grid
                .tiles
                .iter()
                .copied()
                .enumerate()
                .rev()
                .filter(|(i, _)| i % 2 == usize::from(cpu))
                .map(|(_, t)| t)
                .collect();
            let (tx, stop, sent, ready) =
                (tx.clone(), stopped.clone(), sent.clone(), ready_tx.clone());
            workers.push(std::thread::spawn(move || {
                for tile in tiles {
                    if stop.load(Ordering::Acquire) {
                        return;
                    }
                    let mut cfg = RenderConfig::new(tile.w, tile.h, 3);
                    cfg.denoise = false;
                    let mut output = blank(&cfg, false, false).unwrap();
                    for y in 0..tile.h {
                        for x in 0..tile.w {
                            output.color.set(
                                x,
                                y,
                                [
                                    (tile.x + x) as f32 + 2.0,
                                    (tile.y + y) as f32 + 3.0,
                                    17.0,
                                    1.0,
                                ],
                            );
                        }
                    }
                    if tx
                        .send(WorkerEvent::Tile(
                            Box::new(TileResult {
                                tile,
                                output,
                                guides: vec![],
                                dispatches: 1,
                                max_dispatch: Duration::from_millis(1),
                            }),
                            cpu,
                        ))
                        .is_err()
                    {
                        return;
                    }
                    if sent.fetch_add(1, Ordering::AcqRel) == 2 {
                        let _ = ready.send(());
                    }
                }
                let _ = tx.send(WorkerEvent::Finished);
            }));
        }
        drop(tx);
        ready_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("producers fill the result channel");
        TiledExport {
            scene: Scene::example(),
            output: blank(&cfg, false, false).unwrap(),
            cfg,
            mode: ExportMode::Tiled,
            grid,
            guides: vec![],
            identity: (0, 0, 0),
            producers: vec![],
            previous_producers: vec![],
            results: Some(rx),
            workers,
            cpu_preparing: None,
            cpu_unavailable: None,
            stopped,
            dispatch_budget: Arc::new(AtomicU64::new(400)),
            started: Instant::now(),
            previous_elapsed: Duration::ZERO,
            dispatches: 0,
            max_dispatch: Duration::ZERO,
            finished_workers: 0,
            cpu_tiles: 0,
            gpu_tiles: 0,
            last_tile: [Duration::ZERO; 2],
            execution_profile: Arc::new(crate::execution_profile::ExecutionProfile::new(
                Instant::now(),
            )),
            device_buffer_bytes: 0,
            failure: None,
        }
    }

    #[test]
    fn concurrent_out_of_order_tiles_cover_every_pixel_once_without_development() {
        let mut s = synthetic();
        while !s.step(Duration::from_millis(20)).unwrap() {}
        assert_eq!(s.cpu_tiles + s.gpu_tiles, s.grid.tiles.len());
        assert!(s.cpu_tiles > 0 && s.gpu_tiles > 0);
        assert_eq!((s.grid.min_done(), s.grid.max_done()), (3, 3));
        for y in 0..s.cfg.height {
            for x in 0..s.cfg.width {
                assert_eq!(
                    s.output.color.get(x, y),
                    [x as f32 + 2.0, y as f32 + 3.0, 17.0, 1.0]
                );
            }
        }
    }

    #[test]
    fn cancellation_unblocks_a_full_result_channel_before_joining() {
        let s = synthetic();
        // At most three queued results. Producers have many more results and
        // cannot finish unless the receiver is drained or closed by Drop.
        let start = Instant::now();
        drop(s);
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn completed_owned_tiles_advance_progress_before_every_pixel_has_a_sample() {
        let mut s = synthetic();
        s.step(Duration::ZERO).unwrap();
        let progress = s.progress();
        assert_eq!(progress.samples_min, 0);
        assert!(progress.pixel_samples_done > 0);
        assert!(progress.fraction_complete() > 0.0 && progress.fraction_complete() < 1.0);
        assert!(progress.estimated_remaining().is_some());
        assert_eq!(
            progress.pixel_samples_done,
            s.grid
                .tiles
                .iter()
                .map(|t| u64::from(t.w) * u64::from(t.h) * u64::from(t.done))
                .sum()
        );
        while !s.step(Duration::from_millis(20)).unwrap() {}
        assert_eq!(s.progress().fraction_complete(), 1.0);
        assert_eq!(s.progress().estimated_remaining(), Some(Duration::ZERO));
    }

    #[test]
    fn cancellation_does_not_wait_for_a_cpu_jit_that_owns_no_tile() {
        let mut s = synthetic();
        s.stop_workers();
        let (release, block) = mpsc::channel();
        let (done, completion) = mpsc::channel();
        s.cpu_preparing = Some(Arc::new(AtomicBool::new(true)));
        s.workers = vec![
            std::thread::spawn(|| {}),
            std::thread::spawn(move || {
                block.recv().unwrap();
                done.send(()).unwrap();
            }),
        ];
        let started = Instant::now();
        s.stop_workers();
        assert!(started.elapsed() < Duration::from_secs(1));
        release.send(()).unwrap();
        completion.recv_timeout(Duration::from_secs(2)).unwrap();
    }

    #[test]
    fn checkpoint_stream_is_exact_and_failed_replacement_keeps_previous_state() {
        let mut s = synthetic();
        while !s.step(Duration::from_millis(20)).unwrap() {}
        let dir = std::env::temp_dir().join(format!(
            "syfrack-export-state-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        s.save(&dir).unwrap();
        let state: SessionState =
            ron::from_str(&std::fs::read_to_string(dir.join("state.ron")).unwrap()).unwrap();
        let metadata = state.export.unwrap();
        assert_eq!(metadata.bytes, 37 * 23 * 36);
        let mut restored = blank(&s.cfg, false, false).unwrap();
        read_payload(&dir, &mut restored, &mut [], &metadata).unwrap();
        assert_eq!(restored.color, s.output.color);
        assert_eq!(restored.depth, s.output.depth);
        assert!(atomic_checkpoint(&dir, |_| anyhow::bail!("simulated write failure")).is_err());
        read_payload(&dir, &mut restored, &mut [], &metadata).unwrap();
        let mut data = std::fs::read(dir.join("export.bin")).unwrap();
        data[3] ^= 4;
        std::fs::write(dir.join("export.bin"), data).unwrap();
        assert!(read_payload(&dir, &mut restored, &mut [], &metadata)
            .unwrap_err()
            .to_string()
            .contains("checksum"));
        std::fs::write(dir.join("unrelated.txt"), "keep").unwrap();
        assert!(s.save(&dir).is_err());
        assert_eq!(
            std::fs::read_to_string(dir.join("unrelated.txt")).unwrap(),
            "keep"
        );
    }
}
