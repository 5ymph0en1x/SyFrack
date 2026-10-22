use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::Path;

use anyhow::{Context, Result};
use exr::prelude::*;
use syfrack_core::{HdrImage, OutputSpace};

use crate::encode::EncodedImage;

pub const SCENE_KEYWORD: &str = "syfrack.scene";

/// Writes associated linear RGBA, as required by OpenEXR compositing.
/// Zero-alpha additive emission is preserved because it cannot be associated.
pub fn write_exr(path: impl AsRef<Path>, img: &HdrImage) -> Result<()> {
    let mut image = Image::from_channels(
        (img.width as usize, img.height as usize),
        SpecificChannels::rgba(|Vec2(x, y)| {
            let p = img.get(x as u32, y as u32);
            let a = if p[3] > 0.0 { p[3] } else { 1.0 };
            (p[0] * a, p[1] * a, p[2] * a, p[3])
        }),
    );
    // Retain parallel lossless compression, but emit blocks in a canonical
    // order. Unspecified order writes them as compression threads finish and
    // produces different file bytes from identical HDR pixels.
    image.layer_data.encoding.line_order = LineOrder::Increasing;
    image.write().to_file(path.as_ref()).context("write exr")?;
    Ok(())
}

pub fn read_exr(path: impl AsRef<Path>) -> Result<HdrImage> {
    let image = read_first_rgba_layer_from_file(
        path.as_ref(),
        |resolution, _channels| {
            HdrImage::new(resolution.width() as u32, resolution.height() as u32)
        },
        |img, pos, (r, g, b, a): (f32, f32, f32, f32)| {
            let divisor = if a > 0.0 { a } else { 1.0 };
            img.set(
                pos.x() as u32,
                pos.y() as u32,
                [r / divisor, g / divisor, b / divisor, a],
            );
        },
    )
    .context("read exr")?;
    Ok(image.layer_data.channel_data.pixels)
}

/// Writes RGB/straight RGBA PNG with scene RON. Latin-1 uses legacy tEXt;
/// Unicode uses the standard UTF-8 iTXt chunk, without changing scene names.
pub fn write_png(
    path: impl AsRef<Path>,
    img: &EncodedImage,
    space: OutputSpace,
    scene_ron: Option<&str>,
) -> Result<()> {
    anyhow::ensure!(img.bits == 8 || img.bits == 16, "PNG depth must be 8 or 16");
    let channels = if img.has_alpha() { 4 } else { 3 };
    anyhow::ensure!(
        img.data.len() == img.width as usize * img.height as usize * channels,
        "PNG data must contain RGB or RGBA pixels"
    );
    let file = File::create(path.as_ref()).context("create png")?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), img.width, img.height);
    encoder.set_color(if img.has_alpha() {
        png::ColorType::Rgba
    } else {
        png::ColorType::Rgb
    });
    encoder.set_depth(if img.bits == 16 {
        png::BitDepth::Sixteen
    } else {
        png::BitDepth::Eight
    });
    match space {
        OutputSpace::Srgb => encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual),
        OutputSpace::DisplayP3 => {
            encoder.set_source_chromaticities(png::SourceChromaticities::new(
                (0.3127, 0.3290),
                (0.680, 0.320),
                (0.265, 0.690),
                (0.150, 0.060),
            ));
            encoder.set_source_gamma(png::ScaledFloat::new(1.0 / 2.2));
        }
    }
    if let Some(text) = scene_ron {
        if text.chars().all(|c| (c as u32) <= 255) {
            encoder.add_text_chunk(SCENE_KEYWORD.to_string(), text.to_string())
        } else {
            encoder.add_itxt_chunk(SCENE_KEYWORD.to_string(), text.to_string())
        }
        .context("png scene text chunk")?;
    }
    let mut writer = encoder.write_header().context("png header")?;
    let bytes: Vec<u8> = if img.bits == 16 {
        img.data.iter().flat_map(|v| v.to_be_bytes()).collect()
    } else {
        img.data.iter().map(|v| *v as u8).collect()
    };
    writer.write_image_data(&bytes).context("png data")?;
    writer.finish().context("png finish")?;
    Ok(())
}

