//! Material-guided export from a SyFrack checkpoint, without adding samples.
use anyhow::Result;
use std::{path::Path, sync::Arc};
use syfrack_color::{develop_and_encode, write_png};
use syfrack_render::{GpuContext, RenderSession, Renderer};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        args.len() == 2,
        "usage: denoise_checkpoint_export state_directory output.png"
    );
    let renderer = Renderer::new(Arc::new(GpuContext::new()?))?;
    let session = RenderSession::resume(renderer, Path::new(&args[0]))?;
    let output = session.readback()?;
    let started = std::time::Instant::now();
    let (albedo, normals) = session.export_guides()?;
    let filtered = syfrack_render::oidn::filter_guided(
        &output.color,
        Some((&albedo, &normals)),
        &syfrack_render::oidn::runtime().ok_or_else(|| anyhow::anyhow!("OIDN not installed"))?,
    )?;
    let scene = session.scene();
    let encoded = develop_and_encode(&filtered, &scene.look, 16);
    write_png(
        Path::new(&args[1]),
        &encoded,
        scene.look.output,
        Some(&scene.to_ron()),
    )?;
    println!(
        "Guided export: {}x{}, {}..{} samples, {:.3}s, {}",
        output.color.width,
        output.color.height,
        output.samples_min,
        output.samples_max,
        started.elapsed().as_secs_f64(),
        args[1]
    );
    Ok(())
}
