//! Separate shader preparation, tracing, readback, denoising and image export.
//! Usage: render_profile scene.ron output-stem [width height spp bounces steps tolerance tile trials]
use anyhow::{bail, Context, Result};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use syfrack_color::{develop_and_encode, write_exr, write_png};
use syfrack_core::Scene;
use syfrack_render::{GpuContext, RenderConfig, RenderSession, Renderer};

fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    })
}

fn arg<T: std::str::FromStr>(args: &[String], i: usize, default: T) -> Result<T> {
    args.get(i).map_or(Ok(default), |s| {
        s.parse()
            .map_err(|_| anyhow::anyhow!("invalid argument {i}: {s}"))
    })
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() < 3 {
        bail!("render_profile scene.ron output-stem [width height spp bounces steps tolerance tile trials]");
    }
    let scene = Scene::load(&PathBuf::from(&args[1]))?;
    let out = PathBuf::from(&args[2]);
    for ext in ["png", "exr", "rgba.bin", "depth.bin", "normals.bin", "ron"] {
        if out.with_extension(ext).exists() {
            bail!("output exists: {}", out.with_extension(ext).display());
        }
    }
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut cfg = RenderConfig::new(arg(&args, 3, 321)?, arg(&args, 4, 201)?, arg(&args, 5, 4)?);
    cfg.max_bounces = arg(&args, 6, 3)?;
    cfg.max_steps = arg(&args, 7, 384)?;
    cfg.pixel_tolerance = arg(&args, 8, 0.5)?;
    cfg.tile_size = arg(&args, 9, 32)?;
    let trials: u32 = arg(&args, 10, 3)?;
    if trials == 0 {
        bail!("trials must be at least 1");
    }
    let gpu = Arc::new(GpuContext::new()?);
    cfg.validate(&gpu.device.limits())?;
    println!("adapter={} backend={:?}", gpu.info.name, gpu.info.backend);
    println!(
        "width={} height={} samples={} bounces={} steps={} tolerance={} tile={}",
        cfg.width,
        cfg.height,
        cfg.samples_per_pixel,
        cfg.max_bounces,
        cfg.max_steps,
        cfg.pixel_tolerance,
        cfg.tile_size
    );
    let renderer = Renderer::new(gpu)?;
    let t = Instant::now();
    let pipeline = renderer.pipeline_for_scene(&scene)?;
    println!(
        "prepare_ms={:.3} shader={:016x}",
        t.elapsed().as_secs_f64() * 1000.0,
        pipeline.shader_fingerprint
    );
    let mut last = None;
    for trial in 0..trials {
        let t = Instant::now();
        let mut session = RenderSession::new(renderer.clone(), &scene, &cfg)?;
        let setup = t.elapsed();
        while !session.step(Duration::from_millis(250))? {}
        let p = session.progress();
        let t = Instant::now();
        let image = session.readback()?;
        let readback = t.elapsed();
        assert_eq!(
            (image.samples_min, image.samples_max),
            (cfg.samples_per_pixel, cfg.samples_per_pixel)
        );
        assert!(image.color.data.iter().flatten().all(|v| v.is_finite()));
        println!("trial={} setup_ms={:.3} render_ms={:.3} readback_ms={:.3} dispatches={} max_dispatch_ms={} final_tiles={} color={:016x} depth={:016x} normals={:016x}",
            trial, setup.as_secs_f64()*1000.0, image.elapsed.as_secs_f64()*1000.0,
            readback.as_secs_f64()*1000.0, p.dispatches, p.max_dispatch_ms, p.tiles,
            hash(bytemuck::cast_slice(&image.color.data)), hash(bytemuck::cast_slice(&image.depth)),
            hash(bytemuck::cast_slice(&image.normals)));
        if let Some(previous) = &last {
            let previous: &syfrack_render::RenderOutput = previous;
            assert_eq!(
                previous.color.data, image.color.data,
                "non-deterministic radiance"
            );
            assert_eq!(previous.depth, image.depth, "non-deterministic depth");
            assert_eq!(previous.normals, image.normals, "non-deterministic normals");
        }
        last = Some(image);
    }
    let image = last.context("no render output")?;
    let t = Instant::now();
    let filtered = syfrack_render::denoise::denoise(&image.color, &image.depth, &image.normals);
    println!("denoise_ms={:.3}", t.elapsed().as_secs_f64() * 1000.0);
    let t = Instant::now();
    let encoded = develop_and_encode(&filtered, &scene.look, 16);
    println!("develop_ms={:.3}", t.elapsed().as_secs_f64() * 1000.0);
    let t = Instant::now();
    let ron = scene.to_ron();
    write_exr(&out.with_extension("exr"), &image.color)?;
    write_png(
        &out.with_extension("png"),
        &encoded,
        scene.look.output,
        Some(&ron),
    )?;
    std::fs::write(out.with_extension("ron"), ron)?;
    std::fs::write(
        out.with_extension("rgba.bin"),
        bytemuck::cast_slice(&image.color.data),
    )?;
    std::fs::write(
        out.with_extension("depth.bin"),
        bytemuck::cast_slice(&image.depth),
    )?;
    std::fs::write(
        out.with_extension("normals.bin"),
        bytemuck::cast_slice(&image.normals),
    )?;
    println!("export_ms={:.3}", t.elapsed().as_secs_f64() * 1000.0);
    Ok(())
}
