use syfrack_core::{luminance, HdrImage};

/// Saturation scales chroma uniformly. Vibrance adds extra saturation to
/// colours that are not already saturated, protecting vivid ones.
pub fn apply(img: &mut HdrImage, saturation: f32, vibrance: f32) {
    img.map_rgb(|c| {
        let l = luminance(c);
        let max = c[0].max(c[1]).max(c[2]);
        let min = c[0].min(c[1]).min(c[2]);
        let current = if max > 1e-6 { (max - min) / max } else { 0.0 };
        let s = saturation * (1.0 + vibrance * (1.0 - current));
        c.map(|x| (l + (x - l) * s).max(0.0))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use syfrack_core::HdrImage;

    fn img(c: [f32; 3]) -> HdrImage {
        let mut i = HdrImage::new(1, 1);
        i.set(0, 0, [c[0], c[1], c[2], 1.0]);
        i
    }

    #[test]
    fn identity_at_one_and_zero() {
        let mut a = img([0.8, 0.2, 0.3]);
        apply(&mut a, 1.0, 0.0);
        assert_eq!(a.get(0, 0), [0.8, 0.2, 0.3, 1.0]);
    }

    #[test]
    fn zero_saturation_is_grey() {
        let mut a = img([0.8, 0.2, 0.3]);
        apply(&mut a, 0.0, 0.0);
        let p = a.get(0, 0);
        assert!((p[0] - p[1]).abs() < 1e-6 && (p[1] - p[2]).abs() < 1e-6);
    }

    #[test]
    fn vibrance_boosts_muted_colours_more_than_vivid_ones() {
        let mut muted = img([0.5, 0.45, 0.45]);
        let mut vivid = img([0.9, 0.1, 0.1]);
        let m0 = muted.get(0, 0);
        let v0 = vivid.get(0, 0);
        apply(&mut muted, 1.0, 1.0);
        apply(&mut vivid, 1.0, 1.0);
        let gain =
            |before: [f32; 4], after: [f32; 4]| (after[0] - after[1]) / (before[0] - before[1]);
        assert!(gain(m0, muted.get(0, 0)) > gain(v0, vivid.get(0, 0)));
    }
}
