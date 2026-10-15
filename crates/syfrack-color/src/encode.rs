use syfrack_core::{HdrImage, OutputSpace};

use crate::dither::blue_noise;

#[derive(Clone, Debug, PartialEq)]
pub struct EncodedImage {
    pub width: u32,
    pub height: u32,
    /// 8 or 16.
    pub bits: u8,
    /// RGB or straight RGBA interleaved, row-major, values in 0..=2^bits - 1.
    /// Alpha is linear coverage, never sRGB-encoded or dithered.
    pub data: Vec<u16>,
}

impl EncodedImage {
    pub fn has_alpha(&self) -> bool {
        self.data.len() == self.width as usize * self.height as usize * 4
    }
}

pub fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

pub fn linear_to_srgb(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

// Linear sRGB (Rec. 709 primaries) to linear Display P3, D65.
pub(crate) const SRGB_TO_P3: [[f32; 3]; 3] = [
    [0.822_461_97, 0.177_538_03, 0.0],
    [0.033_194_2, 0.966_805_8, 0.0],
    [0.017_082_63, 0.072_397_44, 0.910_519_9],
];

fn to_space(c: [f32; 3], space: OutputSpace) -> [f32; 3] {
    match space {
        OutputSpace::Srgb => c,
        OutputSpace::DisplayP3 => {
            let m = &SRGB_TO_P3;
            [
                m[0][0] * c[0] + m[0][1] * c[1] + m[0][2] * c[2],
                m[1][0] * c[0] + m[1][1] * c[1] + m[1][2] * c[2],
                m[2][0] * c[0] + m[2][1] * c[1] + m[2][2] * c[2],
            ]
        }
    }
}

/// Converts display-linear [0, 1] to the output primaries, applies the sRGB
/// transfer (Display P3 uses the same curve) and quantises, optionally with
/// blue-noise dithering so smooth gradients do not band.
pub fn encode(img: &HdrImage, space: OutputSpace, bits: u8, dither: bool) -> EncodedImage {
    assert!(bits == 8 || bits == 16, "bits must be 8 or 16");
    let max = ((1u32 << bits) - 1) as f32;
    let mask = blue_noise();
    let has_alpha = img.data.iter().any(|p| p[3] != 1.0);
    let mut data = Vec::with_capacity(img.pixel_count() * if has_alpha { 4 } else { 3 });
    for y in 0..img.height {
        for x in 0..img.width {
            let p = img.get(x, y);
            let alpha = if p[3].is_finite() {
                p[3].clamp(0.0, 1.0)
            } else {
                0.0
            };
            let alpha_code = (alpha * max).round() as u16;
            let c = to_space([p[0], p[1], p[2]], space);
            let noise = if dither { mask.at(x, y) - 0.5 } else { 0.0 };
            for &channel in &c {
                let v = linear_to_srgb(channel) * max + noise;
                data.push(if has_alpha && alpha_code == 0 {
                    0
                } else {
                    v.round().clamp(0.0, max) as u16
                });
            }
            if has_alpha {
                data.push(alpha_code);
            }
        }
    }
    EncodedImage {
        width: img.width,
        height: img.height,
        bits,
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syfrack_core::{HdrImage, OutputSpace};

    #[test]
    fn srgb_transfer_is_standard() {
        assert!((linear_to_srgb(0.0)).abs() < 1e-6);
        assert!((linear_to_srgb(1.0) - 1.0).abs() < 1e-6);
        assert!((linear_to_srgb(0.18) - 0.4613).abs() < 1e-3);
        assert!(
            (linear_to_srgb(0.002) - 0.02584).abs() < 1e-4,
            "linear segment"
        );
    }

    #[test]
    fn rgba_keeps_straight_colour_and_linear_undithered_alpha() {
        let image = HdrImage::from_data(
            3,
            1,
            vec![
                [1.0, 0.0, 0.0, 0.5],
                [0.0, 40.0, 0.0, 0.0],
                [0.5, 0.5, 0.5, 1.0],
            ],
        );
        for bits in [8, 16] {
            let clean = encode(&image, OutputSpace::Srgb, bits, false);
            let dithered = encode(&image, OutputSpace::Srgb, bits, true);
            let max = ((1u32 << bits) - 1) as u16;
            assert!(clean.has_alpha());
            assert_eq!(&clean.data[..4], &[max, 0, 0, max / 2 + 1]);
            assert_eq!(&dithered.data[4..8], &[0, 0, 0, 0]);
            assert_eq!(
                clean.data.iter().skip(3).step_by(4).collect::<Vec<_>>(),
                dithered.data.iter().skip(3).step_by(4).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn black_and_white_hit_the_rails() {
        let mut img = HdrImage::new(2, 1);
        img.set(1, 0, [1.0, 1.0, 1.0, 1.0]);
        let e8 = encode(&img, OutputSpace::Srgb, 8, false);
        assert_eq!(&e8.data[..3], &[0, 0, 0]);
        assert_eq!(&e8.data[3..], &[255, 255, 255]);
        let e16 = encode(&img, OutputSpace::Srgb, 16, true);
        assert_eq!(&e16.data[3..], &[65535, 65535, 65535]);
        assert_eq!(e16.bits, 16);
    }

    #[test]
    fn dithering_preserves_the_mean_and_uses_both_codes() {
        let mut img = HdrImage::new(64, 64);
        img.map_rgb(|_| [0.5, 0.5, 0.5]);
        let e = encode(&img, OutputSpace::Srgb, 8, true);
        let codes: Vec<u16> = e.data.iter().step_by(3).copied().collect();
        let mean = codes.iter().map(|c| *c as f32).sum::<f32>() / codes.len() as f32;
        let exact = linear_to_srgb(0.5) * 255.0;
        assert!((mean - exact).abs() < 0.1, "mean {mean} vs {exact}");
        assert!(codes.contains(&(exact.floor() as u16)) && codes.contains(&(exact.ceil() as u16)));
    }

    #[test]
    fn p3_keeps_greys_neutral_and_widens_pure_red() {
        let mut img = HdrImage::new(2, 1);
        img.set(0, 0, [0.5, 0.5, 0.5, 1.0]);
        img.set(1, 0, [1.0, 0.0, 0.0, 1.0]);
        let e = encode(&img, OutputSpace::DisplayP3, 16, false);
        assert!(
            (e.data[0] as i32 - e.data[1] as i32).abs() <= 1
                && (e.data[1] as i32 - e.data[2] as i32).abs() <= 1
        );
        assert!(
            e.data[3] < 65535 && e.data[4] > 0,
            "sRGB red is inside P3: {:?}",
            &e.data[3..6]
        );
    }
}
