//! Reproducible native-GPU acceptance gallery for every natural-field model.
//! cargo run -p syfrack-render --example nature_lab -- OUTPUT_DIRECTORY
use anyhow::{ensure, Result};
use std::{path::PathBuf, sync::Arc, time::Instant};
use syfrack_color::{develop_and_encode, write_exr, write_png, EncodedImage};
use syfrack_core::{scene::phenomena::presets, OutputSpace};
use syfrack_render::{ComputeIntegrator, GpuContext, RenderConfig, Renderer};

fn main() -> Result<()> {
    let output = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .unwrap_or("renders/nature-cosmos-20261019".into()),
    );
    std::fs::create_dir_all(&output)?;
    let scenes = PathBuf::from("scenes/showcase/nature-cosmos");
    std::fs::create_dir_all(&scenes)?;
    let renderer = Renderer::new(Arc::new(GpuContext::new()?))?;
    let (width, height) = (384u32, 288u32);
    let mut gallery = EncodedImage {
        width: width * 4,
        height: height * 3,
        bits: 8,
        data: vec![0; (width * height * 4 * 3 * 3) as usize],
    };
    let mut report=format!("Native SyFrack natural-field acceptance gallery\nAdapter: {:?}\nTile: 64; wavefront; 8 samples (dispersion: 96); {}x{}\n",renderer.context().info,width,height);
    for (index, (name, label)) in presets::PRESETS.iter().enumerate() {
        let start = Instant::now();
        let scene = presets::scene(name)?;
        scene.save(scenes.join(format!("{name}.ron")))?;
        scene.save(output.join(format!("{name}.ron")))?;
        let prepared = renderer.prepare_phenomena(&scene)?;
        if let Some(p) = &prepared {
            for (i, d) in p.diagnostics.iter().enumerate() {
                report+=&format!("{name} snapshot {i}: mass {} -> {}; residual {}; eigenvalue {}; occupied {}; substeps {}\n",d.mass_before,d.mass_after,d.residual,d.eigenvalue,d.occupied,d.substeps);
            }
        }
        let samples = if *name == "dispersion" { 96 } else { 8 };
        let mut cfg = RenderConfig::new(width, height, samples);
        cfg.integrator = ComputeIntegrator::Wavefront;
        cfg.max_bounces = 3;
        cfg.max_steps = 256;
        cfg.tile_size = 64;
        let mut dispatches = 0;
        let mut max_ms = 0;
        let rendered = renderer.render(&scene, &cfg, &mut |p| {
            dispatches = p.dispatches;
            max_ms = p.max_dispatch_ms;
        })?;
        ensure!(rendered.samples_min == samples, "incomplete {name}");
        ensure!(
            rendered
                .color
                .data
                .iter()
                .all(|p| p.iter().all(|v| v.is_finite())),
            "nonfinite {name}"
        );
        let mut look = scene.look.clone();
        look.output = OutputSpace::Srgb;
        let encoded = develop_and_encode(&rendered.color, &look, 8);
        write_png(
            output.join(format!("{name}.png")),
            &encoded,
            OutputSpace::Srgb,
            Some(&scene.to_ron()),
        )?;
        write_exr(output.join(format!("{name}.exr")), &rendered.color)?;
        let channels = if encoded.has_alpha() { 4 } else { 3 };
        let ox = index as u32 % 4 * width;
        let oy = index as u32 / 4 * height;
        for y in 0..height {
            for x in 0..width {
                let source = ((y * width + x) as usize) * channels;
                let dest = (((y + oy) * gallery.width + x + ox) * 3) as usize;
                gallery.data[dest..dest + 3].copy_from_slice(&encoded.data[source..source + 3]);
            }
        }
        let line = format!(
            "{index}: {name} — {label}: {:.2}s, {samples} samples, {} dispatches, max {} ms\n",
            start.elapsed().as_secs_f64(),
            dispatches,
            max_ms
        );
        print!("{line}");
        report += &line;
        std::fs::write(output.join("validation.txt"), &report)?;
    }
    write_png(
        output.join("gallery.png"),
        &gallery,
        OutputSpace::Srgb,
        None,
    )?;
    println!("{}", output.join("gallery.png").display());
    Ok(())
}
