//! Native volume studies, reusing one GPU/pipeline/cache across scene variants.
//! cargo run -p syfrack-render --example volume_study -- OUT_DIR SCENE...
use anyhow::{ensure, Result};
use std::{path::PathBuf, sync::Arc, time::Instant};
use syfrack_color::{develop_and_encode, write_exr, write_png};
use syfrack_core::Scene;
use syfrack_render::{GpuContext, RenderConfig, Renderer};
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let output = PathBuf::from(args.next().expect("OUT_DIR SCENE..."));
    std::fs::create_dir_all(&output)?;
    let renderer = Renderer::new(Arc::new(GpuContext::new()?))?;
    let setting = |key: &str, default: u32| -> Result<u32> {
        Ok(std::env::var(key)
            .ok()
            .map(|s| s.parse())
            .transpose()?
            .unwrap_or(default))
    };
    let width = setting("SYFRACK_STUDY_WIDTH", 800)?;
    let height = setting("SYFRACK_STUDY_HEIGHT", 450)?;
    let samples = setting("SYFRACK_STUDY_SAMPLES", 4)?;
    ensure!(
        (1..=8192).contains(&width)
            && (1..=8192).contains(&height)
            && (1..=1024).contains(&samples),
        "invalid study image budget"
    );
    let mut config = RenderConfig::new(width, height, samples);
    config.max_bounces = 0;
    config.max_steps = 64;
    config.tile_size = 128;
    config.pixel_tolerance = 0.15;
    for input in args {
        let path = PathBuf::from(input);
        let scene = Scene::load(&path)?;
        let stem = output.join(path.file_stem().unwrap());
        ensure!(
            !stem.with_extension("png").exists(),
            "output already exists: {}",
            stem.display()
        );
        let start = Instant::now();
        let image = renderer.render(&scene, &config, &mut |_| {})?;
        write_exr(stem.with_extension("exr"), &image.color)?;
        write_png(
            stem.with_extension("png"),
            &develop_and_encode(&image.color, &scene.look, 8),
            scene.look.output,
            None,
        )?;
        scene.save(stem.with_extension("ron"))?;
        println!("{}: {:.2}s", path.display(), start.elapsed().as_secs_f64());
    }
    Ok(())
}
