//! Optional export filter. The raw radiance and accumulation stay untouched.
//! A three-scale à-trous filter uses geometry to keep architectural edges sharp.
use rayon::prelude::*;
use syfrack_core::{luminance, HdrImage};

/// Material guides used by the common, full-image export filter.
pub trait ExportSource {
    fn scene(&self) -> &syfrack_core::Scene;
    fn config(&self) -> &crate::RenderConfig;
    fn export_guides(&self) -> anyhow::Result<(HdrImage, HdrImage)>;
    fn export_surface_guides(&self) -> anyhow::Result<(HdrImage, HdrImage)>;
}
macro_rules! source {
    ($ty:ty) => {
        impl ExportSource for $ty {
            fn scene(&self) -> &syfrack_core::Scene {
                <$ty>::scene(self)
            }
            fn config(&self) -> &crate::RenderConfig {
                <$ty>::config(self)
            }
            fn export_guides(&self) -> anyhow::Result<(HdrImage, HdrImage)> {
                <$ty>::export_guides(self)
            }
            fn export_surface_guides(&self) -> anyhow::Result<(HdrImage, HdrImage)> {
                <$ty>::export_surface_guides(self)
            }
        }
    };
}
source!(crate::RenderSession);
source!(crate::ExportSession);

/// Preserve filtered material detail using dedicated albedo and bump guides.
pub fn session_output(session: &impl ExportSource, output: &crate::RenderOutput) -> HdrImage {
    if session.scene().dimension == syfrack_core::Dimension::TwoD {
        return output.color.clone();
    }
    if let Some(layers) = &output.layers {
        if let Ok((albedo, normals)) = session.export_surface_guides() {
            let transmission: Vec<f32> = normals.data.iter().map(|p| p[3]).collect();
            let volume = if let Some(components) = &layers.components {
                let direct = crate::volume_reconstruction::reconstruct(
                    &components.direct,
                    &components.direct_variance,
                    &transmission,
                );
                let indirect = HdrImage::from_data(
                    output.color.width,
                    output.color.height,
                    layers
                        .volume
                        .data
                        .iter()
                        .zip(&components.direct.data)
                        .map(|(v, d)| [v[0] - d[0], v[1] - d[1], v[2] - d[2], 1.0])
                        .collect(),
                );
                let indirect = crate::volume_reconstruction::reconstruct_indirect(
                    &indirect,
                    &components.indirect_variance,
                    &transmission,
                );
                HdrImage::from_data(
                    output.color.width,
                    output.color.height,
                    direct
                        .data
                        .iter()
                        .zip(&indirect.data)
                        .map(|(d, v)| [d[0] + v[0], d[1] + v[1], d[2] + v[2], 1.0])
                        .collect(),
                )
            } else {
                crate::volume_reconstruction::reconstruct_indirect(
                    &layers.volume,
                    &layers.volume_variance,
                    &transmission,
                )
            };
            let surface = crate::oidn::runtime()
                .and_then(|runtime| {
                    crate::oidn::filter_guided(&layers.surface, Some((&albedo, &normals)), &runtime)
                        .ok()
                })
                .unwrap_or_else(|| denoise(&layers.surface, &output.depth, &output.normals));
            let surface = if session.config().pixel_tolerance <= 0.35 {
                preserve_surface_detail(&layers.surface, &surface, &albedo, &normals)
            } else {
                surface
            };
            // The raw layer energies already include medium transport. Multiply
            // neither by transmittance again; composite before exposure/tonemapping.
            let data = surface
                .data
                .iter()
                .zip(&volume.data)
                .map(|(s, v)| [s[0] + v[0], s[1] + v[1], s[2] + v[2], 1.0])
                .collect();
            return HdrImage::from_data(output.color.width, output.color.height, data);
        }
    }
    if output.samples_min > 0 && output.color.width >= 64 && output.color.height >= 64 {
        if let Some(runtime) = crate::oidn::runtime() {
            let filtered = session.export_guides().and_then(|(albedo, normals)| {
                crate::oidn::filter_guided(&output.color, Some((&albedo, &normals)), &runtime)
            });
            match filtered {
                Ok(image) => {
                    if session.config().pixel_tolerance <= 0.35 {
                        if let Ok((albedo, normals)) = session.export_surface_guides() {
                            return preserve_surface_detail(
                                &output.color,
                                &image,
                                &albedo,
                                &normals,
                            );
                        }
                    }
                    return image;
                }
                Err(error) => eprintln!(
                    "Material-guided export unavailable; using the standard filter: {error:#}"
                ),
            }
        }
    }
    let filtered = denoise(&output.color, &output.depth, &output.normals);
    if session.config().pixel_tolerance <= 0.35 {
        if let Ok((albedo, normals)) = session.export_surface_guides() {
            return preserve_surface_detail(&output.color, &filtered, &albedo, &normals);
        }
    }
    filtered
}

