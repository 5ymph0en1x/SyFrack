//! Re-export an actual SyFrack HDR image with the optional CPU denoiser.
use anyhow::{Context, Result};
use std::path::Path;
use syfrack_color::{develop_and_encode, read_exr, write_png};
use syfrack_core::Scene;
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        args.len() == 3,
        "usage: denoise_export input.exr scene.ron output.png"
    );
    let image = read_exr(Path::new(&args[0]))?;
    let scene = Scene::load(Path::new(&args[1]))?;
    let started = std::time::Instant::now();
    let filtered = syfrack_render::oidn::filter(
        &image,
        &syfrack_render::oidn::runtime().context("optional OIDN runtime not installed")?,
    )?;
    let encoded = develop_and_encode(&filtered, &scene.look, 16);
    write_png(
        Path::new(&args[2]),
        &encoded,
        scene.look.output,
        Some(&scene.to_ron()),
    )?;
    println!(
        "OIDN CPU: {}x{} in {:.3}s; wrote {}",
        image.width,
        image.height,
        started.elapsed().as_secs_f64(),
        args[2]
    );
    Ok(())
}
