//! Run separate processes with SYFRACK_WORKGROUP=8x8, 16x4, 32x2.
//! Odd dimensions exercise the shader's edge guards; compare the pixel hashes.
use std::{path::Path, sync::Arc};
use syfrack_core::Scene;
use syfrack_render::{GpuContext, RenderConfig, Renderer};

fn main() -> anyhow::Result<()> {
    let mut paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        paths.push("scenes/environments/nef-des-marees.ron".into());
    }
    let ctx = Arc::new(GpuContext::new()?);
    println!(
        "adapter={} backend={:?} workgroup={}",
        ctx.info.name,
        ctx.info.backend,
        std::env::var("SYFRACK_WORKGROUP").unwrap_or_else(|_| "8x8".into())
    );
    let renderer = Renderer::new(ctx)?;
    let mut cfg = RenderConfig::new(321, 181, 4);
    cfg.tile_size = 32;
    cfg.max_bounces = std::env::var("SYFRACK_BOUNCES").map_or(Ok(3), |v| v.parse())?;
    cfg.max_steps = 384;
    cfg.pixel_tolerance = 0.5;
    println!(
        "width=321 height=181 samples=4 bounces={} steps=384 pixel_tolerance=0.5",
        cfg.max_bounces
    );
    for path in paths {
        let scene = Scene::load(Path::new(&path))?;
        println!("scene={path}");
        let prepare = std::time::Instant::now();
        renderer.pipeline_for_scene(&scene)?;
        println!("prepare_ms={:.3}", prepare.elapsed().as_secs_f64() * 1000.0);
        for trial in 0..3 {
            let result = renderer.render(&scene, &cfg, &mut |_| {})?;
            assert_eq!((result.samples_min, result.samples_max), (4, 4));
            assert!(result.color.data.iter().flatten().all(|x| x.is_finite()));
            let hash = result
                .color
                .data
                .iter()
                .flatten()
                .fold(0xcbf29ce484222325u64, |h, v| {
                    (h ^ v.to_bits() as u64).wrapping_mul(0x100000001b3)
                });
            println!(
                "trial={trial} ms={:.3} hash={hash:016x}",
                result.elapsed.as_secs_f64() * 1000.0
            );
        }
    }
    Ok(())
}
