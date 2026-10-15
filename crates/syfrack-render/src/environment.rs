//! Bounded panorama assets and an importance distribution in solid angle.
//! RGB and the cumulative probability share one vec4 per texel (16 bytes).
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use anyhow::{Context, Result};
use image::{ImageDecoder, ImageFormat, ImageReader};
use syfrack_core::{Light, Scene};
use wgpu::util::DeviceExt;

use crate::renderer::Renderer;
use crate::uniforms::Uniforms;

pub const MAX_WIDTH: u32 = 2048;
pub const MAX_HEIGHT: u32 = 1024;
pub const MAX_GPU_BYTES: u64 = 64 * 1024 * 1024;
const MAX_INPUT_PIXELS: u64 = 8192 * 4096;
const UNIFORM_MIX: f64 = 0.0625;

#[derive(Clone, Debug, PartialEq, Hash)]
struct AssetStamp {
    index: usize,
    path: PathBuf,
    size: Option<u64>,
    modified: Option<SystemTime>,
}

fn stamps(scene: &Scene) -> Vec<AssetStamp> {
    scene
        .lights
        .iter()
        .enumerate()
        .filter_map(|(index, light)| {
            let Light::Hdri { path, .. } = light else {
                return None;
            };
            let absolute = std::env::current_dir().unwrap_or_default().join(path);
            let path = absolute.canonicalize().unwrap_or(absolute);
            let metadata = std::fs::metadata(&path).ok();
            Some(AssetStamp {
                index,
                path,
                size: metadata.as_ref().map(|m| m.len()),
                modified: metadata.and_then(|m| m.modified().ok()),
            })
        })
        .collect()
}

/// Preview resource identity; intensity/rotation edits keep the same assets.
/// Scenes without a panorama retain their graph topology identity.
pub fn scene_resource_fingerprint(scene: &Scene) -> u64 {
    crate::particles::resource_key(
        scene,
        resource_hash(crate::graph_wgsl::scene_fingerprint(scene), &stamps(scene)),
    )
}

fn resource_hash(graph: u64, assets: &[AssetStamp]) -> u64 {
    if assets.is_empty() {
        return graph;
    }
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    graph.hash(&mut hash);
    assets.hash(&mut hash);
    hash.finish()
}

#[derive(Default)]
pub(crate) struct EnvironmentCache {
    prepared: Mutex<Option<Arc<PreparedEnvironment>>>,
    empty: OnceLock<Arc<PreparedEnvironment>>,
}

pub(crate) struct PreparedEnvironment {
    pub buffer: wgpu::Buffer,
    entries: Vec<(usize, [u32; 4], bool)>,
    stamps: Vec<AssetStamp>,
    pub fingerprint: u64,
}

impl PreparedEnvironment {
    pub fn resource_fingerprint(&self, graph: u64) -> u64 {
        resource_hash(graph, &self.stamps)
    }
    pub fn apply(&self, uniforms: &mut Uniforms) {
        for (index, kind, uniform) in &self.entries {
            uniforms.lights[*index].kind = *kind;
            uniforms.lights[*index].a[2] = f32::from(u8::from(*uniform));
        }
    }
}

impl Renderer {
    /// Clear the asset cache before a strict checkpoint content check or reload.
    pub fn invalidate_environment(&self) {
        *self
            .environment_cache
            .prepared
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
    }

    pub fn environment_ready(&self, scene: &Scene) -> bool {
        if !self.particles_ready(scene) {
            return false;
        }
        let assets = stamps(scene);
        if assets.is_empty() {
            return true;
        }
        self.environment_cache
            .prepared
            .try_lock()
            .ok()
            .and_then(|cache| cache.as_ref().map(|env| env.stamps == assets))
            .unwrap_or(false)
    }

