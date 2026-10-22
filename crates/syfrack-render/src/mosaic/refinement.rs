//! Refine fitted leaves against an independent reconstruction estimate.
use super::*;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

// Fit the same integrated signal that reconstruction produces. Small leaves
// can be narrower than the fitting grid: their repeated cells must sample the
// same actual pixel rather than different points in a fictitious 8x8 image.
struct IntegratedAtlas {
    base: Vec<Template>,
    sizes: Mutex<HashMap<(u32, u32), Arc<OnceLock<Vec<Template>>>>>,
}
impl IntegratedAtlas {
    fn new() -> Self {
        Self {
            base: atlas(),
            sizes: Mutex::new(HashMap::new()),
        }
    }
    fn for_region(&self, r: Region) -> Arc<OnceLock<Vec<Template>>> {
        let entry = self
            .sizes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry((r.width, r.height))
            .or_default()
            .clone();
        entry.get_or_init(|| {
            self.base
                .iter()
                .map(|template| {
                    let basis = std::array::from_fn(|i| {
                        let x0 = (i % GRID) as u32 * r.width / GRID as u32;
                        let y0 = (i / GRID) as u32 * r.height / GRID as u32;
                        let x1 = (((i % GRID) + 1) as u32 * r.width / GRID as u32).max(x0 + 1);
                        let y1 = (((i / GRID) + 1) as u32 * r.height / GRID as u32).max(y0 + 1);
                        let mut integrated = [0.0; 4];
                        for dy in [0.25, 0.75] {
                            for dx in [0.25, 0.75] {
                                let u = (x0 as f32 + dx * (x1 - x0) as f32) / r.width as f32;
                                let v = (y0 as f32 + dy * (y1 - y0) as f32) / r.height as f32;
                                let feature = template
                                    .fractal
                                    .sample(Vec2::new(3.0 * (u - 0.5), 3.0 * (v - 0.5)))
                                    .traps[template.channel];
                                let b = basis(
                                    (feature - template.range[0])
                                        / (template.range[1] - template.range[0]).max(1e-6),
                                );
                                for c in 0..4 {
                                    integrated[c] += 0.25 * b[c];
                                }
                            }
                        }
                        integrated
                    });
                    Template {
                        fractal: template.fractal.clone(),
                        channel: template.channel,
                        range: template.range,
                        inverse: inverse_gram(&basis),
                        basis,
                    }
                })
                .collect()
        });
        entry
    }
}

/// Priority values are in analysis pixel order, in [0, 1]. A painted pixel
/// receives up to sixteen times the weight, within the same hard tile budget.
#[derive(Clone, Copy, Debug)]
pub struct MosaicGuidance<'a> {
    pub refinement: f32,
    pub priority: Option<&'a [f32]>,
}
impl Default for MosaicGuidance<'_> {
    fn default() -> Self {
        Self {
            refinement: 1.0,
            priority: None,
        }
    }
}

pub(super) fn validate_priority(image: &HdrImage, priority: Option<&[f32]>) -> Result<()> {
    if let Some(values) = priority {
        ensure!(
            values.len() == image.data.len()
                && values
                    .iter()
                    .all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
            "invalid mosaic priority map"
        );
    }
    Ok(())
}

