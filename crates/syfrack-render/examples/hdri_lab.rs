//! A self-contained linear studio panorama and fractal lighting demo.
use anyhow::Result;
use glam::Vec3;
use std::path::PathBuf;
use std::sync::Arc;
use syfrack_color::{develop_and_encode, write_exr, write_png};
use syfrack_core::{Atmosphere, Coloring, HdrImage, Light, Scene};
use syfrack_render::{GpuContext, RenderConfig, Renderer};

fn main() -> Result<()> {
    let dir = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/hdri-lab"));
    std::fs::create_dir_all(&dir)?;
    let dir = dir.canonicalize()?;
    let mut panorama = HdrImage::new(512, 256);
    let key = Vec3::new(-0.55, 0.65, 0.65).normalize();
    let rim = Vec3::new(0.8, 0.25, -0.3).normalize();
    for y in 0..panorama.height {
        let theta = std::f32::consts::PI * (y as f32 + 0.5) / panorama.height as f32;
        for x in 0..panorama.width {
            let phi = 2.0 * std::f32::consts::PI * (x as f32 + 0.5) / panorama.width as f32;
            let d = Vec3::new(
                theta.sin() * phi.cos(),
                theta.cos(),
                theta.sin() * phi.sin(),
            );
            let ambient = 0.08 + 0.30 * d.y.max(0.0);
            let mut rgb = Vec3::splat(ambient);
            if d.dot(key) > 0.965 {
                rgb += Vec3::new(24.0, 18.0, 12.0);
            }
            if d.dot(rim) > 0.985 {
                rgb += Vec3::new(5.0, 14.0, 28.0);
            }
            panorama.set(x, y, [rgb.x, rgb.y, rgb.z, 1.0]);
        }
    }
    let asset = dir.join("studio.exr");
    write_exr(&asset, &panorama)?;
    let mut scene = Scene::example_for("menger")?;
    if let syfrack_core::CameraMode::Orbit { distance, .. } = &mut scene.camera.mode {
        *distance = 5.0;
    }
    scene.material.albedo = [0.88, 0.65, 0.34];
    scene.material.metallic = 0.8;
    scene.material.roughness = 0.22;
    scene.material.coloring = Coloring::None;
    scene.material.surface = None;
    scene.material.texture = None;
    scene.atmosphere = Atmosphere::none();
    scene.camera.f_number = 0.0;
    scene.lights = vec![Light::Hdri {
        path: asset,
        intensity: 1.0,
        rotation: 0.0,
    }];
    scene.save(dir.join("menger-hdri.ron"))?;
    let context = Arc::new(GpuContext::new()?);
    println!("GPU: {}", context.info.name);
    let renderer = Renderer::new(context)?;
    let mut comparison = HdrImage::new(640, 240);
    for (panel, name, spp) in [(0, "preview", 16), (1, "final", 512)] {
        let mut cfg = RenderConfig::new(320, 240, spp);
        cfg.max_bounces = 4;
        let out = renderer.render(&scene, &cfg, &mut |_| {})?;
        println!(
            "{name}: {spp} spp, {:.2}s (render dispatches)",
            out.elapsed.as_secs_f64()
        );
        write_exr(dir.join(format!("{name}.exr")), &out.color)?;
        write_png(
            dir.join(format!("{name}.png")),
            &develop_and_encode(&out.color, &scene.look, 8),
            scene.look.output,
            Some(&scene.to_ron()),
        )?;
        for y in 0..240 {
            for x in 0..320 {
                comparison.set(x + panel * 320, y, out.color.get(x, y));
            }
        }
    }
    write_png(
        dir.join("comparison.png"),
        &develop_and_encode(&comparison, &scene.look, 8),
        scene.look.output,
        None,
    )?;
    println!(
        "16 spp left / 512 spp right, no denoising: {}",
        dir.join("comparison.png").display()
    );
    Ok(())
}
