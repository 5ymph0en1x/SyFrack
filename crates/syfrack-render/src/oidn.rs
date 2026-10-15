//! Optional Open Image Denoise HDR export, isolated in a CPU-only child process.
//! No renderer samples, raw EXR, or GPU resources are changed by this filter.
use anyhow::{bail, Context, Result};
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use syfrack_core::HdrImage;

pub fn runtime() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SYFRACK_OIDN") {
        return Some(PathBuf::from(path));
    }
    let executable = std::env::current_exe().ok()?;
    for parent in executable.parent()?.ancestors().take(5) {
        for relative in [
            "runtime/oidn/bin/oidnDenoise.exe",
            "oidn/bin/oidnDenoise.exe",
        ] {
            let path = parent.join(relative);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

fn write_pfm(writer: impl Write, image: &HdrImage) -> Result<()> {
    let mut writer = BufWriter::new(writer);
    write!(writer, "PF\n{} {}\n-1.0\n", image.width, image.height)?;
    for row in image.data.chunks_exact(image.width as usize).rev() {
        for pixel in row {
            for value in &pixel[..3] {
                writer.write_all(&value.to_le_bytes())?;
            }
        }
    }
    writer.flush()?;
    Ok(())
}

fn read_pfm(reader: impl Read, image: &HdrImage) -> Result<HdrImage> {
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    if line.trim() != "PF" {
        bail!("OIDN output must have three float channels");
    }
    line.clear();
    reader.read_line(&mut line)?;
    let dims: Vec<u32> = line
        .split_whitespace()
        .map(str::parse)
        .collect::<std::result::Result<_, _>>()?;
    if dims != [image.width, image.height] {
        bail!("OIDN output dimensions changed");
    }
    line.clear();
    reader.read_line(&mut line)?;
    let scale: f32 = line.trim().parse()?;
    if !scale.is_finite() || scale == 0.0 {
        bail!("invalid PFM scale");
    }
    let mut result = image.clone();
    for row in result.data.chunks_exact_mut(image.width as usize).rev() {
        for pixel in row {
            for value in &mut pixel[..3] {
                let mut bytes = [0; 4];
                reader.read_exact(&mut bytes)?;
                let decoded = if scale < 0.0 {
                    f32::from_le_bytes(bytes)
                } else {
                    f32::from_be_bytes(bytes)
                };
                let decoded = decoded * scale.abs();
                if !decoded.is_finite() {
                    bail!("nonfinite OIDN output");
                }
                *value = decoded.max(0.0);
            }
        }
    }
    let mut extra = [0];
    if reader.read(&mut extra)? != 0 {
        bail!("unexpected data after OIDN output");
    }
    Ok(result)
}

struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        for name in ["color.pfm", "filtered.pfm", "albedo.pfm", "normals.pfm"] {
            let _ = std::fs::remove_file(self.0.join(name));
        }
        let _ = std::fs::remove_dir(&self.0);
    }
}

/// Filter an opaque linear HDR image. Alpha and source pixels remain intact.
/// The process boundary isolates unsupported external runtimes from Studio.
pub fn filter(image: &HdrImage, executable: &Path) -> Result<HdrImage> {
    filter_guided(image, None, executable)
}