/// Protect geometric creases and narrow material marks from spatial denoising.
/// Guides determine one RGB residual weight; noisy radiance does not invent edges.
pub fn preserve_surface_detail(
    raw: &HdrImage,
    filtered: &HdrImage,
    albedo: &HdrImage,
    normals: &HdrImage,
) -> HdrImage {
    assert_eq!((raw.width, raw.height), (filtered.width, filtered.height));
    assert_eq!((raw.width, raw.height), (albedo.width, albedo.height));
    assert_eq!((raw.width, raw.height), (normals.width, normals.height));
    let w = raw.width as usize;
    let h = raw.height as usize;
    let mut image = filtered.clone();
    rows_mut(&mut image.data, w, |y, row| {
        for (x, out) in row.iter_mut().enumerate() {
            let i = y * w + x;
            let n = normals.data[i];
            let a = albedo.data[i];
            if n[..3].iter().map(|v| v * v).sum::<f32>() < 0.5 {
                continue;
            }
            let mut difference = 0.0f32;
            for yy in y.saturating_sub(1)..=(y + 1).min(h - 1) {
                for xx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                    let j = yy * w + xx;
                    let m = normals.data[j];
                    let b = albedo.data[j];
                    let norm = (1.0 - (0..3).map(|c| n[c] * m[c]).sum::<f32>()).max(0.0);
                    let colour = (0..3).map(|c| (a[c] - b[c]).abs()).fold(0.0, f32::max);
                    difference = difference.max(norm.max(colour));
                }
            }
            let t = ((difference - 0.08) / 0.30).clamp(0.0, 1.0);
            let weight = t * t * (3.0 - 2.0 * t);
            for c in 0..3 {
                out[c] += weight * (raw.data[i][c] - out[c]);
            }
        }
    });
    image
}

#[cfg(test)]
mod detail_tests {
    use super::*;
    #[test]
    fn guided_detail_retains_thin_crease_but_does_not_restore_flat_noise() {
        let mut raw = HdrImage::new(9, 9);
        raw.data.fill([0.4, 0.2, 0.1, 1.0]);
        let mut filtered = raw.clone();
        raw.set(4, 4, [2.0, 1.0, 0.5, 1.0]);
        let mut albedo = HdrImage::new(9, 9);
        albedo.data.fill([0.5, 0.5, 0.5, 1.0]);
        let mut normals = HdrImage::new(9, 9);
        normals.data.fill([0.0, 0.0, 1.0, 1.0]);
        assert_eq!(
            preserve_surface_detail(&raw, &filtered, &albedo, &normals).get(4, 4),
            filtered.get(4, 4)
        );
        normals.set(4, 4, [1.0, 0.0, 0.0, 1.0]);
        filtered.set(4, 4, [0.3, 0.2, 0.1, 1.0]);
        assert_eq!(
            preserve_surface_detail(&raw, &filtered, &albedo, &normals).get(4, 4),
            raw.get(4, 4)
        );
    }
}

// Rows have disjoint output and read only the preceding pass. Keep tiny previews
// serial to avoid waking the pool for less work than scheduling costs.
fn rows_mut<T: Send>(data: &mut [T], width: usize, f: impl Fn(usize, &mut [T]) + Sync + Send) {
    if data.len() < 16_384 {
        data.chunks_mut(width)
            .enumerate()
            .for_each(|(y, row)| f(y, row));
    } else {
        data.par_chunks_mut(width)
            .enumerate()
            .for_each(|(y, row)| f(y, row));
    }
}

