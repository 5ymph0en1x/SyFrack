//! GPU-only comparison, interleaved warmed runs, unchanged scene and samples.
//! wavefront_benchmark scene.ron report.ron [width height samples repeats]
use anyhow::{ensure, Result};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use syfrack_core::{HdrImage, Scene};
use syfrack_render::hybrid_benchmark::{compare, ImageDifference, ImageTolerance, Measurement};
use syfrack_render::{ExportMode, ExportSession, GpuContext, RenderConfig, Renderer};

#[derive(Serialize)]
struct Report {
    version: u32,
    scene: Scene,
    config: RenderConfig,
    warm_seconds: f64,
    tolerance: ImageTolerance,
    measurements: Vec<Measurement>,
    device_buffer_bytes: Vec<u64>,
    differences: Vec<ImageDifference>,
    speedup: f64,
}
fn run(
    r: &Renderer,
    s: &Scene,
    c: &RenderConfig,
    mode: ExportMode,
    repetition: u32,
) -> Result<(HdrImage, Measurement, u64)> {
    let start = Instant::now();
    let mut session = ExportSession::new(r.clone(), s, c, mode)?;
    while !session.step(Duration::from_millis(50))? {}
    session.stop_workers();
    let image = session.readback()?.color;
    let ExportSession::Tiled(t) = &session else {
        unreachable!()
    };
    let seconds = start.elapsed().as_secs_f64();
    let m = Measurement {
        repetition,
        backend: mode,
        seconds,
        pixel_samples_per_second: c.width as f64 * c.height as f64 * c.samples_per_pixel as f64
            / seconds,
        workers: t.work_statistics(),
    };
    ensure!(
        m.workers.cpu_tiles == 0,
        "GPU-only benchmark received CPU work"
    );
    Ok((image, m, t.device_buffer_bytes()))
}
fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 0 {
        (v[n / 2 - 1] + v[n / 2]) * 0.5
    } else {
        v[n / 2]
    }
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    ensure!(
        args.len() >= 3,
        "wavefront_benchmark scene.ron report.ron [width height samples repeats]"
    );
    let scene = Scene::load(&PathBuf::from(&args[1]))?;
    let parse = |i: usize, d: u32| -> Result<u32> {
        Ok(args.get(i).map(|s| s.parse()).transpose()?.unwrap_or(d))
    };
    let mut cfg = RenderConfig::new(parse(3, 192)?, parse(4, 108)?, parse(5, 8)?);
    cfg.max_bounces = 4;
    cfg.max_steps = 512;
    cfg.dispatch_budget_ms = 800;
    cfg.tile_size = 256;
    let repeats = parse(6, 3)?;
    ensure!(repeats > 0, "repeats must be positive");
    let renderer = Renderer::new(Arc::new(GpuContext::new()?))?;
    ensure!(
        renderer.context().info.device_type != wgpu::DeviceType::Cpu,
        "hardware GPU required"
    );
    println!(
        "GPU: {} ({:?}, {})",
        renderer.context().info.name,
        renderer.context().info.backend,
        renderer.context().info.driver_info
    );
    let mut warm = cfg.clone();
    warm.width = 16;
    warm.height = 16;
    warm.samples_per_pixel = 1;
    let start = Instant::now();
    for mode in [ExportMode::Tiled, ExportMode::Wavefront] {
        eprintln!("Warm-up {mode:?} (excluded)...");
        run(&renderer, &scene, &warm, mode, 0)?;
    }
    let warm_seconds = start.elapsed().as_secs_f64();
    let (mut measurements, mut bytes, mut differences) = (Vec::new(), Vec::new(), Vec::new());
    for rep in 0..repeats {
        let order = if rep % 2 == 0 {
            [ExportMode::Tiled, ExportMode::Wavefront]
        } else {
            [ExportMode::Wavefront, ExportMode::Tiled]
        };
        let (mut reference, mut wave) = (None, None);
        for mode in order {
            eprintln!("Run {} {mode:?}...", rep + 1);
            let (image, m, b) = run(&renderer, &scene, &cfg, mode, rep)?;
            println!(
                "{mode:?}: {:.4}s, GPU tiles {}, bytes {b}",
                m.seconds, m.workers.gpu_tiles
            );
            if mode == ExportMode::Tiled {
                reference = Some(image);
            } else {
                wave = Some(image);
            }
            measurements.push(m);
            bytes.push(b);
        }
        differences.push(compare(&reference.unwrap(), &wave.unwrap())?);
    }
    let times = |mode| {
        measurements
            .iter()
            .filter(|m| m.backend == mode)
            .map(|m| m.seconds)
            .collect()
    };
    let speedup = median(times(ExportMode::Tiled)) / median(times(ExportMode::Wavefront));
    let report = Report {
        version: 1,
        scene,
        config: cfg,
        warm_seconds,
        tolerance: ImageTolerance::default(),
        measurements,
        device_buffer_bytes: bytes,
        differences,
        speedup,
    };
    std::fs::write(
        &args[2],
        ron::ser::to_string_pretty(&report, ron::ser::PrettyConfig::new())?,
    )?;
    println!("Median speedup: {:.4}x; report {}", report.speedup, args[2]);
    for d in &report.differences {
        println!(
            "HDR relative RMSE {:.6}%, max {}",
            d.relative_rmse * 100.0,
            d.max_absolute_error
        );
        d.require(report.tolerance)?;
    }
    Ok(())
}
