//! Deterministic parallel orbit accumulation and filament-aware reconstruction.
//! f64 dynamics, integer additive mass/RGB moments, no tile-dependent hue gating.
use anyhow::{bail, Result};
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use syfrack_core::{Canvas2D, Flame, FlameTransform, FlameVariation, HdrImage};

const UNIT: u64 = 65536;
const LANES: usize = 64;

pub struct FlameAccumulator {
    flame: Flame,
    canvas: Canvas2D,
    seed: u64,
    width: u32,
    height: u32,
    histogram: Vec<[AtomicU64; 4]>,
    palette: Vec<[f64; 3]>,
    cdf: Vec<f64>,
    pub passes: u32,
}

fn random(state: &mut u64) -> f64 {
    *state = state.wrapping_add(0x9e3779b97f4a7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    ((z ^ (z >> 31)) >> 11) as f64 / 9007199254740992.0
}

fn affine(a: [f32; 6], p: [f64; 2]) -> [f64; 2] {
    [
        a[0] as f64 * p[0] + a[1] as f64 * p[1] + a[2] as f64,
        a[3] as f64 * p[0] + a[4] as f64 * p[1] + a[5] as f64,
    ]
}

pub fn transform(t: &FlameTransform, p: [f64; 2], rng: &mut u64) -> [f64; 2] {
    let [x, y] = affine(t.affine, p);
    let r2 = (x * x + y * y).max(1e-24);
    let r = r2.sqrt();
    let theta = y.atan2(x);
    let mut result = [0.0; 2];
    for term in &t.terms {
        let v = match term.variation {
            FlameVariation::Linear => [x, y],
            FlameVariation::Sinusoidal => [x.sin(), y.sin()],
            FlameVariation::Spherical => [x / r2, y / r2],
            FlameVariation::Swirl => {
                let (s, c) = r2.sin_cos();
                [x * s - y * c, x * c + y * s]
            }
            FlameVariation::Horseshoe => [(x - y) * (x + y) / r, 2.0 * x * y / r],
            FlameVariation::Disc => {
                let (s, c) = (std::f64::consts::PI * r).sin_cos();
                [
                    theta / std::f64::consts::PI * s,
                    theta / std::f64::consts::PI * c,
                ]
            }
            FlameVariation::Bubble => [4.0 * x / (r2 + 4.0), 4.0 * y / (r2 + 4.0)],
            FlameVariation::Julian { power, distance } => {
                let branch = (random(rng) * power as f64).floor();
                let a = (theta + std::f64::consts::TAU * branch) / power as f64;
                let radius = r2.powf(distance as f64 / (2.0 * power as f64));
                [radius * a.cos(), radius * a.sin()]
            }
            FlameVariation::Curl { c1, c2 } => {
                let re = 1.0 + c1 as f64 * x + c2 as f64 * (x * x - y * y);
                let im = c1 as f64 * y + 2.0 * c2 as f64 * x * y;
                let d = (re * re + im * im).max(1e-24);
                [(x * re + y * im) / d, (y * re - x * im) / d]
            }
        };
        result[0] += term.amount as f64 * v[0];
        result[1] += term.amount as f64 * v[1];
    }
    affine(t.post, result)
}

impl FlameAccumulator {
    pub fn new(canvas: &Canvas2D, seed: u64, width: u32, height: u32) -> Result<Self> {
        let flame = canvas
            .flame
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing flame"))?;
        flame.validate()?;
        let cells = u64::from(width) * u64::from(height) * u64::from(flame.supersampling).pow(2);
        if width == 0 || height == 0 || cells > 16_777_216 {
            bail!("flame histogram exceeds 512 MiB or has zero dimensions");
        }
        let weight = flame
            .transforms
            .iter()
            .map(|t| t.weight as f64)
            .sum::<f64>();
        let mut total = 0.0;
        let cdf = flame
            .transforms
            .iter()
            .map(|t| {
                total += t.weight as f64 / weight;
                total
            })
            .collect();
        Ok(Self {
            flame: flame.clone(),
            canvas: canvas.clone(),
            seed,
            width,
            height,
            histogram: (0..cells)
                .map(|_| std::array::from_fn(|_| AtomicU64::new(0)))
                .collect(),
            palette: (0..4096)
                .map(|i| flame.palette.sample(i as f32 / 4095.0).map(f64::from))
                .collect(),
            cdf,
            passes: 0,
        })
    }

    /// Each pass has immutable streams; batching and worker counts cannot change sums.
    pub fn advance(&mut self, passes: u32) {
        self.advance_with_cancel(passes, &AtomicBool::new(false));
    }

    pub fn advance_with_cancel(&mut self, passes: u32, cancelled: &AtomicBool) {
        let total = self.width as u64 * self.height as u64 * self.flame.points_per_sample as u64;
        let ss = self.flame.supersampling;
        let hw = self.width * ss;
        let hh = self.height * ss;
        let aspect = self.width as f64 / self.height as f64;
        let scale = self.canvas.zoom as f64 / 6.0;
        let (sn, cs) = (self.canvas.rotation as f64).to_radians().sin_cos();
        for pass in self.passes..self.passes + passes {
            (0..LANES).into_par_iter().for_each(|lane| {
                let count = total / LANES as u64 + u64::from((lane as u64) < total % LANES as u64);
                let mut rng = self.seed
                    ^ (pass as u64).wrapping_mul(0xd6e8feb86659fd93)
                    ^ (lane as u64).wrapping_mul(0xa0761d6478bd642f);
                let mut p = [random(&mut rng) * 2.0 - 1.0, random(&mut rng) * 2.0 - 1.0];
                let mut color = random(&mut rng);
                let mut warm = 64u64;
                let mut emitted = 0u64;
                while emitted < count {
                    if emitted & 1023 == 0 && cancelled.load(Ordering::Relaxed) {
                        return;
                    }
                    let choice = random(&mut rng);
                    let index = self
                        .cdf
                        .iter()
                        .position(|v| choice < *v)
                        .unwrap_or(self.cdf.len() - 1);
                    let t = &self.flame.transforms[index];
                    p = transform(t, p, &mut rng);
                    color += (t.color as f64 - color) * t.color_speed as f64;
                    if !p.iter().all(|v| v.is_finite() && v.abs() < 1e8) {
                        p = [random(&mut rng) * 2.0 - 1.0, random(&mut rng) * 2.0 - 1.0];
                        warm = 64;
                        // Diverging artistic formulas must still finish their finite budget.
                        emitted += 1;
                        continue;
                    }
                    if warm > 0 {
                        warm -= 1;
                        continue;
                    }
                    emitted += 1;
                    let q = self
                        .flame
                        .final_transform
                        .as_ref()
                        .map_or(p, |t| transform(t, p, &mut rng));
                    let dx = q[0] - self.canvas.center[0] as f64;
                    let dy = q[1] - self.canvas.center[1] as f64;
                    let fx = ((cs * dx + sn * dy) * scale + 0.5) * hw as f64 - 0.5;
                    let fy = (0.5 - (-sn * dx + cs * dy) * scale * aspect) * hh as f64 - 0.5;
                    if !fx.is_finite()
                        || !fy.is_finite()
                        || fx < 0.0
                        || fy < 0.0
                        || fx >= hw as f64 - 1.0
                        || fy >= hh as f64 - 1.0
                    {
                        continue;
                    }
                    let x = fx.floor() as u32;
                    let y = fy.floor() as u32;
                    let tx = fx - x as f64;
                    let ty = fy - y as f64;
                    let w0 = ((1.0 - tx) * (1.0 - ty) * UNIT as f64).floor() as u64;
                    let w1 = (tx * (1.0 - ty) * UNIT as f64).floor() as u64;
                    let w2 = ((1.0 - tx) * ty * UNIT as f64).floor() as u64;
                    let w3 = UNIT.saturating_sub(w0 + w1 + w2);
                    let position = color.clamp(0.0, 1.0) * 4095.0;
                    let ci = (position.floor() as usize).min(4094);
                    let cf = position - ci as f64;
                    let rgb = std::array::from_fn::<_, 3, _>(|c| {
                        self.palette[ci][c] * (1.0 - cf) + self.palette[ci + 1][c] * cf
                    });
                    for (ox, oy, w) in [(0, 0, w0), (1, 0, w1), (0, 1, w2), (1, 1, w3)] {
                        let cell = &self.histogram[((y + oy) * hw + x + ox) as usize];
                        cell[0].fetch_add(w, Ordering::Relaxed);
                        for c in 0..3 {
                            cell[c + 1]
                                .fetch_add((rgb[c] * w as f64).round() as u64, Ordering::Relaxed);
                        }
                    }
                }
            });
            if cancelled.load(Ordering::Relaxed) {
                return;
            }
        }
        self.passes += passes;
    }

    pub fn reconstruct(&self) -> HdrImage {
        let ss = self.flame.supersampling;
        let hw = self.width * ss;
        let moments: Vec<[f64; 4]> = (0..self.width as usize * self.height as usize)
            .into_par_iter()
            .map(|i| {
                let x = i as u32 % self.width;
                let y = i as u32 / self.width;
                let mut result = [0.0; 4];
                for oy in 0..ss {
                    for ox in 0..ss {
                        let p = &self.histogram[((y * ss + oy) * hw + x * ss + ox) as usize];
                        for c in 0..4 {
                            result[c] += p[c].load(Ordering::Relaxed) as f64 / UNIT as f64;
                        }
                    }
                }
                result
            })
            .collect();
        let scale = (self.passes as f64 * self.flame.points_per_sample as f64).max(1.0);
        let filtered =
            reconstruct_moments(&moments, self.width, self.height, self.flame.filter_radius);
        let data = filtered
            .into_par_iter()
            .map(|p| {
                let density = p[0] / scale;
                let compression = if self.flame.density_log > 0.0 {
                    (density * self.flame.density_log as f64).ln_1p()
                        / (self.flame.density_log as f64).ln_1p()
                } else {
                    density
                };
                let mut rgb = std::array::from_fn::<_, 3, _>(|c| {
                    (self.flame.background[c] as f64
                        + self.flame.brightness as f64 * compression * p[c + 1] / p[0].max(1e-20))
                        as f32
                });
                if !rgb.iter().all(|v| v.is_finite()) {
                    rgb = [0.0; 3];
                }
                [rgb[0], rgb[1], rgb[2], 1.0]
            })
            .collect();
        HdrImage::from_data(self.width, self.height, data)
    }
}

/// Positive normalized kernels share weights for mass and all colour moments.
/// The density structure tensor selects the tangent; no hue enters the kernel.
fn reconstruct_moments(
    moments: &[[f64; 4]],
    width: u32,
    height: u32,
    radius: f32,
) -> Vec<[f64; 4]> {
    let w = width as i32;
    let h = height as i32;
    let read = |x: i32, y: i32| moments[(y.clamp(0, h - 1) * w + x.clamp(0, w - 1)) as usize];
    (0..moments.len())
        .into_par_iter()
        .map(|i| {
            let x = i as i32 % w;
            let y = i as i32 / w;
            let mut tensor = [0.0; 3];
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let gx =
                        read(x + dx + 1, y + dy)[0].ln_1p() - read(x + dx - 1, y + dy)[0].ln_1p();
                    let gy =
                        read(x + dx, y + dy + 1)[0].ln_1p() - read(x + dx, y + dy - 1)[0].ln_1p();
                    tensor[0] += gx * gx;
                    tensor[1] += gx * gy;
                    tensor[2] += gy * gy;
                }
            }
            let angle = 0.5 * (2.0 * tensor[1]).atan2(tensor[0] - tensor[2]);
            let (ny, nx) = angle.sin_cos();
            let coherence = ((tensor[0] - tensor[2]).hypot(2.0 * tensor[1])
                / (tensor[0] + tensor[2] + 1e-12))
                .clamp(0.0, 1.0);
            let noise = (6.0 / moments[i][0].sqrt().max(1.0)).clamp(0.0, 1.0);
            let long = 0.42 + (radius as f64 - 0.42).max(0.0) * noise;
            let short = long * (1.0 - coherence) + 0.42 * coherence;
            let support = (long * 2.5).ceil() as i32;
            let mut sum = [0.0; 4];
            let mut mass = 0.0;
            for dy in -support..=support {
                for dx in -support..=support {
                    if x + dx < 0 || x + dx >= w || y + dy < 0 || y + dy >= h {
                        continue;
                    }
                    let dn = dx as f64 * nx + dy as f64 * ny;
                    let dt = -dx as f64 * ny + dy as f64 * nx;
                    let r2 = dn * dn / (short * short) + dt * dt / (long * long);
                    if r2 > 6.25 {
                        continue;
                    }
                    let weight = (-0.5 * r2).exp();
                    let p = read(x + dx, y + dy);
                    for c in 0..4 {
                        sum[c] += weight * p[c];
                    }
                    mass += weight;
                }
            }
            sum.map(|v| v / mass.max(1e-20))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn orbit_sums_are_bit_identical_across_workers_and_pass_chunking() {
        let mut canvas = Canvas2D::default();
        canvas.flame = Some(Flame::default());
        let run = |threads, batch| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| {
                    let mut a = FlameAccumulator::new(&canvas, 17, 32, 24).unwrap();
                    if batch {
                        a.advance(3)
                    } else {
                        for _ in 0..3 {
                            a.advance(1)
                        }
                    }
                    a.histogram
                        .iter()
                        .map(|p| p.each_ref().map(|v| v.load(Ordering::Relaxed)))
                        .collect::<Vec<_>>()
                })
        };
        assert_eq!(run(1, true), run(4, false));
    }
    #[test]
    fn shared_moment_filter_keeps_chroma_and_a_one_pixel_filament() {
        let (w, h) = (33, 33);
        let mut p = vec![[0.0; 4]; w * h];
        for y in 0..h {
            p[y * w + 16] = [100.0, 100.0, 25.0, 2.0];
        }
        let q = reconstruct_moments(&p, w as u32, h as u32, 2.0);
        assert!(q[16 * w + 16][0] > 80.0, "thin filament erased");
        for v in q {
            if v[0] > 1e-10 {
                assert!((v[2] / v[0] - 0.25).abs() < 1e-12);
                assert!((v[3] / v[0] - 0.02).abs() < 1e-12);
            }
        }
    }
    #[test]
    fn all_variations_handle_the_origin_and_render_finite_values() {
        for variation in [
            FlameVariation::Linear,
            FlameVariation::Sinusoidal,
            FlameVariation::Spherical,
            FlameVariation::Swirl,
            FlameVariation::Horseshoe,
            FlameVariation::Disc,
            FlameVariation::Bubble,
            FlameVariation::Julian {
                power: 3,
                distance: 1.0,
            },
            FlameVariation::Curl { c1: 0.2, c2: -0.1 },
        ] {
            let t = FlameTransform {
                affine: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
                terms: vec![syfrack_core::FlameTerm {
                    amount: 1.0,
                    variation,
                }],
                ..Default::default()
            };
            assert!(transform(&t, [0.0; 2], &mut 7)
                .iter()
                .all(|v| v.is_finite()));
        }
        let mut c = Canvas2D::default();
        c.flame = Some(Flame::default());
        let mut a = FlameAccumulator::new(&c, 7, 24, 16).unwrap();
        a.advance(2);
        assert!(a.reconstruct().data.iter().flatten().all(|v| v.is_finite()));
    }
}