pub fn filter_guided(
    image: &HdrImage,
    guides: Option<(&HdrImage, &HdrImage)>,
    executable: &Path,
) -> Result<HdrImage> {
    if image.width == 0
        || image.height == 0
        || image.data.len() != (u64::from(image.width) * u64::from(image.height)) as usize
        || image
            .data
            .iter()
            .any(|p| p[3] != 1.0 || p[..3].iter().any(|v| !v.is_finite() || *v < 0.0))
    {
        bail!("OIDN requires an opaque, finite, nonnegative HDR image");
    }
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "syfrack-oidn-{}-{stamp}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&dir)?;
    let temporary = Temporary(dir);
    let input = temporary.0.join("color.pfm");
    let output = temporary.0.join("filtered.pfm");
    write_pfm(std::fs::File::create(&input)?, image)?;
    let mut command = std::process::Command::new(executable);
    command
        .args([
            "--device",
            "cpu",
            "--threads",
            "4",
            "--affinity",
            "0",
            "--maxmem",
            "1024",
            "--quality",
            "high",
            "--hdr",
        ])
        .arg(&input)
        .arg("--output")
        .arg(&output);
    if let Some((albedo, normals)) = guides {
        for (name, guide, flag) in [
            ("albedo.pfm", albedo, "--alb"),
            ("normals.pfm", normals, "--nrm"),
        ] {
            anyhow::ensure!(
                guide.width == image.width
                    && guide.height == image.height
                    && guide.data.len() == image.data.len(),
                "guide dimensions changed"
            );
            anyhow::ensure!(
                guide.data.iter().flatten().all(|v| v.is_finite()),
                "nonfinite material guide"
            );
            let path = temporary.0.join(name);
            write_pfm(std::fs::File::create(&path)?, guide)?;
            command.arg(flag).arg(path);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let status = command
        .output()
        .context("launching the optional OIDN CPU filter")?;
    if !status.status.success() {
        bail!(
            "OIDN failed: {} {}",
            String::from_utf8_lossy(&status.stdout),
            String::from_utf8_lossy(&status.stderr)
        );
    }
    read_pfm(
        std::fs::File::open(&output).context("OIDN did not produce an image")?,
        image,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pfm_roundtrip_preserves_rows_hdr_and_alpha() {
        let image = HdrImage::from_data(
            2,
            2,
            vec![
                [0.1, 8., 0.3, 1.],
                [4., 0.5, 6., 1.],
                [0.7, 0.8, 9., 1.],
                [1., 1.1, 1.2, 1.],
            ],
        );
        let mut bytes = Vec::new();
        write_pfm(&mut bytes, &image).unwrap();
        assert_eq!(read_pfm(bytes.as_slice(), &image).unwrap().data, image.data);
        bytes.pop();
        assert!(read_pfm(bytes.as_slice(), &image).is_err());
    }
    #[test]
    fn pfm_rejects_bad_dimensions_and_nonfinite_values() {
        let image = HdrImage::new(2, 2);
        assert!(read_pfm(&b"PF\n1 2\n-1\n"[..], &image).is_err());
        let mut bad = image.clone();
        bad.data[0][0] = f32::NAN;
        let mut bytes = Vec::new();
        write_pfm(&mut bytes, &bad).unwrap();
        assert!(read_pfm(bytes.as_slice(), &image).is_err());
    }
    #[test]
    fn transparent_or_invalid_inputs_are_rejected_before_launch() {
        for pixel in [[1., 2., 3., 0.5], [f32::NAN, 0., 0., 1.], [-1., 0., 0., 1.]] {
            let image = HdrImage::from_data(1, 1, vec![pixel]);
            assert!(filter(&image, Path::new("missing-runtime"))
                .unwrap_err()
                .to_string()
                .contains("requires an opaque"));
        }
    }
    #[test]
    #[ignore = "requires the optional OIDN CPU runtime"]
    fn cpu_filter_reduces_noise_preserves_edges_and_source() {
        let mut data = Vec::new();
        let mut reference = Vec::new();
        for y in 0..96 {
            for x in 0..128 {
                let c = if x < 64 {
                    [0.05, 0.10, 0.40]
                } else {
                    [2.0, 0.10, 0.05]
                };
                reference.push([c[0], c[1], c[2], 1.]);
                let mut bits = (x + y * 128 + 1) as u32;
                bits = (bits ^ (bits >> 16)).wrapping_mul(0x7feb352d);
                bits = (bits ^ (bits >> 15)).wrapping_mul(0x846ca68b);
                bits ^= bits >> 16;
                let noise = (bits >> 8) as f32 / 16777216.0;
                data.push([
                    c[0] * (0.2 + 1.6 * noise),
                    c[1] * (0.2 + 1.6 * noise),
                    c[2] * (0.2 + 1.6 * noise),
                    1.,
                ]);
            }
        }
        let input = HdrImage::from_data(128, 96, data.clone());
        let result = filter(&input, &runtime().expect("install OIDN first")).unwrap();
        assert_eq!(input.data, data);
        assert!(result.data.iter().flatten().all(|v| v.is_finite()));
        assert!(result.data.iter().all(|p| p[3] == 1.0));
        let mse = |pixels: &[[f32; 4]]| {
            pixels
                .iter()
                .zip(&reference)
                .map(|(p, r)| (0..3).map(|c| (p[c] - r[c]).powi(2)).sum::<f32>())
                .sum::<f32>()
                / pixels.len() as f32
        };
        eprintln!("HDR denoise MSE: {} -> {}", mse(&data), mse(&result.data));
        assert!(mse(&result.data) < mse(&data) * 0.35);
        assert!(result.data[48 * 128 + 60][2] > result.data[48 * 128 + 60][0] * 2.0);
        assert!(result.data[48 * 128 + 68][0] > result.data[48 * 128 + 68][2] * 8.0);
    }
}