/// Reads a scene keyword from UTF-8 iTXt or legacy Latin-1 tEXt/zTXt.
pub fn read_png_text(path: impl AsRef<Path>, keyword: &str) -> Result<Option<String>> {
    let decoder = png::Decoder::new(BufReader::new(
        File::open(path.as_ref()).context("open png")?,
    ));
    let reader = decoder.read_info().context("png info")?;
    let info = reader.info();
    if let Some(text) = info.utf8_text.iter().find(|t| t.keyword == keyword) {
        return Ok(Some(text.get_text().context("png UTF-8 scene text")?));
    }
    if let Some(text) = info
        .compressed_latin1_text
        .iter()
        .find(|t| t.keyword == keyword)
    {
        return Ok(Some(text.get_text().context("png compressed scene text")?));
    }
    Ok(info
        .uncompressed_latin1_text
        .iter()
        .find(|t| t.keyword == keyword)
        .map(|t| t.text.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::EncodedImage;
    use syfrack_core::HdrImage;

    fn gradient() -> HdrImage {
        let mut img = HdrImage::new(16, 8);
        for y in 0..8 {
            for x in 0..16 {
                img.set(x, y, [x as f32 / 15.0, y as f32 / 7.0, 2.5, 1.0]);
            }
        }
        img
    }

    #[test]
    fn exr_roundtrips_linear_values() {
        let dir = std::env::temp_dir().join("syfrack-color-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("roundtrip.exr");
        let img = gradient();
        write_exr(&path, &img).unwrap();
        let back = read_exr(&path).unwrap();
        assert_eq!(back.width, 16);
        assert_eq!(back.height, 8);
        for (a, b) in img.data.iter().zip(&back.data) {
            for c in 0..3 {
                assert!((a[c] - b[c]).abs() < 1e-6, "{a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn parallel_exr_compression_preserves_file_bytes_across_repeated_exports() {
        let dir =
            std::env::temp_dir().join(format!("syfrack-exr-reproducible-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Multiple clipped 64x64 blocks expose completion-order differences.
        let mut img = HdrImage::new(129, 73);
        for y in 0..img.height {
            for x in 0..img.width {
                img.set(
                    x,
                    y,
                    [x as f32 / 17.0, y as f32 / 31.0, (x * y) as f32 / 7.0, 1.0],
                );
            }
        }
        let path = dir.join("image.exr");
        write_exr(&path, &img).unwrap();
        let expected = std::fs::read(&path).unwrap();
        for _ in 0..8 {
            write_exr(&path, &img).unwrap();
            assert!(
                std::fs::read(&path).unwrap() == expected,
                "EXR block order changed"
            );
        }
        assert_eq!(read_exr(&path).unwrap(), img);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn exr_associates_alpha_for_compositors_and_restores_straight_hdr() {
        let path = std::env::temp_dir().join("syfrack-color-test/associated.exr");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let img = HdrImage::from_data(
            3,
            1,
            vec![[2.0, 0.4, 0.0, 0.5], [0.0; 4], [3.0, 1.0, 0.0, 0.0]],
        );
        write_exr(&path, &img).unwrap();
        let raw = read_first_rgba_layer_from_file(
            &path,
            |r, _| HdrImage::new(r.width() as u32, r.height() as u32),
            |i, p, (r, g, b, a): (f32, f32, f32, f32)| {
                i.set(p.x() as u32, p.y() as u32, [r, g, b, a])
            },
        )
        .unwrap();
        assert_eq!(
            raw.layer_data.channel_data.pixels.data,
            vec![[1.0, 0.2, 0.0, 0.5], [0.0; 4], [3.0, 1.0, 0.0, 0.0]]
        );
        assert_eq!(read_exr(&path).unwrap(), img);
    }

    #[test]
    fn png_carries_the_scene_text_chunk() {
        let dir = std::env::temp_dir().join("syfrack-color-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("meta.png");
        let enc = EncodedImage {
            width: 2,
            height: 1,
            bits: 16,
            data: vec![0, 65535, 1234, 5, 6, 7],
        };
        write_png(&path, &enc, OutputSpace::Srgb, Some("(seed: 1)")).unwrap();
        assert_eq!(
            read_png_text(&path, SCENE_KEYWORD).unwrap().as_deref(),
            Some("(seed: 1)")
        );
        assert_eq!(read_png_text(&path, "other").unwrap(), None);
    }

    #[test]
    fn png_roundtrips_unicode_scene_names_and_straight_alpha() {
        let path = std::env::temp_dir().join("syfrack-color-test/unicode-scene.png");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut scene = syfrack_core::Scene::example();
        scene.fractal.graph_mut().nodes[0].name = "Nébuleuse de l’Araignée — 星雲 🕷".into();
        let ron = scene.to_ron();
        let image = EncodedImage {
            width: 1,
            height: 1,
            bits: 8,
            data: vec![128, 32, 16, 64],
        };
        write_png(&path, &image, OutputSpace::Srgb, Some(&ron)).unwrap();
        let saved = read_png_text(&path, SCENE_KEYWORD).unwrap().unwrap();
        assert_eq!(syfrack_core::Scene::from_ron(&saved).unwrap(), scene);
        let mut reader = png::Decoder::new(BufReader::new(File::open(path).unwrap()))
            .read_info()
            .unwrap();
        assert_eq!(reader.info().utf8_text.len(), 1);
        assert!(reader.info().uncompressed_latin1_text.is_empty());
        let mut data = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut data).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgba);
        assert_eq!(&data[..info.buffer_size()], &[128, 32, 16, 64]);
    }

    #[test]
    fn rgba_png_roundtrips_alpha_at_both_depths() {
        let dir = std::env::temp_dir().join("syfrack-color-test");
        std::fs::create_dir_all(&dir).unwrap();
        for bits in [8, 16] {
            let img = HdrImage::from_data(2, 1, vec![[1.0, 0.0, 0.0, 0.5], [0.0; 4]]);
            let encoded = crate::encode::encode(&img, OutputSpace::Srgb, bits, false);
            let path = dir.join(format!("alpha-{bits}.png"));
            write_png(&path, &encoded, OutputSpace::Srgb, None).unwrap();
            let mut reader = png::Decoder::new(BufReader::new(File::open(path).unwrap()))
                .read_info()
                .unwrap();
            let mut buf = vec![0; reader.output_buffer_size().unwrap()];
            let info = reader.next_frame(&mut buf).unwrap();
            assert_eq!(info.color_type, png::ColorType::Rgba);
            let decoded: Vec<u16> = if bits == 16 {
                buf.chunks_exact(2)
                    .map(|c| u16::from_be_bytes([c[0], c[1]]))
                    .collect()
            } else {
                buf.iter().map(|c| *c as u16).collect()
            };
            assert_eq!(decoded, encoded.data);
        }
    }

    fn read_info(path: &Path) -> png::Info<'static> {
        let decoder = png::Decoder::new(BufReader::new(File::open(path).unwrap()));
        decoder.read_info().unwrap().info().clone()
    }

    #[test]
    fn png_carries_srgb_chunk() {
        let path = std::env::temp_dir()
            .join("syfrack-color-test")
            .join("srgb.png");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let enc = EncodedImage {
            width: 1,
            height: 1,
            bits: 8,
            data: vec![1, 2, 3],
        };
        write_png(&path, &enc, OutputSpace::Srgb, None).unwrap();
        assert!(read_info(&path).srgb.is_some());
    }

    #[test]
    fn png_carries_p3_chromaticities() {
        let path = std::env::temp_dir()
            .join("syfrack-color-test")
            .join("p3.png");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let enc = EncodedImage {
            width: 1,
            height: 1,
            bits: 8,
            data: vec![1, 2, 3],
        };
        write_png(&path, &enc, OutputSpace::DisplayP3, None).unwrap();
        let info = read_info(&path);
        assert!(info.chrm_chunk.is_some());
        assert!(info.gama_chunk.is_some());
        assert!(info.srgb.is_none());
    }

    #[test]
    fn png_8_bit_writes_one_byte_per_channel() {
        let dir = std::env::temp_dir().join("syfrack-color-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("eight.png");
        let enc = EncodedImage {
            width: 1,
            height: 1,
            bits: 8,
            data: vec![255, 128, 0],
        };
        write_png(&path, &enc, OutputSpace::Srgb, None).unwrap();
        let decoder =
            png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&path).unwrap()));
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!(info.bit_depth, png::BitDepth::Eight);
        assert_eq!(&buf[..3], &[255, 128, 0]);
    }
}
