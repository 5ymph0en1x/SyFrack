use syfrack_core::{HdrImage, Vignette};

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Darkens toward the edges. Coordinates are normalised so that 1 reaches the
/// middle of the short edge; `elliptical` stretches the falloff to the frame.
pub fn apply(img: &mut HdrImage, v: &Vignette) {
    if v.intensity <= 0.0 {
        return;
    }
    let (w, h) = (img.width as f32, img.height as f32);
    let aspect = w / h;
    for y in 0..img.height {
        for x in 0..img.width {
            let mut u = ((x as f32 + 0.5) / w - 0.5) * 2.0;
            let mut t = ((y as f32 + 0.5) / h - 0.5) * 2.0;
            if !v.elliptical {
                if aspect >= 1.0 {
                    u *= aspect;
                } else {
                    t /= aspect;
                }
            }
            let r = (u * u + t * t).sqrt();
            let f = smoothstep(v.radius, v.radius + v.softness.max(1e-3), r);
            let k = 1.0 - v.intensity * f;
            let mut p = img.get(x, y);
            p[0] *= k;
            p[1] *= k;
            p[2] *= k;
            img.set(x, y, p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syfrack_core::{HdrImage, Vignette};

    fn white(w: u32, h: u32) -> HdrImage {
        let mut img = HdrImage::new(w, h);
        img.map_rgb(|_| [1.0, 1.0, 1.0]);
        img
    }

    #[test]
    fn centre_untouched_corner_darker() {
        let mut img = white(64, 32);
        apply(
            &mut img,
            &Vignette {
                radius: 0.5,
                softness: 0.5,
                intensity: 0.8,
                elliptical: false,
            },
        );
        assert!((img.get(32, 16)[0] - 1.0).abs() < 1e-4);
        assert!(img.get(0, 0)[0] < 0.5);
        assert!(
            img.get(0, 0)[0] >= 0.2 - 1e-4,
            "never darker than 1 - intensity"
        );
    }

    #[test]
    fn zero_intensity_is_identity() {
        let mut img = white(16, 16);
        apply(
            &mut img,
            &Vignette {
                radius: 0.5,
                softness: 0.5,
                intensity: 0.0,
                elliptical: true,
            },
        );
        assert_eq!(img, white(16, 16));
    }

    #[test]
    fn circular_vignette_darkens_short_edge_midpoint_less_than_long_edge_midpoint() {
        let mut img = white(64, 32);
        apply(
            &mut img,
            &Vignette {
                radius: 0.3,
                softness: 1.0,
                intensity: 1.0,
                elliptical: false,
            },
        );
        let long_edge_mid = img.get(63, 16)[0];
        let short_edge_mid = img.get(32, 0)[0];
        assert!(
            long_edge_mid < short_edge_mid,
            "{long_edge_mid} vs {short_edge_mid}"
        );
    }
}
