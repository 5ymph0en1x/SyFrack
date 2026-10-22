//! Qualify an assembled hybrid image at a production sample count without
//! spending another full-frame render on the CPU-only diagnostic baseline.
//! Usage: hybrid_quality scene.ron report.ron [width height samples]
use anyhow::{ensure, Result};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use syfrack_core::{HdrImage, Scene};
use syfrack_render::hybrid_benchmark::{ImageDifference, ImageTolerance, Measurement};
use syfrack_render::{ExportMode, ExportSession, GpuContext, RenderConfig, Renderer};

#[derive(Serialize)]
struct Qualification {
    version: u32,
    scene: Scene,
    config: RenderConfig,
    warm_seconds: f64,
    tolerance: ImageTolerance,
    gpu: Measurement,
    hybrid: Measurement,
    difference: ImageDifference,
}

fn run(
    renderer: &Renderer,
    scene: &Scene,
    cfg: &RenderConfig,
    mode: ExportMode,
) -> Result<(HdrImage, Measurement)> {
    let started = Instant::now();
    let mut session = ExportSession::new(renderer.clone(), scene, cfg, mode)?;
    while !session.step(Duration::from_millis(50))? {}
    session.stop_workers();
    let image = session.readback()?.color;
    let ExportSession::Tiled(tiled) = &session else {
        unreachable!()
    };
    let seconds = started.elapsed().as_secs_f64();
    Ok((
        image,
        Measurement {
            repetition: 0,
            backend: mode,
            seconds,
            pixel_samples_per_second: cfg.width as f64
                * cfg.height as f64
                * cfg.samples_per_pixel as f64
                / seconds,
            workers: tiled.work_statistics(),
        },
    ))
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    ensure!(
        args.len() >= 3,
        "hybrid_quality scene.ron report.ron [width height samples]"
    );
    let scene = Scene::load(&PathBuf::from(&args[1]))?;
    let parse = |i: usize, fallback: u32| -> Result<u32> {
        Ok(args
            .get(i)
            .map(|s| s.parse())
            .transpose()?
            .unwrap_or(fallback))
    };
    let mut cfg = RenderConfig::new(parse(3, 192)?, parse(4, 108)?, parse(5, 128)?);
    cfg.max_bounces = 4;
    cfg.max_steps = 512;
    cfg.tile_size = 32;
    cfg.dispatch_budget_ms = 800;
    let renderer = Renderer::new(Arc::new(GpuContext::new()?))?;
    ensure!(
        renderer.context().info.device_type != wgpu::DeviceType::Cpu,
        "hardware GPU required"
    );
    let mut warm = cfg.clone();
    warm.width = 16;
    warm.height = 16;
    warm.samples_per_pixel = 1;
    let started = Instant::now();
    eprintln!("Warm-up GPU, then CPU (excluded from measurement)...");
    run(&renderer, &scene, &warm, ExportMode::Tiled)?;
    run(&renderer, &scene, &warm, ExportMode::Cpu)?;
    let warm_seconds = started.elapsed().as_secs_f64();
    eprintln!("Warm-up complete in {warm_seconds:.1}s. GPU reference...");
    let (gpu_image, gpu) = run(&renderer, &scene, &cfg, ExportMode::Tiled)?;
    eprintln!("GPU {:.3}s. Concurrent CPU + GPU...", gpu.seconds);
    let (hybrid_image, hybrid) = run(&renderer, &scene, &cfg, ExportMode::Hybrid)?;
    let difference = syfrack_render::hybrid_benchmark::compare(&gpu_image, &hybrid_image)?;
    let tolerance = ImageTolerance::default();
    println!("GPU {:.3}s, hybrid {:.3}s, speedup {:.3}x, CPU {} tiles, GPU {} tiles, overlap {:.1}ms, relative HDR RMSE {:.4}% (max absolute {})",
        gpu.seconds, hybrid.seconds, gpu.seconds/hybrid.seconds, hybrid.workers.cpu_tiles, hybrid.workers.gpu_tiles, hybrid.workers.dispatch_timing.overlap_wall_ms, difference.relative_rmse*100.0, difference.max_absolute_error);
    let report = Qualification {
        version: 1,
        scene,
        config: cfg,
        warm_seconds,
        tolerance,
        gpu,
        hybrid,
        difference,
    };
    std::fs::write(
        &args[2],
        ron::ser::to_string_pretty(&report, ron::ser::PrettyConfig::new())?,
    )?;
    ensure!(
        report.hybrid.workers.cpu_tiles > 0
            && report.hybrid.workers.gpu_tiles > 0
            && report.hybrid.workers.dispatch_timing.overlap_wall_ms > 0.0,
        "no concurrent contribution"
    );
    report.difference.require(tolerance)
}
