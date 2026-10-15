//! Render an aesthetic study folder in one GPU process, sharing the compiled pipeline.
use anyhow::{bail, Result};
use std::{path::PathBuf, sync::Arc, time::Duration};
use syfrack_color::{develop_and_encode, write_exr, write_png};
use syfrack_core::Scene;
use syfrack_render::{GpuContext, RenderConfig, RenderSession, Renderer};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() < 3 {
        bail!("complex_lab SCENE_FOLDER OUTPUT_FOLDER [width height samples]");
    }
    let source = PathBuf::from(&args[1]);
    let output = PathBuf::from(&args[2]);
    let width = args.get(3).map_or(Ok(800), |s| s.parse::<u32>())?;
    let height = args.get(4).map_or(Ok(500), |s| s.parse::<u32>())?;
    let samples = args.get(5).map_or(Ok(8), |s| s.parse::<u32>())?;
    let mut files: Vec<_> = std::fs::read_dir(&source)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "ron"))
        .collect();
    files.sort();
    if files.is_empty() {
        bail!("No RON studies in {}", source.display());
    }
    std::fs::create_dir_all(&output)?;
    let gpu = Arc::new(GpuContext::new()?);
    println!("{} {:?}", gpu.info.name, gpu.info.backend);
    let renderer = Renderer::new(gpu)?;
    let mut cfg = RenderConfig::new(width, height, samples);
    cfg.max_bounces = 1;
    cfg.max_steps = 128;
    cfg.pixel_tolerance = 0.35;
    cfg.tile_size = 64;
    for file in files {
        let scene = Scene::load(&file)?;
        let stem = output.join(file.file_stem().unwrap());
        if stem.with_extension("png").exists() {
            bail!("Output exists: {}", stem.display());
        }
        let mut session = RenderSession::new(renderer.clone(), &scene, &cfg)?;
        while !session.step(Duration::from_millis(250))? {}
        let image = session.readback()?;
        if image.samples_min != samples || image.samples_max != samples {
            bail!("Incomplete image");
        }
        if !image.color.data.iter().flatten().all(|v| v.is_finite()) {
            bail!("Non-finite radiance");
        }
        let filtered = if scene.dimension == syfrack_core::Dimension::TwoD {
            image.color.clone()
        } else {
            syfrack_render::denoise::denoise(&image.color, &image.depth, &image.normals)
        };
        let encoded = develop_and_encode(&filtered, &scene.look, 16);
        let ron = scene.to_ron();
        write_png(
            &stem.with_extension("png"),
            &encoded,
            scene.look.output,
            Some(&ron),
        )?;
        write_exr(&stem.with_extension("exr"), &image.color)?;
        std::fs::write(stem.with_extension("ron"), ron)?;
        println!(
            "{}: {}x{} {} spp {:.2}s",
            stem.display(),
            width,
            height,
            samples,
            image.elapsed.as_secs_f64()
        );
    }
    Ok(())
}
