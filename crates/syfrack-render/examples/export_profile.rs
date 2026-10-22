//! Compare bounded export timing and raw HDR against a saved reference.
//! Usage: export_profile scene.ron reference.exr report.ron [width height spp repeats]
use anyhow::{ensure, Context, Result};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use syfrack_core::Scene;
use syfrack_render::{ExportMode, ExportSession, GpuContext, RenderConfig, Renderer};

#[derive(Serialize)]
struct Run {
    seconds: f64,
    render_seconds: f64,
    dispatches: u32,
    buffer_bytes: u64,
    workers: syfrack_render::hybrid_benchmark::WorkStatistics,
    differing_channels: usize,
    difference: syfrack_render::hybrid_benchmark::ImageDifference,
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    ensure!(
        args.len() >= 4,
        "export_profile scene.ron reference.exr report.ron [width height spp repeats]"
    );
    let scene = Scene::load(&PathBuf::from(&args[1]))?;
    let reference = syfrack_color::read_exr(&args[2])?;
    let parse = |i: usize, fallback: u32| -> Result<u32> {
        args.get(i).map_or(Ok(fallback), |s| {
            s.parse().context("invalid numeric argument")
        })
    };
    let mut cfg = RenderConfig::new(parse(4, 384)?, parse(5, 216)?, parse(6, 8)?);
    cfg.max_bounces = 4;
    cfg.max_steps = 512;
    cfg.tile_size = 256;
    cfg.dispatch_budget_ms = 800;
    cfg.denoise = true;
    let repetitions = parse(7, 3)?;
    ensure!((1..=20).contains(&repetitions), "repeats must be in 1..=20");
    let renderer = Renderer::new(Arc::new(GpuContext::new()?))?;
    let mut runs = Vec::new();
    for i in 0..repetitions {
        let started = Instant::now();
        let mut session = ExportSession::new(renderer.clone(), &scene, &cfg, ExportMode::Tiled)?;
        while !session.step(Duration::from_millis(50))? {}
        session.stop_workers();
        let output = session.readback()?;
        let p = session.progress();
        let ExportSession::Tiled(tiled) = &session else {
            unreachable!()
        };
        let difference = syfrack_render::hybrid_benchmark::compare(&reference, &output.color)?;
        let differing_channels = reference
            .data
            .iter()
            .flatten()
            .zip(output.color.data.iter().flatten())
            .filter(|(a, b)| a.to_bits() != b.to_bits())
            .count();
        println!("run {}: total {:.3}s render {:.3}s dispatches {} buffers {} bytes, different channels {}, HDR RMSE {:.6e}", i+1, started.elapsed().as_secs_f64(), p.elapsed.as_secs_f64(), p.dispatches, tiled.device_buffer_bytes(), differing_channels, difference.rmse);
        runs.push(Run {
            seconds: started.elapsed().as_secs_f64(),
            render_seconds: p.elapsed.as_secs_f64(),
            dispatches: p.dispatches,
            buffer_bytes: tiled.device_buffer_bytes(),
            workers: tiled.work_statistics(),
            differing_channels,
            difference,
        });
        // Preserve the report even on a failed exact-value qualification.
        std::fs::write(
            &args[3],
            ron::ser::to_string_pretty(&(&scene, &cfg, &runs), ron::ser::PrettyConfig::new())?,
        )?;
        ensure!(
            differing_channels == 0,
            "GPU batch change altered the reference HDR values"
        );
    }
    Ok(())
}
