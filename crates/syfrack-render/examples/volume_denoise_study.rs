//! Same accumulated HDR pixels, different reconstruction guides. No tracing.
use anyhow::Result;
use std::{path::Path, sync::Arc};
use syfrack_core::{Atmosphere, HdrImage};
use syfrack_render::{GpuContext, RenderSession, Renderer};

fn optical_depth(a: &Atmosphere, y: f32, dy: f32, distance: f32) -> f32 {
    let split = if dy.abs() < 1e-8 {
        distance
    } else {
        ((a.fog_height - y) / dy).clamp(0.0, distance)
    };
    let sigma = |t: f32| {
        a.fog_density
            * (-(y + dy * t - a.fog_height) * a.fog_falloff)
                .min(0.0)
                .exp()
    };
    let segment = |begin: f32, end: f32| {
        let d = end - begin;
        let k = dy * a.fog_falloff;
        if y + dy * (begin + end) * 0.5 <= a.fog_height || a.fog_falloff == 0.0 {
            a.fog_density * d
        } else if (k * d).abs() < 0.001 {
            sigma(begin) * d * (1.0 - k * d * 0.5 + (k * d).powi(2) / 6.0)
        } else {
            ((sigma(begin) - sigma(end)) / k).max(0.0)
        }
    };
    segment(0.0, split) + segment(split, distance)
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    anyhow::ensure!(args.len() == 2, "state directory, output directory");
    let directory = Path::new(&args[1]);
    std::fs::create_dir_all(directory)?;
    let renderer = Renderer::new(Arc::new(GpuContext::new()?))?;
    let session = RenderSession::resume(renderer, Path::new(&args[0]))?;
    let output = session.readback()?;
    let (albedo, normals) = session.export_guides()?;
    let scene = session.scene();
    let runtime = syfrack_render::oidn::runtime().expect("OIDN installed");
    let save = |name: &str, image: &HdrImage| -> Result<()> {
        syfrack_color::write_exr(&directory.join(format!("{name}.exr")), image)?;
        syfrack_color::write_png(
            &directory.join(format!("{name}.png")),
            &syfrack_color::develop_and_encode(image, &scene.look, 16),
            scene.look.output,
            None,
        )?;
        Ok(())
    };
    syfrack_color::write_exr(&directory.join("albedo.exr"), &albedo)?;
    syfrack_color::write_exr(&directory.join("normals.exr"), &normals)?;
    save(
        "surface-guides",
        &syfrack_render::oidn::filter_guided(&output.color, Some((&albedo, &normals)), &runtime)?,
    )?;
    save(
        "color-only",
        &syfrack_render::oidn::filter(&output.color, &runtime)?,
    )?;
    let mut a = albedo.clone();
    let mut n = normals.clone();
    let camera = scene.camera.basis();
    let (w, h) = (output.color.width, output.color.height);
    for i in 0..a.data.len() {
        let (x, y) = (i as u32 % w, i as u32 / w);
        let dx = ((x as f32 + 0.5) / w as f32 * 2.0 - 1.0) * camera.tan_half_fov;
        let dy =
            (1.0 - (y as f32 + 0.5) / h as f32 * 2.0) * camera.tan_half_fov * h as f32 / w as f32;
        let ray = (camera.forward + camera.right * dx + camera.up * dy).normalize();
        let has_surface = output.depth[i] > 0.0
            && output.normals[i][..3].iter().map(|v| v * v).sum::<f32>() > 0.5;
        let distance = if has_surface { output.depth[i] } else { 2500.0 };
        let t = (-optical_depth(&scene.atmosphere, camera.origin.y, ray.y, distance)).exp();
        for c in 0..3 {
            a.data[i][c] = t * albedo.data[i][c] + (1.0 - t) * scene.atmosphere.fog_color[c];
            n.data[i][c] = t * normals.data[i][c];
        }
    }
    syfrack_color::write_exr(&directory.join("volume-albedo.exr"), &a)?;
    syfrack_color::write_exr(&directory.join("volume-normals.exr"), &n)?;
    save(
        "volume-guides",
        &syfrack_render::oidn::filter_guided(&output.color, Some((&a, &n)), &runtime)?,
    )?;
    println!(
        "Identical HDR, {}..{} spp; reconstruction comparison saved to {}",
        output.samples_min,
        output.samples_max,
        directory.display()
    );
    Ok(())
}
