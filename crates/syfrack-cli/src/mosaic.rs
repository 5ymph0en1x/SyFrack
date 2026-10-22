use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{ensure, Context, Result};
use clap::Subcommand;
use syfrack_color::{encode::encode, write_png};
use syfrack_core::{mosaic::Mosaic, HdrImage, OutputSpace};
use syfrack_render::mosaic::{
    decompose_guided_controlled, layout, reconstruct, MosaicConfig, MosaicControl, MosaicGuidance,
};

#[derive(Subcommand)]
pub enum MosaicCommand {
    /// Fit adaptive regions; save PNG, subdivision overlay and independent RON recipe.
    Build {
        /// PNG, JPEG, BMP or TIFF, interpreted as sRGB (EXIF orientation applied).
        image: PathBuf,
        /// Output prefix, e.g. renders/mosaic/portrait.
        #[arg(long)]
        out: PathBuf,
        /// Longest analysis edge; smaller inputs retain their original dimensions.
        #[arg(long, default_value_t = 1024, value_parser = clap::value_parser!(u32).range(8..=2048))]
        size: u32,
        /// Smallest subdivided edge in analysis pixels.
        #[arg(long, default_value_t = 8, value_parser = clap::value_parser!(u32).range(1..=2048))]
        min_size: u32,
        /// Preferred largest edge; the hard region budget takes precedence.
        #[arg(long, default_value_t = 128, value_parser = clap::value_parser!(u32).range(1..=2048))]
        max_size: u32,
        #[arg(long, default_value_t = 2048, value_parser = clap::value_parser!(u32).range(1..=16384))]
        regions: u32,
        /// Perceptual RMS threshold in (0,1]; lower values refine more regions.
        #[arg(long, default_value_t = 0.045)]
        detail: f32,
        /// Reconstruction-driven refinement, 0 (source only) to 1 (full).
        #[arg(long, default_value_t = 1.0)]
        refinement: f32,
        /// Integer enlargement of the reconstruction, up to 16 megapixels.
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=16))]
        scale: u32,
    },
    /// Re-render a saved recipe, without opening the original image.
    Render {
        recipe: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=16))]
        scale: u32,
    },
}

fn load_source(path: &Path, size: u32) -> Result<HdrImage> {
    syfrack_color::source::read_srgb_image(path, size)
}

fn prepare_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).context("create mosaic output directory")?;
    }
    Ok(())
}

fn save_png(path: &Path, image: &HdrImage) -> Result<()> {
    let encoded = encode(image, OutputSpace::Srgb, 16, true);
    write_png(path, &encoded, OutputSpace::Srgb, None)
}

pub fn run(command: MosaicCommand) -> Result<()> {
    let start = Instant::now();
    match command {
        MosaicCommand::Build {
            image,
            out,
            size,
            min_size,
            max_size,
            regions,
            detail,
            refinement,
            scale,
        } => {
            let config = MosaicConfig {
                min_size,
                max_size,
                max_regions: regions as usize,
                detail,
            };
            config.validate()?;
            let source = load_source(&image, size)?;
            ensure!(
                u64::from(source.width) * u64::from(source.height) * u64::from(scale).pow(2)
                    <= syfrack_core::mosaic::MAX_MOSAIC_PIXELS,
                "scaled output exceeds 16 megapixels"
            );
            let recipe_path = out.with_extension("mosaic.ron");
            let png_path = out.with_extension("png");
            let layout_path = out.with_extension("layout.png");
            let source_path = out.with_extension("source.png");
            for output in [&recipe_path, &png_path, &layout_path, &source_path] {
                ensure!(
                    !output.exists(),
                    "output already exists: {}",
                    output.display()
                );
            }
            eprintln!(
                "Mosaic: {}x{}, adaptive subdivision and orbit fitting…",
                source.width, source.height
            );
            let mosaic = decompose_guided_controlled(
                &source,
                config,
                MosaicGuidance {
                    refinement,
                    priority: None,
                },
                &MosaicControl::default(),
            )?;
            eprintln!(
                "{} regions; reconstructing at {}x{}…",
                mosaic.tiles.len(),
                source.width * scale,
                source.height * scale
            );
            let output = reconstruct(&mosaic, scale)?;
            let overlay = layout(&source, &mosaic)?;
            let ron = ron::ser::to_string_pretty(&mosaic, ron::ser::PrettyConfig::default())?;
            prepare_parent(&png_path)?;
            save_png(&png_path, &output)?;
            save_png(&layout_path, &overlay)?;
            save_png(&source_path, &source)?;
            std::fs::write(&recipe_path, ron).context("write mosaic recipe")?;
            println!(
                "{} regions in {:.2}s\n{}\n{}\n{}\n{}",
                mosaic.tiles.len(),
                start.elapsed().as_secs_f64(),
                png_path.display(),
                layout_path.display(),
                source_path.display(),
                recipe_path.display()
            );
        }
        MosaicCommand::Render { recipe, out, scale } => {
            ensure!(!out.exists(), "output already exists: {}", out.display());
            let metadata = std::fs::metadata(&recipe).context("open mosaic recipe")?;
            ensure!(
                metadata.len() <= 64 * 1024 * 1024,
                "mosaic recipe exceeds 64 MiB"
            );
            let text = std::fs::read_to_string(&recipe).context("read mosaic recipe")?;
            let mosaic: Mosaic = ron::from_str(&text).context("parse mosaic recipe")?;
            let output = reconstruct(&mosaic, scale)?;
            prepare_parent(&out)?;
            save_png(&out, &output)?;
            println!(
                "{} regions at {}x{} in {:.2}s\n{}",
                mosaic.tiles.len(),
                output.width,
                output.height,
                start.elapsed().as_secs_f64(),
                out.display()
            );
        }
    }
    Ok(())
}
