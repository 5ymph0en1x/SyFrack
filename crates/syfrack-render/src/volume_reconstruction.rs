//! Lapped reconstruction of linear volume radiance from noisy integration points.
//! Geometry-gated affine models overlap with a normalized C2 partition of unity.
//! RGB shares every matrix and blending weight; hue is never an edge guide.
//! Inspired by Aurea's explicit lapped topology / overlap normalization, not its
//! quantization or entropy mixer. This is a biased export; raw transport is intact.
use rayon::prelude::*;
use syfrack_core::HdrImage;
fn kernel(r2: f64) -> f64 {
    (1.0 - r2).max(0.0).powi(3)
}
const OPTICAL_WIDTH: f64 = 0.12;
#[derive(Clone, Default)]
struct Cell {
    x: f64,
    y: f64,
    t: f64,
    count: f64,
    rgb: [f64; 3],
    noise: [f64; 3],
}
struct Model {
    x: f64,
    y: f64,
    t: f64,
    radius: f64,
    coefficients: [[f64; 3]; 3],
    noise: f64,
    peak: f64,
}

/// Primary lighting contains narrow projector shafts, unlike indirect haze.
/// Limit support to 2.5% of the short side: a 10% support erased real beams
/// into a broad fog wash even when the raw line integral resolved them.
pub fn reconstruct(image: &HdrImage, variance: &[[f32; 3]], transmission: &[f32]) -> HdrImage {
    reconstruct_lapped(image, variance, transmission, 0.025, 160)
}
/// Indirect volume transport has larger dispersion and spatial supports.
pub fn reconstruct_indirect(
    image: &HdrImage,
    variance: &[[f32; 3]],
    transmission: &[f32],
) -> HdrImage {
    reconstruct_lapped(image, variance, transmission, 0.35, 40)
}
fn reconstruct_lapped(
    image: &HdrImage,
    variance: &[[f32; 3]],
    transmission: &[f32],
    support_fraction: f64,
    anchor_divisor: usize,
) -> HdrImage {
    let (w, h) = (image.width as usize, image.height as usize);
    assert_eq!(image.data.len(), w * h);
    assert_eq!(variance.len(), w * h);
    assert_eq!(transmission.len(), w * h);
    if w == 0 || h == 0 || variance.iter().all(|p| p.iter().all(|v| *v == 0.0)) {
        return image.clone();
    }
    let short = w.min(h);
    let cell_size = (short / 160).max(1);
    let step = (short / anchor_divisor).max(2);
    let max_radius = (short as f64 * support_fraction)
        .max(2.0 * step as f64)
        .min(320.0);
    let pilot_radius = (short as f64 * 0.10).clamp(8.0, 64.0);
    let (cw, ch) = (w.div_ceil(cell_size), h.div_ceil(cell_size));
    // Aggregate all points; optical bins retain boundaries inside each cell.
    // No radiance outlier is discarded or sparsely subsampled.
    let cells: Vec<Vec<Cell>> = (0..cw * ch)
        .into_par_iter()
        .map(|i| {
            let (x, y) = ((i % cw) * cell_size, (i / cw) * cell_size);
            let mut bins: [Cell; 32] = std::array::from_fn(|_| Cell::default());
            for yy in y..(y + cell_size).min(h) {
                for xx in x..(x + cell_size).min(w) {
                    let j = yy * w + xx;
                    let bin = ((transmission[j].clamp(0.0, 1.0) * 32.0) as usize).min(31);
                    let c = &mut bins[bin];
                    c.count += 1.0;
                    c.x += xx as f64;
                    c.y += yy as f64;
                    c.t += f64::from(transmission[j]);
                    for k in 0..3 {
                        c.rgb[k] += f64::from(image.data[j][k]);
                        c.noise[k] += f64::from(variance[j][k]);
                    }
                }
            }
            bins.into_iter()
                .filter(|c| c.count > 0.0)
                .map(|mut c| {
                    c.x /= c.count;
                    c.y /= c.count;
                    c.t /= c.count;
                    for k in 0..3 {
                        c.rgb[k] /= c.count;
                        c.noise[k] /= c.count;
                    }
                    c
                })
                .collect()
        })
        .collect();
    // Cover every optical class, including one-pixel foreground silhouettes.
    // Each pixel has a same-class anchor within sqrt(2)*step, inside the minimum
    // support 2*step. The positive overlap denominator is therefore guaranteed.
    let (aw, ah) = (w.div_ceil(step), h.div_ceil(step));
    let anchors: Vec<(usize, usize)> = (0..aw * ah)
        .into_par_iter()
        .map(|i| {
            let (x0, y0) = ((i % aw) * step, (i / aw) * step);
            let (x1, y1) = ((x0 + step).min(w), (y0 + step).min(h));
            let (cx, cy) = ((x0 + x1 - 1) as f64 * 0.5, (y0 + y1 - 1) as f64 * 0.5);
            let mut bins: [Option<(usize, usize, f64)>; 32] = [None; 32];
            for y in y0..y1 {
                for x in x0..x1 {
                    let b = ((transmission[y * w + x].clamp(0.0, 1.0) * 32.0) as usize).min(31);
                    let d = (x as f64 - cx).powi(2) + (y as f64 - cy).powi(2);
                    if bins[b].is_none_or(|(_, _, old)| d < old) {
                        bins[b] = Some((x, y, d));
                    }
                }
            }
            bins.into_iter()
                .flatten()
                .map(|(x, y, _)| (x, y))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>()
        .into_iter()
        .flatten()
        .collect();
    let visit = |x: f64, y: f64, t: f64, r: f64, f: &mut dyn FnMut(&Cell, f64, f64, f64)| {
        let x0 = ((x - r).max(0.0) as usize / cell_size).min(cw - 1);
        let x1 = (((x + r) as usize / cell_size).min(cw - 1)).max(x0);
        let y0 = ((y - r).max(0.0) as usize / cell_size).min(ch - 1);
        let y1 = (((y + r) as usize / cell_size).min(ch - 1)).max(y0);
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                for c in &cells[cy * cw + cx] {
                    let dx = (c.x - x) / r;
                    let dy = (c.y - y) / r;
                    let dt = (c.t - t) / OPTICAL_WIDTH;
                    let weight = kernel(dx * dx + dy * dy) * kernel(dt * dt) * c.count;
                    if weight > 0.0 {
                        f(c, weight, dx, dy);
                    }
                }
            }
        }
    };
    let models: Vec<Model> = anchors
        .into_par_iter()
        .map(|(x, y)| {
            let (x, y) = (x as f64, y as f64);
            let t = f64::from(transmission[y as usize * w + x as usize]);
            let (mut rgb, mut noise, mut mass) = ([0.0f64; 3], [0.0f64; 3], 0.0);
            visit(x, y, t, pilot_radius, &mut |c, weight, _, _| {
                mass += weight;
                for k in 0..3 {
                    rgb[k] += weight * c.rgb[k];
                    noise[k] += weight * c.noise[k];
                }
            });
            let peak = rgb.into_iter().fold(0.0, f64::max) / mass.max(1e-20);
            let noise = noise.into_iter().fold(0.0, f64::max) / mass.max(1e-20);
            // Wide pilot statistics cannot give a lone bright point its own small support.
            // Effective sample count of this radial kernel is 7*pi*R^2/16.
            let desired = noise.max(0.0).sqrt()
                / (0.02 * peak.max(1e-6) * (7.0 * std::f64::consts::PI / 16.0).sqrt());
            let radius = desired.clamp(2.0 * step as f64, max_radius);
            let mut a = [0.0f64; 6];
            let mut b = [[0.0f64; 3]; 3];
            visit(x, y, t, radius, &mut |c, weight, dx, dy| {
                a[0] += weight;
                a[1] += weight * dx;
                a[2] += weight * dy;
                a[3] += weight * dx * dx;
                a[4] += weight * dx * dy;
                a[5] += weight * dy * dy;
                for k in 0..3 {
                    b[k][0] += weight * c.rgb[k];
                    b[k][1] += weight * c.rgb[k] * dx;
                    b[k][2] += weight * c.rgb[k] * dy;
                }
            });
            a[3] += a[0] * 1e-10;
            a[5] += a[0] * 1e-10;
            let inv = [
                a[3] * a[5] - a[4] * a[4],
                a[2] * a[4] - a[1] * a[5],
                a[1] * a[4] - a[2] * a[3],
                a[0] * a[5] - a[2] * a[2],
                a[1] * a[2] - a[0] * a[4],
                a[0] * a[3] - a[1] * a[1],
            ];
            let det = a[0] * inv[0] + a[1] * inv[1] + a[2] * inv[2];
            let coefficients = std::array::from_fn(|k| {
                if det > 1e-24 {
                    [
                        (inv[0] * b[k][0] + inv[1] * b[k][1] + inv[2] * b[k][2]) / det,
                        (inv[1] * b[k][0] + inv[3] * b[k][1] + inv[4] * b[k][2]) / det,
                        (inv[2] * b[k][0] + inv[4] * b[k][1] + inv[5] * b[k][2]) / det,
                    ]
                } else {
                    [b[k][0] / a[0].max(1e-20), 0.0, 0.0]
                }
            });
            Model {
                x,
                y,
                t,
                radius,
                coefficients,
                noise,
                peak,
            }
        })
        .collect();
    let mut data = image.data.clone();
    // Disjoint row bands and fixed model order make synthesis worker-independent,
    // following the same ordering principle as Aurea's lapped synthesis.
    data.par_chunks_mut(w * 16)
        .enumerate()
        .for_each(|(band, out)| {
            let y0 = band * 16;
            let y1 = y0 + out.len() / w;
            let mut sum = vec![[0.0f64; 4]; out.len()];
            let mut stats = vec![[0.0f64; 2]; out.len()];
            for m in &models {
                let lo = ((m.y - m.radius).max(y0 as f64).ceil() as usize).min(y1);
                let hi = ((m.y + m.radius).floor() as usize + 1).min(y1);
                if lo >= hi {
                    continue;
                }
                let xl = ((m.x - m.radius).max(0.0).ceil() as usize).min(w);
                let xh = ((m.x + m.radius).floor() as usize + 1).min(w);
                for y in lo..hi {
                    for x in xl..xh {
                        let i = (y - y0) * w + x;
                        let dx = (x as f64 - m.x) / m.radius;
                        let dy = (y as f64 - m.y) / m.radius;
                        let dt = (f64::from(transmission[y * w + x]) - m.t) / OPTICAL_WIDTH;
                        let weight =
                            kernel(dx * dx + dy * dy) * kernel(dt * dt) / (m.radius * m.radius);
                        if weight == 0.0 {
                            continue;
                        }
                        for k in 0..3 {
                            sum[i][k] += weight
                                * (m.coefficients[k][0]
                                    + m.coefficients[k][1] * dx
                                    + m.coefficients[k][2] * dy);
                        }
                        sum[i][3] += weight;
                        stats[i][0] += weight * m.noise;
                        stats[i][1] += weight * m.peak;
                    }
                }
            }
            for (i, p) in out.iter_mut().enumerate() {
                let mass = sum[i][3];
                if mass <= 1e-20 {
                    continue;
                }
                let noise = stats[i][0] / mass;
                let tolerance = (0.005 * stats[i][1] / mass).powi(2);
                // As dispersion tends to zero, reconstruction tends to raw identity.
                let blend = noise / (noise + tolerance).max(1e-30);
                for k in 0..3 {
                    p[k] = ((1.0 - blend) * f64::from(p[k]) + blend * (sum[i][k] / mass).max(0.0))
                        as f32;
                }
            }
        });
    HdrImage::from_data(image.width, image.height, data)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn narrow_direct_beam_retains_profile_colour_and_energy() {
        let (w, h) = (256, 256);
        let image = HdrImage::from_data(
            w as u32,
            h as u32,
            (0..w * h)
                .map(|i| {
                    let x = (i % w) as f32;
                    let y = (i / w) as f32;
                    let distance = x - (128.0 + 0.35 * (y - 128.0));
                    let beam = (-0.5 * (distance / 3.0).powi(2)).exp();
                    [0.18 * beam, 0.60 * beam, beam, 1.0]
                })
                .collect(),
        );
        let out = reconstruct(&image, &vec![[0.5; 3]; w * h], &vec![0.8; w * h]);
        assert!(
            out.get(128, 128)[2] > 0.70,
            "a resolved shaft must not dissolve into haze"
        );
        assert!(
            out.get(146, 128)[2] < 0.015,
            "light must not spread far outside the beam"
        );
        let before: f64 = image.data.iter().map(|p| f64::from(p[2])).sum();
        let after: f64 = out.data.iter().map(|p| f64::from(p[2])).sum();
        assert!(
            (after / before - 1.0).abs() < 0.03,
            "retain integrated beam energy"
        );
        for p in &out.data {
            assert!((p[0] - 0.18 * p[2]).abs() < 1e-6);
            assert!((p[1] - 0.60 * p[2]).abs() < 1e-6);
        }
    }
    #[test]
    fn affine_colour_mixtures_survive_borders_and_optical_discontinuities() {
        let (w, h) = (71, 53);
        let image = HdrImage::from_data(
            w as u32,
            h as u32,
            (0..w * h)
                .map(|i| {
                    let x = (i % w) as f32 / w as f32;
                    let y = (i / w) as f32 / h as f32;
                    [0.1 + 0.7 * x, 0.03 + 0.3 * y, 0.9 - 0.6 * x, 1.0]
                })
                .collect(),
        );
        let t = (0..w * h)
            .map(|i| if i % w < w / 2 { 0.9 } else { 0.2 })
            .collect::<Vec<_>>();
        for out in [
            reconstruct(&image, &vec![[0.2; 3]; w * h], &t),
            reconstruct_indirect(&image, &vec![[0.2; 3]; w * h], &t),
        ] {
            let error = image
                .data
                .iter()
                .zip(out.data)
                .flat_map(|(a, b)| (0..3).map(move |c| (a[c] - b[c]).abs()))
                .fold(0.0, f32::max);
            assert!(error < 2e-6, "gradient error {error}");
        }
    }
    #[test]
    fn chromatic_shot_noise_is_reduced_without_colour_gating_or_crossing_a_boundary() {
        let (w, h) = (96, 64);
        let expected = (0..w * h)
            .map(|i| {
                let x = (i % w) as f32 / w as f32;
                if i % w < w / 2 {
                    [0.1 + 0.5 * x, 0.02, 0.6 - 0.4 * x, 1.0]
                } else {
                    [0.002, 0.003, 0.004, 1.0]
                }
            })
            .collect::<Vec<_>>();
        let mut source = expected.clone();
        for (i, p) in source.iter_mut().enumerate() {
            for c in 0..3 {
                let hash = (i as u32)
                    .wrapping_mul(747796405u32)
                    .wrapping_add(c as u32 * 277803737u32);
                p[c] *= if hash % 11 == 0 { 11.0 } else { 0.0 };
            }
        }
        let image = HdrImage::from_data(w as u32, h as u32, source.clone());
        let t = (0..w * h)
            .map(|i| if i % w < w / 2 { 0.2 } else { 0.98 })
            .collect::<Vec<_>>();
        let variance = expected
            .iter()
            .map(|p| [p[0] * p[0] * 10.0, p[1] * p[1] * 10.0, p[2] * p[2] * 10.0])
            .collect::<Vec<_>>();
        let result = reconstruct_indirect(&image, &variance, &t);
        let error = |pixels: &[[f32; 4]]| {
            pixels
                .iter()
                .zip(&expected)
                .map(|(a, b)| (0..3).map(|c| f64::from(a[c] - b[c]).powi(2)).sum::<f64>())
                .sum::<f64>()
        };
        assert!(error(&result.data) < error(&source) * 0.08);
        assert!(result
            .data
            .iter()
            .enumerate()
            .filter(|(i, _)| i % w >= w / 2)
            .all(|(_, p)| p[..3].iter().all(|v| *v < 0.03)));
        assert_eq!(image.data, source);
    }
    #[test]
    fn reconstruction_is_finite_and_worker_count_independent() {
        let image = HdrImage::from_data(1, 9, vec![[0.2, 0.4, 0.8, 1.0]; 9]);
        let run = |threads| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| reconstruct(&image, &vec![[1.0; 3]; 9], &vec![0.2; 9]))
        };
        assert_eq!(run(1).data, run(4).data);
        assert!(run(1).data.iter().flatten().all(|v| v.is_finite()));
    }
    #[test]
    fn zero_dispersion_is_identity_for_non_linear_detail() {
        let image = HdrImage::from_data(
            71,
            53,
            (0..71 * 53)
                .map(|i| [(i as f32 * 0.013).sin().abs(), 0.2, 0.4, 1.0])
                .collect(),
        );
        for result in [
            reconstruct(&image, &vec![[0.0; 3]; 71 * 53], &vec![0.2; 71 * 53]),
            reconstruct_indirect(&image, &vec![[0.0; 3]; 71 * 53], &vec![0.2; 71 * 53]),
        ] {
            assert_eq!(result.data, image.data);
        }
    }
}