struct Leaf {
    tile: MosaicTile,
    rank: Candidate,
    loss: f64,
}
impl PartialEq for Leaf {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Leaf {}
impl PartialOrd for Leaf {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Leaf {
    fn cmp(&self, other: &Self) -> Ordering {
        self.rank.cmp(&other.rank)
    }
}

fn perceptual(p: [f32; 4]) -> [f32; 4] {
    [
        p[0].sqrt() * p[3],
        p[1].sqrt() * p[3],
        p[2].sqrt() * p[3],
        p[3],
    ]
}

/// Measure every reconstructed analysis pixel with exactly the final renderer's
/// 2x2 supersampling. Palette-fit grid error cannot substitute for this signal.
fn reconstruction_loss(
    image: &HdrImage,
    tile: &MosaicTile,
    priority: Option<&[f32]>,
    control: &MosaicControl,
) -> Result<f64> {
    let r = tile.region;
    let mut loss = 0.0;
    for y in 0..r.height {
        control.check()?;
        for x in 0..r.width {
            let mut predicted = [0.0; 4];
            for dy in [0.25, 0.75] {
                for dx in [0.25, 0.75] {
                    let p = tile.sample(
                        (x as f32 + dx) / r.width as f32,
                        (y as f32 + dy) / r.height as f32,
                    );
                    for c in 0..4 {
                        predicted[c] += p[c] * 0.25;
                    }
                }
            }
            if predicted[3] > 0.0 {
                for c in 0..3 {
                    predicted[c] /= predicted[3];
                }
            }
            let predicted = perceptual(predicted);
            let target = perceptual(image.get(r.x + x, r.y + y));
            let weight = 1.0 + 15.0 * priority.map_or(0.0, |m| m[image.index(r.x + x, r.y + y)]);
            loss += f64::from(weight)
                * (0..4)
                    .map(|c| f64::from(target[c] - predicted[c]).powi(2))
                    .sum::<f64>()
                / 4.0;
        }
    }
    Ok(loss)
}

fn leaf(
    image: &HdrImage,
    region: Region,
    templates: &IntegratedAtlas,
    guidance: MosaicGuidance<'_>,
    config: MosaicConfig,
    control: &MosaicControl,
) -> Result<Leaf> {
    control.check()?;
    let integrated = templates.for_region(region);
    let mut tile = fit(image, region, integrated.get().unwrap());
    let mut loss = reconstruction_loss(image, &tile, guidance.priority, control)?;
    let original = fit(image, region, &templates.base);
    let original_loss = reconstruction_loss(image, &original, guidance.priority, control)?;
    if original_loss < loss {
        tile = original;
        loss = original_loss;
    }
    let rank = Candidate {
        region,
        priority: if region.width.max(region.height) > config.max_size {
            1e12 + region.area() as f64
        } else {
            loss
        },
        error: (loss / region.area() as f64).sqrt(),
    };
    Ok(Leaf { tile, rank, loss })
}

/// Reserve part of the budget, fit the coarse leaves, then spend the reserve on
/// the largest weighted reconstruction losses. A split is retained only if its
/// children reduce measured loss, or are needed for the preferred size cap.
/// At zero refinement, reproduce the original source-driven decomposition.
pub fn decompose_guided_controlled(
    image: &HdrImage,
    config: MosaicConfig,
    guidance: MosaicGuidance<'_>,
    control: &MosaicControl,
) -> Result<Mosaic> {
    control.check()?;
    config.validate()?;
    validate_source(image)?;
    validate_priority(image, guidance.priority)?;
    ensure!(
        guidance.refinement.is_finite() && (0.0..=1.0).contains(&guidance.refinement),
        "invalid mosaic refinement strength"
    );
    if guidance.refinement == 0.0 && guidance.priority.is_none() {
        return decompose_controlled(image, config, control);
    }
    let coarse = MosaicConfig {
        max_regions: ((config.max_regions as f32 * (1.0 - 0.35 * guidance.refinement)) as usize)
            .max(1),
        ..config
    };
    let regions = subdivide_guided_controlled(image, coarse, guidance.priority, control)?;
    let templates = IntegratedAtlas::new();
    control.begin(1, regions.len());
    let fitted = regions
        .par_iter()
        .map(|r| {
            let node = leaf(image, *r, &templates, guidance, config, control)?;
            control.advance(1);
            Ok(node)
        })
        .collect::<Result<Vec<_>>>()?;
    let mut queue = BinaryHeap::from(fitted);
    let mut leaves = Vec::new();
    let mut final_loss = 0.0;
    control.begin(3, config.max_regions);
    control.advance(queue.len());
    while let Some(node) = queue.pop() {
        control.check()?;
        let r = node.tile.region;
        let parts = children(r, config.min_size);
        let oversized = r.width.max(r.height) > config.max_size;
        if parts.len() == 1
            || leaves.len() + queue.len() + parts.len() > config.max_regions
            || (!oversized && node.rank.error <= f64::from(config.detail))
            || guidance.refinement == 0.0
        {
            final_loss += node.loss;
            leaves.push(node.tile);
            continue;
        }
        let refined = parts
            .par_iter()
            .map(|r| leaf(image, *r, &templates, guidance, config, control))
            .collect::<Result<Vec<_>>>()?;
        let loss: f64 = refined.iter().map(|n| n.loss).sum();
        if oversized || loss < node.loss * (1.0 - 1e-4) {
            control.advance(refined.len() - 1);
            queue.extend(refined);
        } else {
            final_loss += node.loss;
            leaves.push(node.tile);
        }
    }
    control.check()?;
    // Preserve a measured quality floor at the same analysis resolution and
    // hard budget. Priorities use the same weights in both comparisons.
    let baseline_regions = subdivide_controlled(image, config, control)?;
    control.begin(3, baseline_regions.len());
    let baseline = baseline_regions
        .par_iter()
        .map(|r| {
            control.check()?;
            let tile = fit(image, *r, &templates.base);
            let loss = reconstruction_loss(image, &tile, guidance.priority, control)?;
            control.advance(1);
            Ok((tile, loss))
        })
        .collect::<Result<Vec<_>>>()?;
    let baseline_loss: f64 = baseline.iter().map(|(_, loss)| loss).sum();
    control.check()?;
    if baseline_loss < final_loss {
        leaves = baseline.into_iter().map(|(tile, _)| tile).collect();
    }
    control.begin(3, leaves.len());
    control.advance(leaves.len());
    leaves.sort_by_key(|t| (t.region.y, t.region.x));
    let mosaic = Mosaic {
        version: 1,
        width: image.width,
        height: image.height,
        tiles: leaves,
    };
    mosaic.validate().map_err(anyhow::Error::msg)?;
    Ok(mosaic)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integrated_basis_matches_actual_supersampling_below_the_fit_grid_size() {
        let templates = IntegratedAtlas::new();
        let region = Region {
            x: 0,
            y: 0,
            width: 6,
            height: 3,
        };
        let integrated = templates.for_region(region);
        let template = &integrated.get().unwrap()[17];
        let tile = MosaicTile {
            region,
            fractal: template.fractal.clone(),
            channel: template.channel,
            feature_range: template.range,
            palette: [
                [0.1, 0.3, 0.2, 1.0],
                [0.4, 0.2, 0.6, 1.0],
                [0.7, 0.8, 0.3, 1.0],
                [0.9, 0.5, 0.8, 1.0],
            ],
            error: 0.0,
        };
        for i in 0..SAMPLES {
            let x = (i % GRID) as u32 * region.width / GRID as u32;
            let y = (i / GRID) as u32 * region.height / GRID as u32;
            let mut actual = [0.0; 4];
            for dy in [0.25, 0.75] {
                for dx in [0.25, 0.75] {
                    let sample = tile.sample(
                        (x as f32 + dx) / region.width as f32,
                        (y as f32 + dy) / region.height as f32,
                    );
                    for c in 0..4 {
                        actual[c] += sample[c] * 0.25;
                    }
                }
            }
            for (c, value) in actual.iter().enumerate() {
                let fitted: f32 = (0..4)
                    .map(|j| template.basis[i][j] * tile.palette[j][c])
                    .sum();
                assert!((value - fitted).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn guided_refinement_preserves_coverage_budget_and_determinism() {
        let mut image = HdrImage::new(79, 47);
        for y in 0..47 {
            for x in 0..79 {
                let v = if x < 39 {
                    (x / 5 % 2) as f32 * 0.7 + 0.1
                } else {
                    (y / 3 % 2) as f32 * 0.6 + 0.2
                };
                image.set(x, y, [v, v * 0.4, 0.2, 1.0]);
            }
        }
        let config = MosaicConfig {
            min_size: 3,
            max_regions: 61,
            detail: 0.01,
            ..Default::default()
        };
        let run = || {
            decompose_guided_controlled(
                &image,
                config,
                MosaicGuidance::default(),
                &MosaicControl::default(),
            )
            .unwrap()
        };
        let a = run();
        assert!(a.tiles.len() <= 61);
        assert!(a.tiles.len() > 20);
        a.validate().unwrap();
        assert_eq!(a, run());
        for (w, h) in [(1, 19), (19, 1), (1, 1)] {
            let input = HdrImage::from_data(w, h, vec![[0.2, 0.4, 0.7, 0.5]; (w * h) as usize]);
            for budget in [1, 2, 3, 4, 7] {
                decompose_guided_controlled(
                    &input,
                    MosaicConfig {
                        min_size: 1,
                        max_regions: budget,
                        ..Default::default()
                    },
                    MosaicGuidance::default(),
                    &MosaicControl::default(),
                )
                .unwrap()
                .validate()
                .unwrap();
            }
        }
    }

    #[test]
    fn guidance_validates_and_zero_strength_matches_legacy() {
        let image = HdrImage::from_data(8, 8, vec![[0.2, 0.6, 0.7, 0.4]; 64]);
        let config = MosaicConfig::default();
        let run = |g| decompose_guided_controlled(&image, config, g, &MosaicControl::default());
        assert_eq!(
            run(MosaicGuidance {
                refinement: 0.0,
                priority: None
            })
            .unwrap(),
            decompose(&image, config).unwrap()
        );
        for invalid in [f32::NAN, -1.0, 2.0] {
            assert!(run(MosaicGuidance {
                refinement: invalid,
                priority: None
            })
            .is_err());
        }
        for values in [vec![0.0; 63], vec![f32::NAN; 64], vec![1.1; 64]] {
            assert!(decompose_guided_controlled(
                &image,
                config,
                MosaicGuidance {
                    refinement: 1.0,
                    priority: Some(&values)
                },
                &MosaicControl::default()
            )
            .is_err());
        }
        let control = MosaicControl::default();
        control.cancel();
        assert!(
            decompose_guided_controlled(&image, config, MosaicGuidance::default(), &control)
                .is_err()
        );
    }

    #[test]
    fn refinement_reduces_rendered_error_and_priority_redistributes_the_budget() {
        let mut image = HdrImage::new(64, 32);
        for y in 0..32 {
            for x in 0..64 {
                let v = if (x % 32 + y / 2) % 11 < 4 { 0.8 } else { 0.08 };
                image.set(x, y, [v, v * 0.6, v * 0.3, 1.0]);
            }
        }
        let config = MosaicConfig {
            min_size: 2,
            max_regions: 61,
            detail: 0.01,
            ..Default::default()
        };
        let coarse = decompose(
            &image,
            MosaicConfig {
                max_regions: 21,
                ..config
            },
        )
        .unwrap();
        let refined = decompose_guided_controlled(
            &image,
            config,
            MosaicGuidance::default(),
            &MosaicControl::default(),
        )
        .unwrap();
        let error = |m: &Mosaic| {
            let output = reconstruct(m, 1).unwrap();
            output
                .data
                .iter()
                .zip(&image.data)
                .map(|(a, b)| {
                    let a = perceptual(*a);
                    let b = perceptual(*b);
                    (0..4).map(|c| (a[c] - b[c]).powi(2) as f64).sum::<f64>()
                })
                .sum::<f64>()
        };
        assert!(error(&refined) < error(&coarse) * 0.9);
        let legacy = decompose(&image, config).unwrap();
        assert!(error(&refined) <= error(&legacy) + 1e-8);
        let priority: Vec<_> = (0..32)
            .flat_map(|_| (0..64).map(|x| if x >= 32 { 1.0 } else { 0.0 }))
            .collect();
        let painted = decompose_guided_controlled(
            &image,
            config,
            MosaicGuidance {
                refinement: 1.0,
                priority: Some(&priority),
            },
            &MosaicControl::default(),
        )
        .unwrap();
        let left = painted
            .tiles
            .iter()
            .filter(|t| t.region.x + t.region.width <= 32)
            .count();
        let right = painted.tiles.iter().filter(|t| t.region.x >= 32).count();
        assert!(right > left * 2, "left {left}, right {right}");
        assert!(painted.tiles.len() <= config.max_regions);
    }
}
