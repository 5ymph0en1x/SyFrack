//! Shared SDR image import and non-destructive mosaic image adjustments.
use std::path::Path;

use anyhow::{ensure, Context, Result};
use image::{imageops::FilterType, ImageReader};
use syfrack_core::{luminance, HdrImage};

use crate::encode::srgb_to_linear;

/// Import bounded sRGB files, apply EXIF orientation and resize in associated
/// linear light. Hidden RGB never bleeds into transparent edges.
pub fn read_srgb_image(path: &Path, size: u32) -> Result<HdrImage> {
    ensure!((1..=4096).contains(&size), "invalid source preview size");
    let mut reader = ImageReader::open(path)
        .context("open image")?
        .with_guessed_format()?;
    ensure!(
        matches!(
            reader.format(),
            Some(
                image::ImageFormat::Png
                    | image::ImageFormat::Jpeg
                    | image::ImageFormat::Bmp
                    | image::ImageFormat::Tiff
            )
        ),
        "image must be PNG, JPEG, BMP or TIFF"
    );
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(512 * 1024 * 1024);
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().context("decode image")?;
    let orientation = image::ImageDecoder::orientation(&mut decoder)?;
    let mut source = image::DynamicImage::from_decoder(decoder)?;
    source.apply_orientation(orientation);
    let mut linear = source.into_rgba32f();
    for p in linear.pixels_mut() {
        let a = p.0[3].clamp(0.0, 1.0);
        *p = image::Rgba([
            srgb_to_linear(p.0[0]) * a,
            srgb_to_linear(p.0[1]) * a,
            srgb_to_linear(p.0[2]) * a,
            a,
        ]);
    }
    from_associated(resize(linear, size))
}

fn resize(image: image::Rgba32FImage, size: u32) -> image::Rgba32FImage {
    let longest = image.width().max(image.height());
    if longest <= size {
        return image;
    }
    let w = (u64::from(image.width()) * u64::from(size) / u64::from(longest)).max(1) as u32;
    let h = (u64::from(image.height()) * u64::from(size) / u64::from(longest)).max(1) as u32;
    image::imageops::resize(&image, w, h, FilterType::Triangle)
}

