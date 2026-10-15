use syfrack_core::{luminance, Grain, HdrImage};

fn pcg(v: u32) -> u32 {
    let state = v.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let word = ((state >> ((state >> 28).wrapping_add(4))) ^ state).wrapping_mul(277_803_737);
    (word >> 22) ^ word
}

fn hash3(x: u32, y: u32, seed: u64) -> u32 {
    let mut h = pcg(seed as u32 ^ pcg((seed >> 32) as u32));
    h = pcg(h ^ x);
    pcg(h ^ y)
}

fn unit(h: u32) -> f32 {
    (h >> 8) as f32 / 16_777_216.0
}

/// Standard normal sample from two hashes (Box-Muller).
fn gaussian(h: u32) -> f32 {
    let u1 = unit(h).max(1e-7);
    let u2 = unit(pcg(h));
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f32::consts::PI * u2).cos()
}

/// Luminance-weighted film grain, strongest in midtones (4 l (1 - l)), with a
/// fixed seed so the result is reproducible. Expects display-linear input.
pub fn apply(img: &mut HdrImage, g: &Grain) {
    if g.amount <= 0.0 {
        return;
    }
    let cell = g.size.max(0.5);
    for y in 0..img.height {
        for x in 0..img.width {
            let cx = (x as f32 / cell).floor() as u32;
            let cy = (y as f32 / cell).floor() as u32;
            let n = gaussian(hash3(cx, cy, g.seed));
            let mut p = img.get(x, y);
            let l = luminance([p[0], p[1], p[2]]).clamp(0.0, 1.0);
            let w = 4.0 * l * (1.0 - l);
            let delta = n * g.amount * w * 0.25;
            for c in &mut p[..3] {
                *c = (*c + delta).clamp(0.0, 1.0);
            }
            img.set(x, y, p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syfrack_core::{Grain, HdrImage};

    fn grey(w: u32, h: u32, v: f32) -> HdrImage {
        let mut img = HdrImage::new(w, h);
        img.map_rgb(|_| [v, v, v]);
        img
    }

    #[test]
    fn zero_amount_is_identity() {
        let mut img = grey(8, 8, 0.5);
        apply(
            &mut img,
            &Grain {
                amount: 0.0,
                size: 1.0,
                seed: 1,
            },
        );
        assert_eq!(img, grey(8, 8, 0.5));
    }

    #[test]
    fn grain_is_zero_mean_and_deterministic() {
        let mut a = grey(128, 128, 0.5);
        let mut b = grey(128, 128, 0.5);
        let g = Grain {
            amount: 0.2,
            size: 1.0,
            seed: 42,
        };
        apply(&mut a, &g);
        apply(&mut b, &g);
        assert_eq!(a, b);
        let mean: f32 = a.data.iter().map(|p| p[1]).sum::<f32>() / a.data.len() as f32;
        assert!((mean - 0.5).abs() < 0.005, "mean {mean}");
        let var: f32 =
            a.data.iter().map(|p| (p[1] - 0.5).powi(2)).sum::<f32>() / a.data.len() as f32;
        assert!(var > 1e-5, "grain must actually do something");
    }

    #[test]
    fn grain_is_weaker_in_blacks_and_whites() {
        let g = Grain {
            amount: 0.2,
            size: 1.0,
            seed: 7,
        };
        let spread = |v: f32| {
            let mut img = grey(64, 64, v);
            apply(&mut img, &g);
            img.data.iter().map(|p| (p[1] - v).abs()).sum::<f32>() / img.data.len() as f32
        };
        assert!(spread(0.5) > 2.0 * spread(0.02));
        assert!(spread(0.5) > 2.0 * spread(0.98));
    }

    #[test]
    fn larger_size_makes_neighbouring_pixels_equal() {
        let mut img = grey(8, 8, 0.5);
        apply(
            &mut img,
            &Grain {
                amount: 0.2,
                size: 4.0,
                seed: 3,
            },
        );
        assert_eq!(img.get(0, 0), img.get(3, 3));
        assert_ne!(img.get(0, 0), img.get(4, 4));
    }
}