    pub(crate) fn environment_for(&self, scene: &Scene) -> Result<Arc<PreparedEnvironment>> {
        let assets = stamps(scene);
        if assets.is_empty() {
            if let Some(empty) = self.environment_cache.empty.get() {
                return Ok(empty.clone());
            }
            let buffer = self.with_scopes(|| {
                Ok(self
                    .context()
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("empty HDRI"),
                        contents: &[0; 16],
                        usage: wgpu::BufferUsages::STORAGE,
                    }))
            })?;
            let empty = Arc::new(PreparedEnvironment {
                buffer,
                entries: Vec::new(),
                stamps: Vec::new(),
                fingerprint: 0,
            });
            let _ = self.environment_cache.empty.set(empty);
            return Ok(self.environment_cache.empty.get().unwrap().clone());
        }
        let mut cached = self
            .environment_cache
            .prepared
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(env) = &*cached {
            if env.stamps == assets {
                return Ok(env.clone());
            }
        }
        let mut pixels = Vec::<[f32; 4]>::new();
        let mut entries = Vec::new();
        let mut loaded = HashMap::<PathBuf, ([u32; 4], bool)>::new();
        let limit = MAX_GPU_BYTES.min(u64::from(
            self.context()
                .device
                .limits()
                .max_storage_buffer_binding_size,
        ));
        for asset in &assets {
            let (kind, uniform) = if let Some(descriptor) = loaded.get(&asset.path) {
                *descriptor
            } else {
                let map = EnvironmentMap::load(&asset.path)
                    .with_context(|| format!("loading HDRI {}", asset.path.display()))?;
                let bytes = (pixels.len() as u64 + map.texels.len() as u64) * 16;
                anyhow::ensure!(bytes <= limit,
                    "HDRI assets need {bytes} GPU bytes, above the {limit}-byte panorama budget; use smaller maps");
                let kind = [
                    crate::uniforms::KIND_HDRI,
                    pixels.len() as u32,
                    map.width,
                    map.height,
                ];
                let uniform = map.uniform_sampling;
                pixels.extend(map.texels);
                loaded.insert(asset.path.clone(), (kind, uniform));
                (kind, uniform)
            };
            entries.push((asset.index, kind, uniform));
        }
        let fingerprint = if assets.is_empty() {
            0
        } else {
            let mut hash = 0xcbf29ce484222325u64;
            for (_, kind, uniform) in &entries {
                hash_bytes(&mut hash, &[u8::from(*uniform)]);
                for value in kind {
                    hash_bytes(&mut hash, &value.to_le_bytes());
                }
            }
            for pixel in &pixels {
                for value in pixel {
                    hash_bytes(&mut hash, &value.to_bits().to_le_bytes());
                }
            }
            hash
        };
        if pixels.is_empty() {
            pixels.push([0.0; 4]);
        }
        let buffer = self.with_scopes(|| {
            Ok(self
                .context()
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("HDRI radiance + CDF"),
                    contents: bytemuck::cast_slice(&pixels),
                    usage: wgpu::BufferUsages::STORAGE,
                }))
        })?;
        let env = Arc::new(PreparedEnvironment {
            buffer,
            entries,
            stamps: assets,
            fingerprint,
        });
        *cached = Some(env.clone());
        Ok(env)
    }
}

fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash = (*hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
    }
}

/// Linear panorama plus a normalized luminance/solid-angle CDF.
pub struct EnvironmentMap {
    pub width: u32,
    pub height: u32,
    pub texels: Vec<[f32; 4]>,
    pub uniform_sampling: bool,
}

impl EnvironmentMap {
    pub fn load(path: &Path) -> Result<Self> {
        let extension = path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        let format = match extension.as_str() {
            "hdr" => ImageFormat::Hdr,
            "exr" => ImageFormat::OpenExr,
            _ => anyhow::bail!("a panorama must be a linear .hdr or .exr image"),
        };
        let mut reader = ImageReader::open(path)?;
        reader.set_format(format);
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(512 * 1024 * 1024);
        reader.limits(limits);
        let decoder = reader.into_decoder()?;
        let (width, height) = decoder.dimensions();
        anyhow::ensure!(
            width > 0 && height > 0 && u64::from(width) * u64::from(height) <= MAX_INPUT_PIXELS,
            "panorama exceeds the 32-megapixel decode budget"
        );
        let rgb = image::DynamicImage::from_decoder(decoder)?.into_rgb32f();
        Self::from_rgb(width, height, rgb.as_raw())
    }

