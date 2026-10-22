//! CPU inverse rendering: adaptive spatial subdivision and a shared orbit atlas.
//! The fitted document contains only fractals and palettes, never source pixels.
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering as AtomicOrdering};
use std::{cmp::Ordering, collections::BinaryHeap};

use anyhow::{ensure, Result};
use glam::Vec2;
use rayon::prelude::*;
use syfrack_core::{
    mosaic::{Mosaic, MosaicTile, Region, MAX_MOSAIC_PIXELS, MAX_MOSAIC_TILES},
    HdrImage, PlanarFamily, PlanarFractal,
};

const GRID: usize = 8;
const SAMPLES: usize = GRID * GRID;

mod refinement;
pub use refinement::{decompose_guided_controlled, MosaicGuidance};

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MosaicConfig {
    pub min_size: u32,
    pub max_size: u32,
    pub max_regions: usize,
    /// RMS variation of premultiplied perceptual RGB/alpha; lower adds detail.
    pub detail: f32,
}

/// Independent control for a background job. No UI or GPU state is retained.
#[derive(Default)]
pub struct MosaicControl {
    cancelled: AtomicBool,
    phase: AtomicUsize,
    completed: AtomicUsize,
    total: AtomicUsize,
}

impl MosaicControl {
    pub fn cancel(&self) {
        self.cancelled.store(true, AtomicOrdering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(AtomicOrdering::Relaxed)
    }
    /// Phase 0: subdivision, 1: fitting, 2: reconstruction, 3: refinement.
    /// Work is phase-local; refinement reports the current leaf budget.
    pub fn progress(&self) -> (usize, usize, usize) {
        (
            self.phase.load(AtomicOrdering::Relaxed),
            self.completed.load(AtomicOrdering::Relaxed),
            self.total.load(AtomicOrdering::Relaxed),
        )
    }
    fn check(&self) -> Result<()> {
        ensure!(!self.is_cancelled(), "mosaic job cancelled");
        Ok(())
    }
    fn begin(&self, phase: usize, total: usize) {
        self.total.store(total, AtomicOrdering::Relaxed);
        self.completed.store(0, AtomicOrdering::Relaxed);
        self.phase.store(phase, AtomicOrdering::Relaxed);
    }
    fn advance(&self, amount: usize) {
        self.completed.fetch_add(amount, AtomicOrdering::Relaxed);
    }
}

impl Default for MosaicConfig {
    fn default() -> Self {
        Self {
            min_size: 8,
            max_size: 128,
            max_regions: 2048,
            detail: 0.045,
        }
    }
}

impl MosaicConfig {
    pub fn validate(self) -> Result<()> {
        ensure!(self.min_size > 0, "minimum region size must be positive");
        ensure!(
            self.max_size >= self.min_size,
            "maximum size is below minimum size"
        );
        ensure!(
            (1..=MAX_MOSAIC_TILES).contains(&self.max_regions),
            "region budget must be 1..={MAX_MOSAIC_TILES}"
        );
        ensure!(
            self.detail.is_finite() && self.detail > 0.0 && self.detail <= 1.0,
            "detail must be finite and in (0, 1]"
        );
        Ok(())
    }
}

fn validate_source(image: &HdrImage) -> Result<()> {
    let pixels = u64::from(image.width) * u64::from(image.height);
    ensure!(
        pixels > 0 && pixels <= MAX_MOSAIC_PIXELS && pixels as usize == image.data.len(),
        "invalid mosaic input dimensions"
    );
    ensure!(
        image
            .data
            .iter()
            .flatten()
            .all(|v| v.is_finite() && *v >= 0.0 && *v <= 1.0),
        "mosaic input must be display-linear RGBA in [0, 1]"
    );
    Ok(())
}

fn associated(p: [f32; 4]) -> [f32; 4] {
    [p[0] * p[3], p[1] * p[3], p[2] * p[3], p[3]]
}

#[derive(Clone, Debug)]
struct Candidate {
    region: Region,
    priority: f64,
    error: f64,
}
impl PartialEq for Candidate {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Candidate {}
impl PartialOrd for Candidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Candidate {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority
            .total_cmp(&other.priority)
            .then_with(|| other.region.y.cmp(&self.region.y))
            .then_with(|| other.region.x.cmp(&self.region.x))
    }
}

fn measure_guided(
    image: &HdrImage,
    region: Region,
    max_size: u32,
    priority: Option<&[f32]>,
) -> Candidate {
    let mut sum = [0.0f64; 4];
    let mut square = [0.0f64; 4];
    let mut importance = 0.0f64;
    for y in region.y..region.y + region.height {
        for x in region.x..region.x + region.width {
            let p = image.get(x, y);
            importance += priority.map_or(0.0, |m| f64::from(m[image.index(x, y)]));
            // Alpha is coverage, not a transfer-encoded colour. Invisible RGB
            // must not consume the adaptive region budget.
            let values = [
                p[0].sqrt() * p[3],
                p[1].sqrt() * p[3],
                p[2].sqrt() * p[3],
                p[3],
            ];
            for c in 0..4 {
                let v = f64::from(values[c]);
                sum[c] += v;
                square[c] += v * v;
            }
        }
    }
    let area = region.area() as f64;
    let variance = (0..4)
        .map(|c| (square[c] / area - (sum[c] / area).powi(2)).max(0.0))
        .sum::<f64>()
        / 4.0
        * (1.0 + 15.0 * importance / area);
    // The size cap is a preference; the total region budget is always hard.
    let oversized = region.width.max(region.height) > max_size;
    Candidate {
        region,
        priority: if oversized {
            1e12 + area
        } else {
            variance * area
        },
        error: variance.sqrt(),
    }
}

fn children(r: Region, min: u32) -> Vec<Region> {
    let split_x = r.width / 2 >= min;
    let split_y = r.height / 2 >= min;
    let widths = if split_x {
        vec![r.width / 2, r.width - r.width / 2]
    } else {
        vec![r.width]
    };
    let heights = if split_y {
        vec![r.height / 2, r.height - r.height / 2]
    } else {
        vec![r.height]
    };
    let mut result = Vec::new();
    let mut y = r.y;
    for height in heights {
        let mut x = r.x;
        for &width in &widths {
            result.push(Region {
                x,
                y,
                width,
                height,
            });
            x += width;
        }
        y += height;
    }
    result
}

/// Quadtree subdivision with binary splits for narrow strips and odd edges.
/// All source pixels contribute to error; thin features cannot fall between probes.
pub fn subdivide(image: &HdrImage, config: MosaicConfig) -> Result<Vec<Region>> {
    subdivide_controlled(image, config, &MosaicControl::default())
}

pub fn subdivide_controlled(
    image: &HdrImage,
    config: MosaicConfig,
    control: &MosaicControl,
) -> Result<Vec<Region>> {
    subdivide_guided_controlled(image, config, None, control)
}

/// Fast, source-based estimate. Guided fitting can refine these boundaries later.
pub fn subdivide_guided_controlled(
    image: &HdrImage,
    config: MosaicConfig,
    priority: Option<&[f32]>,
    control: &MosaicControl,
) -> Result<Vec<Region>> {
    control.check()?;
    control.begin(0, 0);
    config.validate()?;
    validate_source(image)?;
    refinement::validate_priority(image, priority)?;
    let root = Region {
        x: 0,
        y: 0,
        width: image.width,
        height: image.height,
    };
    let mut queue = BinaryHeap::from([measure_guided(image, root, config.max_size, priority)]);
    let mut leaves = Vec::new();
    while let Some(node) = queue.pop() {
        control.check()?;
        let r = node.region;
        let parts = children(r, config.min_size);
        if (node.error <= f64::from(config.detail) && r.width.max(r.height) <= config.max_size)
            || parts.len() == 1
            || leaves.len() + queue.len() + parts.len() > config.max_regions
        {
            leaves.push(r);
        } else {
            for part in parts {
                queue.push(measure_guided(image, part, config.max_size, priority));
            }
        }
    }
    leaves.sort_by_key(|r| (r.y, r.x));
    Ok(leaves)
}

struct Template {
    fractal: PlanarFractal,
    channel: usize,
    range: [f32; 2],
    basis: [[f32; 4]; SAMPLES],
    inverse: [[f32; 4]; 4],
}

fn basis(t: f32) -> [f32; 4] {
    let t = t.clamp(0.0, 1.0) * 3.0;
    let i = (t.floor() as usize).min(2);
    let mut b = [0.0; 4];
    b[i + 1] = t - i as f32;
    b[i] = 1.0 - b[i + 1];
    b
}

// The ridge keeps unused palette stops tied to the region mean, including
// constant orbit channels. The matrix is positive definite for every template.
const RIDGE: f32 = 0.05;
fn inverse_gram(basis: &[[f32; 4]; SAMPLES]) -> [[f32; 4]; 4] {
    let mut matrix = [[0.0f32; 8]; 4];
    for i in 0..4 {
        for j in 0..4 {
            matrix[i][j] =
                basis.iter().map(|b| b[i] * b[j]).sum::<f32>() + if i == j { RIDGE } else { 0.0 };
        }
        matrix[i][i + 4] = 1.0;
    }
    for i in 0..4 {
        let divisor = matrix[i][i];
        for v in &mut matrix[i] {
            *v /= divisor;
        }
        for j in 0..4 {
            if i == j {
                continue;
            }
            let factor = matrix[j][i];
            for k in 0..8 {
                matrix[j][k] -= factor * matrix[i][k];
            }
        }
    }
    std::array::from_fn(|i| std::array::from_fn(|j| matrix[i][j + 4]))
}

fn atlas() -> Vec<Template> {
    use PlanarFamily::*;
    let mut templates = Vec::new();
    for family in [Julia, Mandelbrot, BurningShip, Newton, Nova, Phoenix] {
        for view in 0..3 {
            let mut p = PlanarFractal {
                family,
                iterations: 96,
                ..PlanarFractal::default()
            };
            match family {
                Julia => {
                    p.julia = [-0.8, 0.156];
                    p.zoom = [1.0, 2.5, 6.0][view];
                    p.center = [0.0, 0.1];
                }
                Mandelbrot => {
                    p.center = [-0.7435, 0.1314];
                    p.zoom = [1.0, 12.0, 80.0][view];
                }
                BurningShip => {
                    p.center = [-1.76, -0.035];
                    p.zoom = [1.0, 12.0, 60.0][view];
                }
                Newton => {
                    p.power = [3, 4, 5][view];
                    p.zoom = 1.2;
                }
                Nova => {
                    p.power = 3;
                    p.julia = [-0.16, 0.18];
                    p.zoom = [1.0, 2.4, 5.0][view];
                }
                Phoenix => {
                    p.julia = [0.56667, 0.0];
                    p.zoom = [1.0, 2.8, 6.0][view];
                }
            }
            for rotation in 0..4 {
                p.angle = [0.0, 90.0, 180.0, -90.0][rotation];
                let samples: [[f32; 4]; SAMPLES] = std::array::from_fn(|i| {
                    let u = (i % GRID) as f32 + 0.5;
                    let v = (i / GRID) as f32 + 0.5;
                    p.sample(Vec2::new(
                        3.0 * (u / GRID as f32 - 0.5),
                        3.0 * (v / GRID as f32 - 0.5),
                    ))
                    .traps
                });
                for channel in 0..3 {
                    let lo = samples
                        .iter()
                        .map(|s| s[channel])
                        .fold(f32::INFINITY, f32::min);
                    let hi = samples
                        .iter()
                        .map(|s| s[channel])
                        .fold(f32::NEG_INFINITY, f32::max);
                    let basis = std::array::from_fn(|i| {
                        basis((samples[i][channel] - lo) / (hi - lo).max(1e-6))
                    });
                    templates.push(Template {
                        fractal: p.clone(),
                        channel,
                        range: [lo, hi],
                        inverse: inverse_gram(&basis),
                        basis,
                    });
                }
            }
        }
    }
    templates
}

fn target_grid(image: &HdrImage, r: Region) -> [[f32; 4]; SAMPLES] {
    std::array::from_fn(|i| {
        let gx = (i % GRID) as u32;
        let gy = (i / GRID) as u32;
        let x0 = gx * r.width / GRID as u32;
        let y0 = gy * r.height / GRID as u32;
        let x1 = ((gx + 1) * r.width / GRID as u32).max(x0 + 1);
        let y1 = ((gy + 1) * r.height / GRID as u32).max(y0 + 1);
        let mut sum = [0.0; 4];
        for y in y0..y1 {
            for x in x0..x1 {
                let p = associated(image.get(r.x + x, r.y + y));
                for c in 0..4 {
                    sum[c] += p[c];
                }
            }
        }
        sum.map(|v| v / ((x1 - x0) * (y1 - y0)) as f32)
    })
}

fn fit(image: &HdrImage, r: Region, templates: &[Template]) -> MosaicTile {
    let target = target_grid(image, r);
    let mean: [f32; 4] =
        std::array::from_fn(|c| target.iter().map(|p| p[c]).sum::<f32>() / SAMPLES as f32);
    let mut best_error = f32::INFINITY;
    let mut best = 0;
    let mut best_palette = [mean; 4];
    for (index, template) in templates.iter().enumerate() {
        let mut rhs = [mean.map(|v| RIDGE * v); 4];
        for (b, p) in template.basis.iter().zip(&target) {
            for i in 0..4 {
                for (c, value) in p.iter().enumerate() {
                    rhs[i][c] += b[i] * value;
                }
            }
        }
        let mut palette: [[f32; 4]; 4] = std::array::from_fn(|i| {
            std::array::from_fn(|c| {
                (0..4)
                    .map(|j| template.inverse[i][j] * rhs[j][c])
                    .sum::<f32>()
                    .clamp(0.0, 1.0)
            })
        });
        for p in &mut palette {
            for c in 0..3 {
                p[c] = p[c].min(p[3]);
            }
        }
        let mut error = 0.0;
        for (b, target) in template.basis.iter().zip(&target) {
            for (c, value) in target.iter().enumerate() {
                let predicted = (0..4).map(|i| b[i] * palette[i][c]).sum::<f32>();
                error += (predicted - value).powi(2);
            }
        }
        if error < best_error {
            best_error = error;
            best = index;
            best_palette = palette;
        }
    }
    let t = &templates[best];
    MosaicTile {
        region: r,
        fractal: t.fractal.clone(),
        channel: t.channel,
        feature_range: t.range,
        palette: best_palette,
        error: (best_error / (SAMPLES * 4) as f32).sqrt(),
    }
}

pub fn decompose(image: &HdrImage, config: MosaicConfig) -> Result<Mosaic> {
    decompose_controlled(image, config, &MosaicControl::default())
}

pub fn decompose_controlled(
    image: &HdrImage,
    config: MosaicConfig,
    control: &MosaicControl,
) -> Result<Mosaic> {
    let regions = subdivide_controlled(image, config, control)?;
    let templates = atlas();
    control.begin(1, regions.len());
    let mosaic = Mosaic {
        version: 1,
        width: image.width,
        height: image.height,
        tiles: regions
            .par_iter()
            .map(|r| {
                control.check()?;
                let tile = fit(image, *r, &templates);
                control.advance(1);
                Ok(tile)
            })
            .collect::<Result<Vec<_>>>()?,
    };
    control.check()?;
    mosaic.validate().map_err(anyhow::Error::msg)?;
    Ok(mosaic)
}

/// Reconstruct without the original image, with 2x2 supersampling. Integer
/// scaling preserves the exact shared boundaries of neighbouring regions.
pub fn reconstruct(mosaic: &Mosaic, scale: u32) -> Result<HdrImage> {
    reconstruct_controlled(mosaic, scale, &MosaicControl::default())
}

pub fn reconstruct_controlled(
    mosaic: &Mosaic,
    scale: u32,
    control: &MosaicControl,
) -> Result<HdrImage> {
    control.check()?;
    mosaic.validate().map_err(anyhow::Error::msg)?;
    let width = mosaic.width.checked_mul(scale);
    let height = mosaic.height.checked_mul(scale);
    ensure!(
        scale > 0 && width.is_some() && height.is_some(),
        "invalid mosaic output scale"
    );
    let (width, height) = (width.unwrap(), height.unwrap());
    ensure!(
        u64::from(width) * u64::from(height) <= MAX_MOSAIC_PIXELS,
        "mosaic output exceeds {MAX_MOSAIC_PIXELS} pixels"
    );
    control.begin(2, width as usize * height as usize);
    let blocks: Vec<_> = mosaic
        .tiles
        .par_iter()
        .map(|tile| {
            control.check()?;
            let w = tile.region.width * scale;
            let h = tile.region.height * scale;
            let mut pixels = Vec::with_capacity((w as usize) * (h as usize));
            for y in 0..h {
                control.check()?;
                for x in 0..w {
                    let mut p = [0.0; 4];
                    for dy in [0.25, 0.75] {
                        for dx in [0.25, 0.75] {
                            let s =
                                tile.sample((x as f32 + dx) / w as f32, (y as f32 + dy) / h as f32);
                            for c in 0..4 {
                                p[c] += 0.25 * s[c];
                            }
                        }
                    }
                    if p[3] > 0.0 {
                        for c in 0..3 {
                            p[c] /= p[3];
                        }
                    }
                    pixels.push(p);
                }
            }
            control.advance(w as usize * h as usize);
            Ok((tile.region, pixels))
        })
        .collect::<Result<Vec<_>>>()?;
    control.check()?;
    let mut output = HdrImage::new(width, height);
    for (r, pixels) in blocks {
        let w = (r.width * scale) as usize;
        for (row, data) in pixels.chunks_exact(w).enumerate() {
            let start = ((r.y * scale) as usize + row) * width as usize + (r.x * scale) as usize;
            output.data[start..start + w].copy_from_slice(data);
        }
    }
    Ok(output)
}

/// Diagnostic overlay at analysis resolution; does not affect reconstruction.
pub fn layout(image: &HdrImage, mosaic: &Mosaic) -> Result<HdrImage> {
    mosaic.validate().map_err(anyhow::Error::msg)?;
    ensure!(
        image.width == mosaic.width && image.height == mosaic.height,
        "layout dimensions differ"
    );
    let mut output = image.clone();
    for tile in &mosaic.tiles {
        let r = tile.region;
        for y in r.y..r.y + r.height {
            for x in r.x..r.x + r.width {
                if x == r.x || y == r.y {
                    output.set(x, y, [1.0, 0.25, 0.015, 1.0]);
                }
            }
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controlled_jobs_report_progress_and_cancel_without_partial_images() {
        let source = HdrImage::new(4, 3);
        let control = MosaicControl::default();
        let document = decompose_controlled(&source, MosaicConfig::default(), &control).unwrap();
        assert_eq!(control.progress(), (1, 1, 1));
        let rendered = reconstruct_controlled(&document, 2, &control).unwrap();
        assert_eq!(rendered.data.len(), 48);
        assert_eq!(control.progress(), (2, 48, 48));
        control.cancel();
        assert!(subdivide_controlled(&source, MosaicConfig::default(), &control).is_err());
        assert!(decompose_controlled(&source, MosaicConfig::default(), &control).is_err());
        assert!(reconstruct_controlled(&document, 1, &control).is_err());
    }

    fn coverage(image: &HdrImage, regions: &[Region]) {
        let mut counts = vec![0; image.data.len()];
        for r in regions {
            assert!(r.width > 0 && r.height > 0);
            assert!(r.x + r.width <= image.width && r.y + r.height <= image.height);
            for y in r.y..r.y + r.height {
                for x in r.x..r.x + r.width {
                    counts[image.index(x, y)] += 1;
                }
            }
        }
        assert!(counts.iter().all(|c| *c == 1));
    }

    #[test]
    fn subdivision_spends_regions_on_detail_and_covers_odd_edges() {
        let mut image = HdrImage::new(137, 83);
        for y in 0..image.height {
            for x in 0..image.width {
                let value = if x < 68 { 0.3 } else { ((x + y) % 2) as f32 };
                image.set(x, y, [value, value, value, 1.0]);
            }
        }
        let config = MosaicConfig {
            min_size: 5,
            max_regions: 500,
            ..MosaicConfig::default()
        };
        let regions = subdivide(&image, config).unwrap();
        coverage(&image, &regions);
        assert_eq!(regions, subdivide(&image, config).unwrap());
        let left = regions
            .iter()
            .filter(|r| r.x + r.width <= 68)
            .map(|r| r.area())
            .max()
            .unwrap();
        let right = regions
            .iter()
            .filter(|r| r.x >= 68)
            .map(|r| r.area())
            .max()
            .unwrap();
        assert!(left > right * 4, "flat {left} vs detailed {right}");
    }

    #[test]
    fn subdivision_keeps_hard_budget_and_handles_one_pixel_strips() {
        for (width, height) in [(79, 47), (1, 79), (79, 1), (1, 1)] {
            let mut image = HdrImage::new(width, height);
            for (i, p) in image.data.iter_mut().enumerate() {
                *p = [(i % 3) as f32 / 2.0, 0.0, 0.0, 1.0];
            }
            for budget in [1, 2, 3, 4, 7, 16] {
                let regions = subdivide(
                    &image,
                    MosaicConfig {
                        min_size: 1,
                        max_size: 8,
                        max_regions: budget,
                        detail: 0.001,
                    },
                )
                .unwrap();
                assert!(regions.len() <= budget);
                coverage(&image, &regions);
            }
        }
    }

    #[test]
    fn invisible_rgb_does_not_consume_budget() {
        let mut image = HdrImage::new(32, 32);
        for (i, p) in image.data.iter_mut().enumerate() {
            *p = [(i % 2) as f32, 1.0, 0.0, 0.0];
        }
        assert_eq!(subdivide(&image, MosaicConfig::default()).unwrap().len(), 1);
        let document = decompose(&image, MosaicConfig::default()).unwrap();
        let output = reconstruct(&document, 1).unwrap();
        assert!(output.data.iter().all(|p| *p == [0.0; 4]));
    }

    #[test]
    fn structural_fit_recovers_an_orbit_pattern_better_than_its_mean() {
        let templates = atlas();
        let t = &templates[17];
        let mut image = HdrImage::new(GRID as u32, GRID as u32);
        for (i, p) in image.data.iter_mut().enumerate() {
            let v = t.basis[i][1] / 3.0 + t.basis[i][2] * 2.0 / 3.0 + t.basis[i][3];
            *p = [0.1 + v * 0.8, 0.2 + v * 0.5, 0.05 + v * 0.3, 1.0];
        }
        let region = Region {
            x: 0,
            y: 0,
            width: GRID as u32,
            height: GRID as u32,
        };
        let result = fit(&image, region, &templates);
        let mean = image.data.iter().map(|p| p[0]).sum::<f32>() / SAMPLES as f32;
        let flat = (image
            .data
            .iter()
            .map(|p| (p[0] - mean).powi(2))
            .sum::<f32>()
            / SAMPLES as f32)
            .sqrt();
        assert!(flat > 0.05, "test pattern must have structure");
        assert!(
            result.error < flat * 0.02,
            "fit {} vs flat {flat}",
            result.error
        );
    }

    #[test]
    fn recipes_roundtrip_reconstruct_without_source_and_reject_bad_coverage() {
        let image = HdrImage::from_data(7, 5, vec![[0.2, 0.6, 0.7, 0.4]; 35]);
        let document = decompose(&image, MosaicConfig::default()).unwrap();
        drop(image);
        let text =
            ron::ser::to_string_pretty(&document, ron::ser::PrettyConfig::default()).unwrap();
        let restored: Mosaic = ron::from_str(&text).unwrap();
        assert_eq!(document, restored);
        let rendered = reconstruct(&restored, 2).unwrap();
        assert_eq!((rendered.width, rendered.height), (14, 10));
        for p in &rendered.data {
            for c in 0..4 {
                assert!((p[c] - [0.2, 0.6, 0.7, 0.4][c]).abs() < 0.001);
            }
        }
        assert!(reconstruct(&restored, 0).is_err());
        assert!(reconstruct(&restored, u32::MAX).is_err());
        let mut broken = restored.clone();
        broken.tiles.push(broken.tiles[0].clone());
        assert!(broken.validate().unwrap_err().contains("overlapping"));
        broken = restored.clone();
        broken.tiles[0].region.width -= 1;
        assert!(broken.validate().unwrap_err().contains("incomplete"));
        broken = restored;
        broken.tiles[0].fractal.zoom = f32::NAN;
        assert!(broken.validate().is_err());
    }

    #[test]
    fn bad_configuration_and_pixels_fail_before_analysis() {
        let image = HdrImage::new(8, 8);
        for config in [
            MosaicConfig {
                min_size: 0,
                ..MosaicConfig::default()
            },
            MosaicConfig {
                max_regions: 0,
                ..MosaicConfig::default()
            },
            MosaicConfig {
                max_size: 1,
                ..MosaicConfig::default()
            },
            MosaicConfig {
                detail: f32::NAN,
                ..MosaicConfig::default()
            },
        ] {
            assert!(subdivide(&image, config).is_err());
        }
        let mut invalid = image;
        invalid.data[0][0] = f32::INFINITY;
        assert!(decompose(&invalid, MosaicConfig::default()).is_err());
    }
}
