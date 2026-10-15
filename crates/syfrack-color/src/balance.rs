use syfrack_core::{luminance, Color, HdrImage};

/// White balance as an artist sees it: `temperature` is the colour the image
/// takes on (lower = warmer), `tint` > 0 pushes magenta, < 0 green.
/// Luminance of neutral greys is preserved.
pub fn apply(img: &mut HdrImage, temperature: f32, tint: f32) {
    let gain = gains(temperature, tint);
    img.map_rgb(|c| [c[0] * gain[0], c[1] * gain[1], c[2] * gain[2]]);
}

/// The linear RGB gain that [`apply`] multiplies every pixel by.
pub fn gains(temperature: f32, tint: f32) -> [f32; 3] {
    let reference = Color::Kelvin(6500.0).to_linear();
    let target = Color::Kelvin(temperature).to_linear();
    let mut gain = [
        target[0] / reference[0],
        target[1] / reference[1],
        target[2] / reference[2],
    ];
    gain[1] *= 1.0 - 0.3 * tint;
    let l = luminance(gain).max(1e-6);
    for g in &mut gain {
        *g /= l;
    }
    gain
}

#[cfg(test)]
mod tests {
    use super::*;
    use syfrack_core::{luminance, HdrImage};

    fn grey() -> HdrImage {
        let mut img = HdrImage::new(2, 2);
        img.map_rgb(|_| [0.5, 0.5, 0.5]);
        img
    }

    #[test]
    fn neutral_settings_are_identity() {
        let mut img = grey();
        apply(&mut img, 6500.0, 0.0);
        let p = img.get(0, 0);
        assert!(
            (p[0] - 0.5).abs() < 1e-5 && (p[1] - 0.5).abs() < 1e-5 && (p[2] - 0.5).abs() < 1e-5
        );
    }

    #[test]
    fn low_temperature_warms_and_keeps_luminance() {
        let mut img = grey();
        apply(&mut img, 3000.0, 0.0);
        let p = img.get(0, 0);
        assert!(p[0] > p[2], "warm means red over blue: {p:?}");
        assert!((luminance([p[0], p[1], p[2]]) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn positive_tint_is_magenta() {
        let mut img = grey();
        apply(&mut img, 6500.0, 0.5);
        let p = img.get(0, 0);
        assert!(p[1] < p[0] && p[1] < p[2]);
    }

    #[test]
    fn gains_match_apply() {
        let g = gains(3200.0, 0.2);
        let mut img = HdrImage::new(1, 1);
        img.set(0, 0, [1.0, 1.0, 1.0, 1.0]);
        apply(&mut img, 3200.0, 0.2);
        let p = img.get(0, 0);
        assert!(
            (p[0] - g[0]).abs() < 1e-6 && (p[1] - g[1]).abs() < 1e-6 && (p[2] - g[2]).abs() < 1e-6
        );
        assert!((luminance(gains(6500.0, 0.0)) - 1.0).abs() < 1e-5);
    }
}
