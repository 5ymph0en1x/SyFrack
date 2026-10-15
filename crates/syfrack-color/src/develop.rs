use syfrack_core::{HdrImage, Look};

use crate::encode::{encode, EncodedImage};
use crate::{balance, exposure, grade, grain, saturation, tonemap, vignette};

/// Runs the look on a scene-linear image. Output is display-linear in [0, 1].
pub fn develop(hdr: &HdrImage, look: &Look) -> HdrImage {
    let mut img = hdr.clone();
    let mut ev = look.exposure_ev;
    if let Some(target) = look.auto_exposure {
        ev += exposure::auto_ev(&img, target);
    }
    exposure::apply(&mut img, ev);
    balance::apply(&mut img, look.temperature, look.tint);
    grade::apply(&mut img, &look.shadows, &look.midtones, &look.highlights);
    tonemap::apply(&mut img, &look.tonemap);
    saturation::apply(&mut img, look.saturation, look.vibrance);
    if look.neon_softness > 0.0 {
        img.map_rgb(|c| crate::neon::soften(c, look.neon_softness));
    }
    if let Some(v) = &look.vignette {
        vignette::apply(&mut img, v);
    }
    if let Some(g) = &look.grain {
        grain::apply(&mut img, g);
    }
    img
}

pub fn develop_and_encode(hdr: &HdrImage, look: &Look, bits: u8) -> EncodedImage {
    let img = develop(hdr, look);
    encode(&img, look.output, bits, look.dither)
}

#[cfg(test)]
mod tests {
    use super::*;
    use syfrack_core::{Grain, HdrImage, Look, OutputSpace, Vignette};

    fn ramp() -> HdrImage {
        let mut img = HdrImage::new(32, 4);
        for y in 0..4 {
            for x in 0..32 {
                let v = 0.001 * 1.3f32.powi(x as i32);
                img.set(x, y, [v, v * 0.9, v * 0.8, 1.0]);
            }
        }
        img
    }

    #[test]
    fn neutral_look_yields_bounded_display_linear() {
        let out = develop(&ramp(), &Look::neutral());
        assert_eq!((out.width, out.height), (32, 4));
        assert!(out
            .data
            .iter()
            .all(|p| p[..3].iter().all(|c| (0.0..=1.0).contains(c))));
        assert!(
            out.get(31, 0)[0] > out.get(0, 0)[0],
            "brighter in stays brighter out"
        );
    }

    #[test]
    fn auto_exposure_makes_different_exposures_converge() {
        let mut look = Look::neutral();
        look.auto_exposure = Some(0.18);
        let dark = develop(&ramp(), &look);
        let mut bright_in = ramp();
        bright_in.map_rgb(|c| [c[0] * 16.0, c[1] * 16.0, c[2] * 16.0]);
        let bright = develop(&bright_in, &look);
        for (a, b) in dark.data.iter().zip(&bright.data) {
            assert!((a[1] - b[1]).abs() < 1e-3);
        }
    }

    #[test]
    fn every_optional_stage_runs_and_encodes() {
        let mut look = Look::neutral();
        look.vignette = Some(Vignette {
            radius: 0.5,
            softness: 0.5,
            intensity: 0.3,
            elliptical: true,
        });
        look.grain = Some(Grain {
            amount: 0.1,
            size: 1.0,
            seed: 9,
        });
        look.output = OutputSpace::DisplayP3;
        let enc = develop_and_encode(&ramp(), &look, 16);
        assert_eq!(enc.data.len(), 32 * 4 * 3);
        assert_eq!(enc.bits, 16);
    }
}
