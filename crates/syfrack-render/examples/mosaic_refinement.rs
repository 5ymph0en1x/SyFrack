//! Compare both subdivision strategies on the same input, resolution and budget.
//! Arguments: source.png output-directory [size=768] [regions=4096].
use anyhow::{ensure, Result};
use std::path::PathBuf;
use std::time::Instant;
use syfrack_color::{encode::encode, source::read_srgb_image, write_png};
use syfrack_core::{HdrImage, OutputSpace};
use syfrack_render::mosaic::{
    decompose, decompose_guided_controlled, layout, reconstruct, MosaicConfig, MosaicControl,
    MosaicGuidance,
};

fn save(path: PathBuf, image: &HdrImage) -> Result<()> {
    ensure!(!path.exists(), "output already exists: {}", path.display());
    write_png(
        &path,
        &encode(image, OutputSpace::Srgb, 16, true),
        OutputSpace::Srgb,
        None,
    )
}
fn rmse(source: &HdrImage, output: &HdrImage, face_only: bool) -> f64 {
    let mut error = 0.0;
    let mut count = 0;
    for y in 0..source.height {
        for x in 0..source.width {
            let u = x as f32 / source.width as f32;
            let v = y as f32 / source.height as f32;
            if face_only && !((0.32..0.55).contains(&u) && (0.28..0.57).contains(&v)) {
                continue;
            }
            let a = source.get(x, y);
            let b = output.get(x, y);
            for c in 0..3 {
                error += f64::from(a[c].sqrt() * a[3] - b[c].sqrt() * b[3]).powi(2);
            }
            error += f64::from(a[3] - b[3]).powi(2);
            count += 4;
        }
    }
    (error / count as f64).sqrt()
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    ensure!(args.len() >= 3, "source and output directory required");
    let dir = PathBuf::from(&args[2]);
    std::fs::create_dir_all(&dir)?;
    let size = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(768);
    let regions = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(4096);
    let source = read_srgb_image(&PathBuf::from(&args[1]), size)?;
    let config = MosaicConfig {
        min_size: 4,
        max_regions: regions,
        detail: 0.03,
        ..Default::default()
    };
    let priority: Vec<_> = (0..source.height)
        .flat_map(|y| {
            (0..source.width).map(move |x| {
                let u = x as f32 / source.width as f32;
                let v = y as f32 / source.height as f32;
                // Explicit manual priority on this portrait's eyes, nose and mouth.
                let eyes = ((u - 0.43) / 0.12).powi(2) + ((v - 0.34) / 0.035).powi(2);
                let mouth = ((u - 0.435) / 0.065).powi(2) + ((v - 0.48) / 0.035).powi(2);
                let nose = ((u - 0.455) / 0.04).powi(2) + ((v - 0.41) / 0.055).powi(2);
                (1.0 - eyes.min(mouth).min(nose)).clamp(0.0, 1.0)
            })
        })
        .collect();
    let mut report = format!("{}x{}; min_size=4, max_size=128, max_regions={regions}, detail=0.03\nPerceptual associated RGBA RMSE measured at full analysis resolution.\nFace ROI: x 32–55%, y 28–57% (manual, portrait-specific).\n", source.width, source.height);
    for mode in ["source", "refined", "priority"] {
        let start = Instant::now();
        let recipe = if mode == "source" {
            decompose(&source, config)?
        } else {
            decompose_guided_controlled(
                &source,
                config,
                MosaicGuidance {
                    refinement: 1.0,
                    priority: (mode == "priority").then_some(priority.as_slice()),
                },
                &MosaicControl::default(),
            )?
        };
        let image = reconstruct(&recipe, 1)?;
        let line = format!(
            "{mode}: {} regions, {:.2}s, RMSE {:.6}, face RMSE {:.6}\n",
            recipe.tiles.len(),
            start.elapsed().as_secs_f32(),
            rmse(&source, &image, false),
            rmse(&source, &image, true)
        );
        eprint!("{line}");
        report.push_str(&line);
        save(dir.join(format!("{mode}.png")), &image)?;
        save(
            dir.join(format!("{mode}.layout.png")),
            &layout(&source, &recipe)?,
        )?;
    }
    ensure!(!dir.join("metrics.txt").exists(), "metrics already exist");
    std::fs::write(dir.join("metrics.txt"), report)?;
    Ok(())
}