    pub fn from_rgb(width: u32, height: u32, rgb: &[f32]) -> Result<Self> {
        anyhow::ensure!(
            width > 0 && height > 0 && rgb.len() as u64 == u64::from(width) * u64::from(height) * 3,
            "invalid panorama dimensions or RGB pixel count"
        );
        anyhow::ensure!(
            rgb.iter().all(|v| v.is_finite()),
            "panorama contains non-finite radiance"
        );
        let scale = (f64::from(width) / f64::from(MAX_WIDTH))
            .max(f64::from(height) / f64::from(MAX_HEIGHT))
            .max(1.0);
        let out_w = (f64::from(width) / scale).floor().max(1.0) as u32;
        let out_h = (f64::from(height) / scale).floor().max(1.0) as u32;
        let mut texels = Vec::with_capacity((out_w * out_h) as usize);
        // Integrate source texels over each output cell in solid angle.
        // Small bright sources keep their energy when an 8K map becomes 2K.
        for oy in 0..out_h {
            let top = f64::from(oy) * f64::from(height) / f64::from(out_h);
            let bottom = f64::from(oy + 1) * f64::from(height) / f64::from(out_h);
            for ox in 0..out_w {
                let left = f64::from(ox) * f64::from(width) / f64::from(out_w);
                let right = f64::from(ox + 1) * f64::from(width) / f64::from(out_w);
                let mut sum = [0.0f64; 3];
                let mut total = 0.0;
                for iy in top.floor() as u32..(bottom.ceil() as u32).min(height) {
                    let y0 = top.max(f64::from(iy)) / f64::from(height);
                    let y1 = bottom.min(f64::from(iy + 1)) / f64::from(height);
                    let latitude =
                        (std::f64::consts::PI * y0).cos() - (std::f64::consts::PI * y1).cos();
                    for ix in left.floor() as u32..(right.ceil() as u32).min(width) {
                        let weight =
                            latitude * (right.min(f64::from(ix + 1)) - left.max(f64::from(ix)));
                        let index = ((iy * width + ix) * 3) as usize;
                        for channel in 0..3 {
                            sum[channel] += f64::from(rgb[index + channel].max(0.0)) * weight;
                        }
                        total += weight;
                    }
                }
                texels.push([
                    (sum[0] / total) as f32,
                    (sum[1] / total) as f32,
                    (sum[2] / total) as f32,
                    0.0,
                ]);
            }
        }
        let uniform_sampling = texels.iter().all(|p| p[..3] == texels[0][..3]);
        if uniform_sampling {
            texels.last_mut().unwrap()[3] = 1.0;
            return Ok(Self {
                width: out_w,
                height: out_h,
                texels,
                uniform_sampling,
            });
        }
        // Bilinear reconstruction spreads a bright texel into its neighbours.
        // Integrating its tent over each cell gives [1/8, 3/4, 1/8].
        // Include that halo in the proposal, avoiding rare enormous weights
        // in nominally black cells at the edge of a tiny HDR light source.
        let mut weights = Vec::with_capacity(texels.len());
        for y in 0..out_h {
            for x in 0..out_w {
                let mut weight = 0.0;
                for (dy, wy) in [(-1, 0.125), (0, 0.75), (1, 0.125)] {
                    let row = (y as i32 + dy).clamp(0, out_h as i32 - 1) as u32;
                    for (dx, wx) in [(-1, 0.125), (0, 0.75), (1, 0.125)] {
                        let col = (x as i32 + dx).rem_euclid(out_w as i32) as u32;
                        weight += luminance(&texels[(row * out_w + col) as usize]) * wx * wy;
                    }
                }
                weights.push(weight * cell_area(out_w, out_h, y));
            }
        }
        let total: f64 = weights.iter().sum();
        let mut cumulative = 0.0;
        for (i, pixel) in texels.iter_mut().enumerate() {
            let area = cell_area(out_w, out_h, i as u32 / out_w);
            cumulative += if total > 0.0 {
                weights[i] / total
            } else {
                area / (4.0 * std::f64::consts::PI)
            };
            pixel[3] = cumulative.min(1.0) as f32;
        }
        texels.last_mut().unwrap()[3] = 1.0;
        Ok(Self {
            width: out_w,
            height: out_h,
            texels,
            uniform_sampling,
        })
    }

