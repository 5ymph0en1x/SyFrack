//! Native backend comparison, using the same scene, integrator and exports.
use anyhow::{bail, Context, Result};
use std::{path::PathBuf, sync::Arc};
use syfrack_color::{develop_and_encode, write_exr, write_png};
use syfrack_core::Scene;
use syfrack_render::{GpuContext, RenderConfig, Renderer};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let backend = match args.get(1).map(String::as_str) {
        Some("dx12") => wgpu::Backends::DX12,
        Some("gl") => wgpu::Backends::GL,
        Some("vulkan") => wgpu::Backends::VULKAN,
        _ => bail!("backend_study <dx12|gl|vulkan> [scene.ron output-stem width height samples]"),
    };
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: backend,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .context("request adapter")?;
    let info = adapter.get_info();
    eprintln!(
        "adapter: {} {:?} {:?}; uniform limit {}",
        info.name,
        info.backend,
        info.device_type,
        adapter.limits().max_uniform_buffer_binding_size
    );
    if info.device_type == wgpu::DeviceType::Cpu {
        bail!("Software adapters are excluded from this study.");
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("SyFrack backend study"),
        required_limits: adapter.limits(),
        ..Default::default()
    }))?;
    let gpu = Arc::new(GpuContext::from_device(device, queue, info));
    gpu.run_probe()?;
    if args.len() < 4 {
        return Ok(());
    }
    let scene = Scene::load(&PathBuf::from(&args[2]))?;
    let out = PathBuf::from(&args[3]);
    for ext in ["png", "exr", "ron"] {
        if out.with_extension(ext).exists() {
            bail!("Output already exists.");
        }
    }
    let mut cfg = RenderConfig::new(
        args.get(4).map(|s| s.parse()).transpose()?.unwrap_or(320),
        args.get(5).map(|s| s.parse()).transpose()?.unwrap_or(180),
        args.get(6).map(|s| s.parse()).transpose()?.unwrap_or(2),
    );
    cfg.max_bounces = 1;
    cfg.tile_size = 8;
    cfg.max_steps = 384;
    cfg.pixel_tolerance = 0.8;
    let renderer = Renderer::new(gpu)?;
    let image = renderer.render(&scene, &cfg, &mut |p| {
        if p.dispatches % 64 == 0 {
            eprintln!(
                "{} dispatches, {:.1}s",
                p.dispatches,
                p.elapsed.as_secs_f32()
            );
        }
    })?;
    let ron = scene.to_ron();
    let filtered = syfrack_render::denoise::denoise(&image.color, &image.depth, &image.normals);
    write_exr(&out.with_extension("exr"), &image.color)?;
    write_png(
        &out.with_extension("png"),
        &develop_and_encode(&filtered, &scene.look, 16),
        scene.look.output,
        Some(&ron),
    )?;
    std::fs::write(out.with_extension("ron"), ron)?;
    eprintln!(
        "done: {}x{} spp {}..{} in {:.1}s",
        cfg.width,
        cfg.height,
        image.samples_min,
        image.samples_max,
        image.elapsed.as_secs_f32()
    );
    Ok(())
}