fn from_associated(image: image::Rgba32FImage) -> Result<HdrImage> {
    let mut data = Vec::with_capacity((image.width() * image.height()) as usize);
    for p in image.pixels() {
        ensure!(
            p.0.iter().all(|v| v.is_finite()),
            "image contains non-finite colour values"
        );
        let a = p.0[3].clamp(0.0, 1.0);
        let mut colour = [0.0, 0.0, 0.0, a];
        if a > 0.0 {
            for (c, v) in colour[..3].iter_mut().enumerate() {
                *v = (p.0[c] / a).clamp(0.0, 1.0);
            }
        }
        data.push(colour);
    }
    Ok(HdrImage::from_data(image.width(), image.height(), data))
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ImageAdjustments {
    /// Left, top, right, bottom in normalised original-image coordinates.
    pub crop: [f32; 4],
    /// Clockwise quarter turns, after cropping.
    pub rotation: u32,
    pub exposure: f32,
    pub contrast: f32,
    pub saturation: f32,
}

impl Default for ImageAdjustments {
    fn default() -> Self {
        Self {
            crop: [0.0, 0.0, 1.0, 1.0],
            rotation: 0,
            exposure: 0.0,
            contrast: 1.0,
            saturation: 1.0,
        }
    }
}

pub fn adjust_image(image: &HdrImage, settings: &ImageAdjustments, size: u32) -> Result<HdrImage> {
    let [left, top, right, bottom] = settings.crop;
    ensure!(
        (1..=4096).contains(&size)
            && image.width > 0
            && image.height > 0
            && image.data.len() as u64 == u64::from(image.width) * u64::from(image.height),
        "invalid image dimensions"
    );
    ensure!(
        settings
            .crop
            .iter()
            .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
            && left < right
            && top < bottom,
        "invalid image crop"
    );
    ensure!(
        settings.rotation < 4
            && settings.exposure.is_finite()
            && (-4.0..=4.0).contains(&settings.exposure)
            && settings.contrast.is_finite()
            && (0.25..=3.0).contains(&settings.contrast)
            && settings.saturation.is_finite()
            && (0.0..=3.0).contains(&settings.saturation),
        "invalid image adjustments"
    );
    let x0 = ((left * image.width as f32).floor() as u32).min(image.width - 1);
    let y0 = ((top * image.height as f32).floor() as u32).min(image.height - 1);
    let x1 = ((right * image.width as f32).ceil().min(image.width as f32) as u32).max(x0 + 1);
    let y1 = (bottom * image.height as f32)
        .ceil()
        .min(image.height as f32) as u32;
    let y1 = y1.max(y0 + 1);
    let (w, h) = (x1 - x0, y1 - y0);
    let (width, height) = if settings.rotation % 2 == 0 {
        (w, h)
    } else {
        (h, w)
    };
    let mut output = image::Rgba32FImage::new(width, height);
    let exposure = settings.exposure.exp2();
    for y in 0..height {
        for x in 0..width {
            let (sx, sy) = match settings.rotation {
                1 => (y, h - 1 - x),
                2 => (w - 1 - x, h - 1 - y),
                3 => (w - 1 - y, x),
                _ => (x, y),
            };
            let p = image.get(x0 + sx, y0 + sy);
            let l = luminance([p[0], p[1], p[2]]);
            let alpha = p[3];
            let c = std::array::from_fn::<_, 3, _>(|i| {
                let saturated = if settings.saturation == 1.0 {
                    p[i]
                } else {
                    l + (p[i] - l) * settings.saturation
                };
                let exposed = saturated * exposure;
                let contrast = if settings.contrast == 1.0 {
                    exposed
                } else {
                    (exposed - 0.18) * settings.contrast + 0.18
                };
                contrast.clamp(0.0, 1.0) * alpha
            });
            output.put_pixel(x, y, image::Rgba([c[0], c[1], c[2], alpha]));
        }
    }
    from_associated(resize(output, size))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crop_and_clockwise_rotation_preserve_pixel_order() {
        let source = HdrImage::from_data(
            3,
            2,
            (0..6).map(|i| [i as f32 / 8.0, 0.0, 0.0, 1.0]).collect(),
        );
        let settings = ImageAdjustments {
            crop: [1.0 / 3.0, 0.0, 1.0, 1.0],
            rotation: 1,
            ..Default::default()
        };
        let result = adjust_image(&source, &settings, 100).unwrap();
        assert_eq!((result.width, result.height), (2, 2));
        assert_eq!(
            result.data.iter().map(|p| p[0]).collect::<Vec<_>>(),
            [0.5, 0.125, 0.625, 0.25]
        );
        assert_eq!(source.width, 3);
    }
    #[test]
    fn adjustments_are_non_destructive_and_preserve_alpha() {
        let source = HdrImage::from_data(1, 1, vec![[0.4, 0.2, 0.1, 0.5]]);
        let result = adjust_image(
            &source,
            &ImageAdjustments {
                exposure: 1.0,
                saturation: 0.0,
                ..Default::default()
            },
            100,
        )
        .unwrap();
        assert!((result.data[0][0] - luminance([0.4, 0.2, 0.1]) * 2.0).abs() < 1e-6);
        assert_eq!(result.data[0][0], result.data[0][1]);
        assert_eq!(result.data[0][3], 0.5);
        assert_eq!(source.data[0], [0.4, 0.2, 0.1, 0.5]);
        assert!(adjust_image(
            &source,
            &ImageAdjustments {
                crop: [0.5, 0.0, 0.5, 1.0],
                ..Default::default()
            },
            100
        )
        .is_err());
    }
    #[test]
    fn resizing_does_not_reveal_hidden_rgb() {
        let source = HdrImage::from_data(2, 1, vec![[1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 1.0, 1.0]]);
        let result = adjust_image(&source, &ImageAdjustments::default(), 1).unwrap();
        assert_eq!(result.width, 1);
        assert!(result.data[0][0] < 1e-6);
        assert!((result.data[0][2] - 1.0).abs() < 1e-6);
        assert!((result.data[0][3] - 0.5).abs() < 1e-6);
    }
}