fn cache_logs(source: &[[f32; 4]], logs: &mut [[f32; 4]], width: usize) {
    rows_mut(logs, width, |y, row| {
        for (x, out) in row.iter_mut().enumerate() {
            let p = source[y * width + x];
            *out = [
                p[0].max(0.0).ln_1p(),
                p[1].max(0.0).ln_1p(),
                p[2].max(0.0).ln_1p(),
                luminance([p[0], p[1], p[2]]).max(0.0).ln_1p(),
            ];
        }
    });
}

pub fn denoise(image: &HdrImage, depth: &[f32], normals: &[[f32; 4]]) -> HdrImage {
    let (w, h) = (image.width as usize, image.height as usize);
    assert_eq!(depth.len(), w * h);
    assert_eq!(normals.len(), w * h);
    if w == 0 || h == 0 {
        return image.clone();
    }
    // Optional high-quality CPU export, suitable even on GPUs too old for
    // OIDN's CUDA backend. Retain the native coverage-aware filter for cutouts
    // and small previews, or if the external runtime fails.
    if w >= 64 && h >= 64 && image.data.iter().all(|p| p[3] == 1.0) {
        if let Some(runtime) = crate::oidn::runtime() {
            match crate::oidn::filter(image, &runtime) {
                Ok(filtered) => return filtered,
                Err(error) => {
                    static REPORTED: std::sync::atomic::AtomicBool =
                        std::sync::atomic::AtomicBool::new(false);
                    if !REPORTED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                        eprintln!("OIDN unavailable; using the native export filter: {error:#}");
                    }
                }
            }
        }
    }
    // A fog event records a positive depth but no surface normal. Treat it
    // like the background; a normal-based gate would reject even its centre
    // sample, leaving zero total weight and exporting NaN pixels as black.
    let surface: Vec<bool> = depth
        .iter()
        .zip(normals)
        .map(|(d, n)| *d > 0.0 && n[..3].iter().map(|v| v * v).sum::<f32>() > 0.5)
        .collect();
    let mut source = image.data.clone();
    // Work on straight RGB, but never let invisible RGB contaminate a visible
    // edge. Alpha is coverage and is kept at the centre pixel, not blurred.
    for p in &mut source {
        if !p[3].is_finite() || p[3] <= 0.0 {
            *p = [0.0; 4];
        }
    }
    let mut target = source.clone();
    // Suppress isolated bright paths only when enough neighbours describe
    // the same surface. Silhouettes, normal breaks and the sky are excluded.
    rows_mut(&mut target, w, |y, row| {
        for (x, out) in row.iter_mut().enumerate() {
            let i = y * w + x;
            if !surface[i] {
                continue;
            }
            let mut neighbours = [0.0f32; 8];
            let mut count = 0;
            for yy in y.saturating_sub(1)..=(y + 1).min(h - 1) {
                for xx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                    let j = yy * w + xx;
                    if i == j || !surface[j] {
                        continue;
                    }
                    let cosine = (0..3).map(|c| normals[i][c] * normals[j][c]).sum::<f32>();
                    if cosine < 0.95 || (depth[j] - depth[i]).abs() > depth[i].max(1.0) * 0.025 {
                        continue;
                    }
                    neighbours[count] =
                        luminance([source[j][0], source[j][1], source[j][2]]).max(0.0);
                    count += 1;
                }
            }
            if count >= 5 {
                neighbours[..count].sort_unstable_by(f32::total_cmp);
                let limit = neighbours[count / 2] * 2.0 + 0.05;
                let p = source[i];
                let l = luminance([p[0], p[1], p[2]]);
                if l > limit {
                    for c in 0..3 {
                        out[c] = p[c] * limit / l;
                    }
                }
            }
        }
    });
    std::mem::swap(&mut source, &mut target);
    // Reuse each expensive logarithm across the neighbouring filter taps.
    let mut logs = vec![[0.0; 4]; w * h];
    cache_logs(&source, &mut logs, w);
    let mut sigma = vec![0.0f32; w * h];
    rows_mut(&mut sigma, w, |y, row| {
        for (x, out) in row.iter_mut().enumerate() {
            let i = y * w + x;
            let mut sum = 0.0;
            let mut squares = 0.0;
            let mut count = 0.0;
            for yy in y.saturating_sub(1)..=(y + 1).min(h - 1) {
                for xx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                    let j = yy * w + xx;
                    if (depth[i] > 0.0) != (depth[j] > 0.0) {
                        continue;
                    }
                    let l = logs[j][3];
                    let alpha = source[j][3].clamp(0.0, 1.0);
                    sum += l * alpha;
                    squares += l * l * alpha;
                    count += alpha;
                }
            }
            *out = if count > 0.0 {
                (squares / count - (sum / count).powi(2)).max(0.0).sqrt()
            } else {
                0.0
            };
        }
    });
    for pass in 0..3 {
        let step = 1usize << pass;
        if pass > 0 {
            cache_logs(&source, &mut logs, w);
        }
        rows_mut(&mut target, w, |y, row| {
            for (x, out) in row.iter_mut().enumerate() {
                let i = y * w + x;
                let centre = source[i];
                if centre[3] == 0.0 {
                    *out = [0.0; 4];
                    continue;
                }
                let lc = logs[i][3];
                let colour_sigma = 0.035 + sigma[i] * 1.5 / (1.0 + pass as f32 * 0.5);
                let mut rgb = [0.0f32; 3];
                let mut weights = 0.0;
                for dy in -1isize..=1 {
                    for dx in -1isize..=1 {
                        let xx = x as isize + dx * step as isize;
                        let yy = y as isize + dy * step as isize;
                        if xx < 0 || yy < 0 || xx >= w as isize || yy >= h as isize {
                            continue;
                        }
                        let j = yy as usize * w + xx as usize;
                        if surface[i] != surface[j] {
                            continue;
                        }
                        let mut geometry = 1.0;
                        if surface[i] {
                            let ni = normals[i];
                            let nj = normals[j];
                            let cosine = ni[0] * nj[0] + ni[1] * nj[1] + ni[2] * nj[2];
                            if cosine < 0.95 {
                                continue;
                            }
                            geometry = cosine.max(0.0).powi(32)
                                * (-(depth[i] - depth[j]).abs()
                                    / (0.015 * depth[i].max(1.0) * step as f32))
                                    .exp();
                        }
                        let spatial =
                            if dx == 0 { 2.0 } else { 1.0 } * if dy == 0 { 2.0 } else { 1.0 };
                        let p = source[j];
                        // RGB gating preserves gold/ivory boundaries even at equal luminance.
                        let chroma =
                            (0..3).map(|c| (logs[j][c] - logs[i][c]).abs()).sum::<f32>() / 3.0;
                        let weight = spatial
                            * geometry
                            * (-((logs[j][3] - lc).abs().max(chroma)) / colour_sigma).exp()
                            * p[3].clamp(0.0, 1.0);
                        for c in 0..3 {
                            rgb[c] += p[c] * weight;
                        }
                        weights += weight;
                    }
                }
                *out = [
                    rgb[0] / weights,
                    rgb[1] / weights,
                    rgb[2] / weights,
                    centre[3],
                ];
            }
        });
        std::mem::swap(&mut source, &mut target);
    }
    HdrImage::from_data(image.width, image.height, source)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hidden_rgb_cannot_make_a_halo_in_semitransparent_edges() {
        let (w, h) = (17, 9);
        let mut image = HdrImage::new(w, h);
        for p in &mut image.data {
            *p = [50.0, 80.0, 100.0, 0.0];
        }
        for y in 0..h {
            for x in 4..13 {
                image.set(x, y, [1.0, 0.0, 0.0, 0.5]);
            }
        }
        let filtered = denoise(
            &image,
            &vec![-1.0; (w * h) as usize],
            &vec![[0.0; 4]; (w * h) as usize],
        );
        for p in filtered.data {
            if p[3] == 0.0 {
                assert_eq!(p, [0.0; 4]);
            } else {
                assert!((p[0] - 1.0).abs() < 1e-5);
                assert_eq!(&p[1..], [0.0, 0.0, 0.5]);
            }
        }
    }
    #[test]
    fn worker_count_preserves_pixels_on_odd_dimensions_and_geometry_edges() {
        let (w, h) = (257, 129);
        let mut data = Vec::new();
        let mut depth = Vec::new();
        let mut normals = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let v = ((x * 73 + y * 31) % 101) as f32 / 100.0;
                data.push([v, v * 0.6, v * 0.3, 0.7]);
                depth.push(if x < 40 { -1.0 } else { 4.0 + y as f32 * 0.01 });
                normals.push(if x < 40 {
                    [0.0; 4]
                } else if y < 60 {
                    [1.0, 0.0, 0.0, 1.0]
                } else {
                    [0.0, 1.0, 0.0, 1.0]
                });
            }
        }
        let image = HdrImage::from_data(w, h, data.clone());
        let run = |threads| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| denoise(&image, &depth, &normals))
        };
        let one = run(1);
        let four = run(4);
        assert_eq!(one.data, four.data);
        assert!(one.data.iter().flatten().all(|v| v.is_finite()));
        assert!(one.data.iter().all(|p| p[3] == 0.7));
        assert_eq!(image.data, data);
        assert_eq!(denoise(&HdrImage::new(0, 0), &[], &[]).data.len(), 0);
    }
    #[test]
    fn fog_events_with_no_normal_stay_finite_and_preserve_the_silhouette() {
        let (w, h) = (32, 16);
        let mut data = Vec::new();
        let mut depth = Vec::new();
        let mut normals = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let is_surface = x >= w / 2;
                let value = if is_surface { 0.8 } else { 0.2 };
                data.push([value, value, value, 1.0]);
                // Sky and first-sample scattering events have the same normal
                // marker, but arbitrarily different depths.
                depth.push(if is_surface {
                    4.0
                } else if (x + y) % 3 == 0 {
                    -1.0
                } else {
                    (x + y + 1) as f32
                });
                normals.push(if is_surface {
                    [0.0, 1.0, 0.0, 1.0]
                } else {
                    [0.0, 0.0, 0.0, -1.0]
                });
            }
        }
        let image = HdrImage::from_data(w as u32, h as u32, data.clone());
        let out = denoise(&image, &depth, &normals);
        assert!(out.data.iter().flatten().all(|v| v.is_finite()));
        for (actual, expected) in out.data.iter().zip(&data) {
            for c in 0..4 {
                assert!((actual[c] - expected[c]).abs() < 1e-5);
            }
        }
        assert_eq!(image.data, data);
    }
    #[test]
    fn suppresses_an_isolated_firefly_but_keeps_the_raw_radiance() {
        let mut image = HdrImage::from_data(16, 16, vec![[0.3, 0.3, 0.3, 1.0]; 256]);
        image.data[8 * 16 + 8] = [100.0, 100.0, 100.0, 1.0];
        let out = denoise(&image, &vec![4.0; 256], &vec![[0.0, 1.0, 0.0, 1.0]; 256]);
        assert!(out.get(8, 8)[0] < 0.5);
        assert_eq!(image.get(8, 8)[0], 100.0);
    }
    #[test]
    fn reduces_noise_without_mixing_surfaces_or_changing_the_raw_image() {
        let (w, h) = (32, 16);
        let mut data = Vec::new();
        let mut normals = Vec::new();
        let depth = vec![4.0; w * h];
        for y in 0..h {
            for x in 0..w {
                let base = if x < w / 2 { 0.2 } else { 0.8 };
                let noise = (((x * 73 + y * 31) % 23) as f32 / 22.0 - 0.5) * 0.2;
                data.push([base + noise, base + noise, base + noise, 1.0]);
                normals.push(if x < w / 2 {
                    [0.0, 1.0, 0.0, 1.0]
                } else {
                    [1.0, 0.0, 0.0, 1.0]
                });
            }
        }
        let image = HdrImage::from_data(w as u32, h as u32, data.clone());
        let out = denoise(&image, &depth, &normals);
        let error = |img: &HdrImage| {
            img.data
                .iter()
                .enumerate()
                .map(|(i, p)| (p[0] - if i % w < w / 2 { 0.2 } else { 0.8 }).powi(2))
                .sum::<f32>()
        };
        assert!(error(&out) < error(&image) * 0.4);
        assert_eq!(image.data, data);
        assert!(out.data.iter().enumerate().all(|(i, p)| if i % w < w / 2 {
            p[0] < 0.3
        } else {
            p[0] > 0.7
        }));
    }
}
