//! Run after compilation, without another GPU render, to measure actual control
//! latency. All durations are host wall time, not GPU timestamps.
use anyhow::{Context, Result};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use syfrack_core::{Fractal, Graph, Scene};
use syfrack_render::{control::ControlSession, CompileStatus, GpuContext, Renderer};

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "scenes/orfevrerie.ron".into());
    let scene = Scene::load(&path)?;
    let renderer = Renderer::new(Arc::new(GpuContext::new()?))?;
    println!("adapter: {}", renderer.context().info.name);
    let start = Instant::now();
    let mut control = ControlSession::new(renderer.clone(), &scene, (160, 90))?;
    println!(
        "fixed engine + first allocation: {:.3} ms",
        start.elapsed().as_secs_f64() * 1000.0
    );
    let start = Instant::now();
    control.wait_complete()?;
    println!(
        "geometry image: {:.3} ms, size {:?}, {} quanta",
        start.elapsed().as_secs_f64() * 1000.0,
        control.size(),
        control.dispatches()
    );
    if let Some(path) = std::env::args().nth(2) {
        let (w, h) = control.size();
        let bytes = renderer.context().read_buffer(
            control.accumulation_buffer(),
            u64::from(w) * u64::from(h) * 16,
        )?;
        let pixels = bytes
            .chunks_exact(16)
            .map(bytemuck::pod_read_unaligned)
            .collect();
        let image = syfrack_core::HdrImage::from_data(w, h, pixels);
        let mut look = syfrack_core::clay_scene(&scene).look;
        look.auto_exposure = None;
        look.exposure_ev = 0.0;
        let encoded = syfrack_color::develop_and_encode(&image, &look, 8);
        syfrack_color::write_png(path, &encoded, look.output, None)?;
    }
    let mut times = Vec::new();
    for &(name, _) in syfrack_core::graph::bundled::BUNDLED {
        let mut edit = scene.clone();
        edit.fractal = Fractal::Graph(Graph::bundled(name).context("missing bundled graph")?);
        let start = Instant::now();
        control.set_scene(&edit)?;
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(f64::total_cmp);
    println!(
        "topology updates: median {:.3} ms, max {:.3} ms; native pipelines compiled: {}",
        times[times.len() / 2],
        times[times.len() - 1],
        renderer.cached_pipelines()
    );
    let start = Instant::now();
    control.set_scene(&scene)?;
    control.wait_complete()?;
    println!(
        "return to scene + image: {:.3} ms",
        start.elapsed().as_secs_f64() * 1000.0
    );
    let start = Instant::now();
    let ticket = renderer.request_scene(&scene);
    loop {
        match Renderer::poll(&ticket) {
            CompileStatus::Ready(_) => break,
            CompileStatus::Failed(e) => anyhow::bail!(e),
            CompileStatus::Pending => std::thread::sleep(Duration::from_millis(1)),
        }
    }
    println!(
        "optional native specialization: {:.3} ms",
        start.elapsed().as_secs_f64() * 1000.0
    );
    Ok(())
}
