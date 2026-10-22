use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use super::priority::{self, BrushStroke};
use anyhow::{ensure, Context, Result};
use egui::{Color32, ColorImage};
use serde::{Deserialize, Serialize};
use syfrack_color::{
    encode::encode,
    source::{adjust_image, read_srgb_image, ImageAdjustments},
    write_png,
};
use syfrack_core::{
    mosaic::{Mosaic, Region},
    HdrImage, OutputSpace,
};
use syfrack_render::mosaic::{
    decompose_guided_controlled, reconstruct_controlled, subdivide_guided_controlled, MosaicConfig,
    MosaicControl, MosaicGuidance,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub size: u32,
    pub config: MosaicConfig,
    pub image: ImageAdjustments,
    pub refinement: f32,
    pub priority: Vec<BrushStroke>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            size: 768,
            config: MosaicConfig {
                max_regions: 4096,
                detail: 0.05,
                ..Default::default()
            },
            image: ImageAdjustments::default(),
            refinement: 1.0,
            priority: vec![],
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        self.config.validate()?;
        ensure!(
            self.refinement.is_finite() && (0.0..=1.0).contains(&self.refinement),
            "invalid mosaic refinement"
        );
        priority::validate(&self.priority)?;
        ensure!(
            (64..=2048).contains(&self.size),
            "invalid mosaic analysis size"
        );
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Project {
    version: u32,
    source: PathBuf,
    settings: Settings,
}

pub struct Imported {
    pub raw: Arc<HdrImage>,
    pub path: PathBuf,
    pub settings: Settings,
    pub image: ColorImage,
}

pub fn import(path: &Path) -> Result<Imported> {
    let (source, settings) = if path.file_name().is_some_and(|n| {
        n.to_string_lossy()
            .to_lowercase()
            .ends_with(".mosaic-project.ron")
    }) {
        ensure!(
            std::fs::metadata(path)?.len() <= 2 * 1024 * 1024,
            "mosaic project is too large"
        );
        let project: Project =
            ron::from_str(&std::fs::read_to_string(path)?).context("read mosaic project")?;
        ensure!(project.version == 1, "unsupported mosaic project version");
        project.settings.validate()?;
        (
            path.parent().unwrap_or(Path::new(".")).join(project.source),
            project.settings,
        )
    } else {
        (path.to_path_buf(), Settings::default())
    };
    let raw = Arc::new(read_srgb_image(&source, 2048)?);
    // Validate all crop/colour values before adopting an imported project.
    adjust_image(&raw, &settings.image, 64)?;
    let image = colour_image(&raw);
    Ok(Imported {
        raw,
        path: source,
        settings,
        image,
    })
}

pub struct Preview {
    pub settings: Settings,
    pub source: ColorImage,
    pub layout: ColorImage,
    pub regions: usize,
}

pub fn preview(raw: &HdrImage, settings: Settings, control: &MosaicControl) -> Result<Preview> {
    settings.validate()?;
    let source = adjust_image(raw, &settings.image, settings.size)?;
    let mask = priority::rasterize(
        [raw.width, raw.height],
        &settings.image,
        [source.width, source.height],
        &settings.priority,
    );
    let regions = subdivide_guided_controlled(&source, settings.config, Some(&mask), control)?;
    Ok(Preview {
        source: colour_image(&source),
        layout: region_image(&source, &regions),
        settings,
        regions: regions.len(),
    })
}

pub struct Product {
    pub settings: Settings,
    pub source: Arc<HdrImage>,
    pub recipe: Arc<Mosaic>,
    pub image: Arc<HdrImage>,
    pub seconds: f32,
}

pub struct Built {
    pub product: Arc<Product>,
    pub source: ColorImage,
    pub mosaic: ColorImage,
    pub layout: ColorImage,
}

pub fn build(raw: &HdrImage, settings: Settings, control: &MosaicControl) -> Result<Built> {
    let start = Instant::now();
    settings.validate()?;
    let source = Arc::new(adjust_image(raw, &settings.image, settings.size)?);
    let mask = priority::rasterize(
        [raw.width, raw.height],
        &settings.image,
        [source.width, source.height],
        &settings.priority,
    );
    let recipe = Arc::new(decompose_guided_controlled(
        &source,
        settings.config,
        MosaicGuidance {
            refinement: settings.refinement,
            priority: (!settings.priority.is_empty()).then_some(mask.as_slice()),
        },
        control,
    )?);
    let image = Arc::new(reconstruct_controlled(&recipe, 1, control)?);
    let regions: Vec<_> = recipe.tiles.iter().map(|t| t.region).collect();
    let built = Built {
        source: colour_image(&source),
        mosaic: colour_image(&image),
        layout: region_image(&source, &regions),
        product: Arc::new(Product {
            settings,
            source,
            recipe,
            image,
            seconds: start.elapsed().as_secs_f32(),
        }),
    };
    Ok(built)
}

pub fn colour_image(image: &HdrImage) -> ColorImage {
    let encoded = encode(image, OutputSpace::Srgb, 8, false);
    let bytes: Vec<_> = encoded.data.iter().map(|v| *v as u8).collect();
    let size = [image.width as usize, image.height as usize];
    if encoded.has_alpha() {
        ColorImage::from_rgba_unmultiplied(size, &bytes)
    } else {
        ColorImage::from_rgb(size, &bytes)
    }
}

fn region_image(image: &HdrImage, regions: &[Region]) -> ColorImage {
    let mut result = colour_image(image);
    for r in regions {
        for x in r.x..r.x + r.width {
            result.pixels[(r.y * image.width + x) as usize] = Color32::from_rgb(255, 145, 40);
        }
        for y in r.y..r.y + r.height {
            result.pixels[(y * image.width + r.x) as usize] = Color32::from_rgb(255, 145, 40);
        }
    }
    result
}

struct Files {
    paths: Vec<PathBuf>,
    committed: bool,
}
impl Drop for Files {
    fn drop(&mut self) {
        if !self.committed {
            for path in &self.paths {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}
fn save_png(path: &Path, image: &HdrImage) -> Result<()> {
    write_png(
        path,
        &encode(image, OutputSpace::Srgb, 16, true),
        OutputSpace::Srgb,
        None,
    )
}

pub fn export(
    product: &Product,
    raw: &HdrImage,
    directory: &Path,
    name: &str,
    scale: u32,
    control: &MosaicControl,
) -> Result<PathBuf> {
    ensure!(
        !name.trim().is_empty()
            && name == name.trim()
            && name != "."
            && name != ".."
            && !name.ends_with('.')
            && !name
                .chars()
                .any(|c| c.is_control() || "/\\:*?\"<>|".contains(c)),
        "invalid mosaic output name"
    );
    ensure!(!control.is_cancelled(), "mosaic export cancelled");
    std::fs::create_dir_all(directory)?;
    let paths: Vec<_> = [
        "png",
        "layout.png",
        "source.png",
        "original.png",
        "mosaic.ron",
        "mosaic-project.ron",
    ]
    .map(|extension| directory.join(format!("{name}.{extension}")))
    .into();
    // Reserve every path before writing payloads, and remove only files created
    // by this job if any part fails. Existing user files are never replaced.
    let mut files = Files {
        paths: vec![],
        committed: false,
    };
    for path in &paths {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .with_context(|| {
                format!(
                    "output already exists or cannot be written: {}",
                    path.display()
                )
            })?;
        files.paths.push(path.clone());
    }
    let output = if scale == 1 {
        product.image.clone()
    } else {
        Arc::new(reconstruct_controlled(&product.recipe, scale, control)?)
    };
    save_png(&paths[0], &output)?;
    ensure!(!control.is_cancelled(), "mosaic export cancelled");
    let layout = syfrack_render::mosaic::layout(&product.source, &product.recipe)?;
    save_png(&paths[1], &layout)?;
    save_png(&paths[2], &product.source)?;
    ensure!(!control.is_cancelled(), "mosaic export cancelled");
    save_png(&paths[3], raw)?;
    std::fs::write(
        &paths[4],
        ron::ser::to_string_pretty(&*product.recipe, ron::ser::PrettyConfig::default())?,
    )?;
    let project = Project {
        version: 1,
        source: PathBuf::from(format!("{name}.original.png")),
        settings: product.settings.clone(),
    };
    std::fs::write(
        &paths[5],
        ron::ser::to_string_pretty(&project, ron::ser::PrettyConfig::default())?,
    )?;
    ensure!(!control.is_cancelled(), "mosaic export cancelled");
    files.committed = true;
    Ok(paths[0].clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exported_project_reopens_with_adjustments_and_refuses_collisions() {
        let raw = HdrImage::from_data(4, 3, vec![[0.2, 0.5, 0.7, 1.0]; 12]);
        let settings = Settings {
            refinement: 0.6,
            priority: vec![BrushStroke {
                points: vec![[0.5, 0.4], [0.6, 0.7]],
                radius: 0.1,
                erase: false,
            }],
            image: ImageAdjustments {
                exposure: 0.5,
                rotation: 1,
                ..Default::default()
            },
            ..Default::default()
        };
        let built = build(&raw, settings.clone(), &MosaicControl::default()).unwrap();
        let dir =
            std::env::temp_dir().join(format!("syfrack-mosaic-project-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let name = format!(
            "project-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let saved = export(
            &built.product,
            &raw,
            &dir,
            &name,
            2,
            &MosaicControl::default(),
        )
        .unwrap();
        let original_png = std::fs::read(&saved).unwrap();
        let imported = import(&dir.join(format!("{name}.mosaic-project.ron"))).unwrap();
        assert_eq!(imported.settings, settings);
        assert_eq!((imported.raw.width, imported.raw.height), (4, 3));
        assert!(export(
            &built.product,
            &raw,
            &dir,
            &name,
            1,
            &MosaicControl::default()
        )
        .is_err());
        assert_eq!(std::fs::read(saved).unwrap(), original_png);
        for extension in [
            "png",
            "layout.png",
            "source.png",
            "original.png",
            "mosaic.ron",
            "mosaic-project.ron",
        ] {
            std::fs::remove_file(dir.join(format!("{name}.{extension}"))).unwrap();
        }
        let _ = std::fs::remove_dir(dir);
    }

    #[test]
    fn older_settings_default_to_refinement_and_invalid_brushes_are_rejected() {
        let settings: Settings = ron::from_str("(size: 512, config: (min_size: 8, max_size: 128, max_regions: 512, detail: 0.05), image: ())").unwrap();
        assert_eq!(settings.refinement, 1.0);
        assert!(settings.priority.is_empty());
        let mut settings = settings;
        settings.priority.push(BrushStroke {
            points: vec![[f32::NAN, 0.5]],
            radius: 0.1,
            erase: false,
        });
        assert!(settings.validate().is_err());
    }
}