    /// Density of the same uniform/importance mixture used by the shader.
    pub fn cell_pdf(&self, index: usize) -> f64 {
        if self.uniform_sampling {
            return 1.0 / (4.0 * std::f64::consts::PI);
        }
        let before = if index == 0 {
            0.0
        } else {
            f64::from(self.texels[index - 1][3])
        };
        let mass = f64::from(self.texels[index][3]) - before;
        (1.0 - UNIFORM_MIX) * mass / cell_area(self.width, self.height, index as u32 / self.width)
            + UNIFORM_MIX / (4.0 * std::f64::consts::PI)
    }
}

fn luminance(pixel: &[f32; 4]) -> f64 {
    f64::from(pixel[0]) * 0.2126 + f64::from(pixel[1]) * 0.7152 + f64::from(pixel[2]) * 0.0722
}

pub fn cell_area(width: u32, height: u32, row: u32) -> f64 {
    let top = std::f64::consts::PI * f64::from(row) / f64::from(height);
    let bottom = std::f64::consts::PI * f64::from(row + 1) / f64::from(height);
    // Stable even near the poles, where subtracting cosines loses precision.
    2.0 * std::f64::consts::PI / f64::from(width)
        * 2.0
        * ((top + bottom) * 0.5).sin()
        * ((bottom - top) * 0.5).sin()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distribution_integrates_to_one_and_keeps_dark_cells_in_support() {
        let mut rgb = vec![0.0; 16 * 8 * 3];
        rgb[3 * 22..3 * 23].fill(1e8);
        let map = EnvironmentMap::from_rgb(16, 8, &rgb).unwrap();
        let integral: f64 = (0..128)
            .map(|i| map.cell_pdf(i) * cell_area(16, 8, i as u32 / 16))
            .sum();
        assert!((integral - 1.0).abs() < 1e-10);
        assert!((0..128).all(|i| map.cell_pdf(i).is_finite() && map.cell_pdf(i) > 0.0));
        assert!(map.cell_pdf(22) > map.cell_pdf(23) * 4.0);
    }

    #[test]
    fn black_map_uses_uniform_solid_angle_and_invalid_pixels_are_refused() {
        let map = EnvironmentMap::from_rgb(32, 16, &vec![0.0; 32 * 16 * 3]).unwrap();
        for i in 0..512 {
            assert!((map.cell_pdf(i) - 1.0 / (4.0 * std::f64::consts::PI)).abs() < 1e-5);
        }
        assert!(EnvironmentMap::from_rgb(1, 1, &[f32::NAN, 0.0, 0.0]).is_err());
        assert!(EnvironmentMap::from_rgb(1, 1, &[f32::INFINITY, 0.0, 0.0]).is_err());
        assert!(EnvironmentMap::from_rgb(0, 1, &[]).is_err());
    }

    #[test]
    fn downsampling_preserves_a_tiny_sources_spherical_energy() {
        let (w, h) = (4097u32, 1u32);
        let mut rgb = vec![0.0; (w * h * 3) as usize];
        rgb[3 * 2048..3 * 2049].fill(1000.0);
        let map = EnvironmentMap::from_rgb(w, h, &rgb).unwrap();
        assert_eq!((map.width, map.height), (MAX_WIDTH, 1));
        let actual: f64 = map
            .texels
            .iter()
            .map(|p| f64::from(p[0]) * cell_area(map.width, 1, 0))
            .sum();
        let expected = 1000.0 * cell_area(w, h, 0);
        assert!((actual - expected).abs() < expected * 1e-6);
    }
}
