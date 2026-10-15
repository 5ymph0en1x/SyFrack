//! Particle/grid smoke snapshots and cached heterogeneous volume lighting.
//! Gas is transported as mass-carrying particles, not camera-facing sprites.
//! Velocity uses semi-Lagrangian advection and a MAC pressure projection.
//! Gaussian reconstruction is baked once; the GPU integrates the 3D medium.
use crate::renderer::Renderer;
use anyhow::Result;
use glam::{Vec3, Vec4};
use rayon::prelude::*;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use syfrack_core::{Light, Nebula, Particles, Scene};

#[derive(Default)]
pub(crate) struct ParticleCache {
    prepared: Mutex<Option<Arc<PreparedParticles>>>,
    density: Mutex<Option<(u64, Arc<Grid<[f32; 4]>>)>>,
    empty: OnceLock<Arc<PreparedParticles>>,
    microstructure: OnceLock<Arc<Microstructure>>,
}
pub(crate) struct PreparedParticles {
    pub view: wgpu::TextureView,
    pub lighting: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub microstructure: Arc<Microstructure>,
    pub fingerprint: u64,
}
pub(crate) struct Microstructure {
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
}

fn sun(scene: &Scene) -> Option<Vec3> {
    scene.lights.iter().find_map(|l| match l {
        Light::Sun { direction, .. } => Some(direction.normalize()),
        _ => None,
    })
}
fn local(v: Vec3, angle: f32) -> Vec3 {
    let (s, c) = (-angle.to_radians()).sin_cos();
    Vec3::new(c * v.x - s * v.y, s * v.x + c * v.y, v.z)
}
pub fn resource_key(scene: &Scene, base: u64) -> u64 {
    let key = particle_key(scene);
    if key == 0 {
        return base;
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    base.hash(&mut h);
    key.hash(&mut h);
    h.finish()
}
fn particle_key(scene: &Scene) -> u64 {
    let Some(n) = scene
        .atmosphere
        .nebula
        .as_ref()
        .filter(|n| n.particles.is_some())
    else {
        return 0;
    };
    // Camera, exposure, palette, emission energy and quadrature do not rebuild
    // density/visibility. Shadow extinction and source locations do.
    let mut h = std::collections::hash_map::DefaultHasher::new();
    // CPU generation affects transport just as much as WGSL; checkpoints
    // must reject a changed simulation, even if their shader is unchanged.
    crate::shader_source::transport_fingerprint(include_str!("particles.rs")).hash(&mut h);
    let mut transport = n.particles.clone();
    if let Some(p) = &mut transport {
        p.cluster_stars = 0.0;
        p.spectral_detail = 0.0;
    }
    ron::to_string(&(
        transport,
        n.half_size,
        n.seed,
        n.turbulence,
        n.detail,
        n.density,
        n.dust,
        n.scattering,
        n.core_position,
        n.core_radius,
        n.core_energy > 0.0,
        sun(scene).map(|d| local(d, n.angle)),
    ))
    .unwrap()
    .hash(&mut h);
    h.finish()
}
fn simulation_key(n: &Nebula, p: &Particles) -> u64 {
    let mut p = p.clone();
    p.shadow_steps = 0;
    p.multiple_scattering = 0.0;
    p.cluster_stars = 0.0;
    p.gas_opacity = 1.0;
    p.spectral_detail = 0.0;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let cavity = if p.filaments && (p.spider || p.storm || p.remnant) && !p.smoke {
        Some(n.core_position)
    } else {
        None
    };
    ron::to_string(&(p, n.half_size, n.seed, n.turbulence, n.detail, cavity))
        .unwrap()
        .hash(&mut h);
    h.finish()
}
impl Renderer {
    pub(crate) fn particles_ready(&self, scene: &Scene) -> bool {
        let key = particle_key(scene);
        key == 0
            || self
                .particle_cache
                .prepared
                .try_lock()
                .ok()
                .and_then(|p| p.as_ref().map(|p| p.fingerprint == key))
                .unwrap_or(false)
    }
    pub(crate) fn particles_for(&self, scene: &Scene) -> Result<Arc<PreparedParticles>> {
        let key = particle_key(scene);
        if key == 0 {
            return Ok(self
                .particle_cache
                .empty
                .get_or_init(|| {
                    Arc::new(upload(
                        self,
                        [1, 1, 1],
                        &[[0.0; 4]],
                        &[[1.0, 1.0, 0.0, 0.0]],
                        0,
                    ))
                })
                .clone());
        }
        let mut cache = self
            .particle_cache
            .prepared
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(p) = cache.as_ref().filter(|p| p.fingerprint == key) {
            return Ok(p.clone());
        }
        let start = Instant::now();
        let n = scene.atmosphere.nebula.as_ref().unwrap();
        let p = n.particles.as_ref().unwrap();
        let simulation_key = simulation_key(n, p);
        let mut density = self
            .particle_cache
            .density
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let volume =
            if let Some((_, v)) = density.as_ref().filter(|(key, _)| *key == simulation_key) {
                v.clone()
            } else {
                let v = Arc::new(simulate(n, p));
                *density = Some((simulation_key, v.clone()));
                v
            };
        drop(density);
        let lighting = illuminate(&volume, n, p, sun(scene).map(|d| local(d, n.angle)));
        let prepared = self.with_scopes(|| {
            Ok(Arc::new(upload(
                self,
                volume.dims,
                &volume.data,
                &lighting,
                key,
            )))
        })?;
        let model = if p.filaments && !p.smoke {
            "continuous filaments".to_string()
        } else {
            format!("{} particles", p.count)
        };
        eprintln!(
            "volume: {}, {}x{}x{} voxels, {:.1} MiB GPU, prepared in {:.2}s",
            model,
            volume.dims[0],
            volume.dims[1],
            volume.dims[2],
            volume.data.len() as f64 * 16.0 / (1024.0 * 1024.0),
            start.elapsed().as_secs_f64()
        );
        *cache = Some(prepared.clone());
        Ok(prepared)
    }
}
fn upload(
    r: &Renderer,
    dims: [usize; 3],
    density: &[[f32; 4]],
    lighting: &[[f32; 4]],
    key: u64,
) -> PreparedParticles {
    let make = |name, data: &[[f32; 4]]| {
        let texture = r.ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(name),
            size: wgpu::Extent3d {
                width: dims[0] as u32,
                height: dims[1] as u32,
                depth_or_array_layers: dims[2] as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let data: Vec<u16> = data
            .iter()
            .flat_map(|p| p.map(|v| half::f16::from_f32(v.min(65504.0)).to_bits()))
            .collect();
        r.ctx.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&data),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(dims[0] as u32 * 8),
                rows_per_image: Some(dims[1] as u32),
            },
            texture.size(),
        );
        texture.create_view(&Default::default())
    };
    PreparedParticles {
        view: make("particle density", density),
        lighting: make("volume visibility and diffusion", lighting),
        sampler: r.ctx.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("volume trilinear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        }),
        fingerprint: key,
        microstructure: r
            .particle_cache
            .microstructure
            .get_or_init(|| Arc::new(upload_microstructure(r)))
            .clone(),
    }
}

// A small, reusable periodic noise/gradient atlas replaces hundreds of hashes
// per ray sample. Coordinates are in 16 noise cells; each cell has 4 texels.
// Signed derivatives require RGBA16F. Exactly 2 MiB, independent of scene size.
fn upload_microstructure(r: &Renderer) -> Microstructure {
    const SIDE: usize = 64;
    let mut data = vec![[0u16; 4]; SIDE * SIDE * SIDE];
    data.par_iter_mut().enumerate().for_each(|(i, output)| {
        let p = Vec3::new(
            (i % SIDE) as f32,
            ((i / SIDE) % SIDE) as f32,
            (i / (SIDE * SIDE)) as f32,
        ) / 4.0;
        let (value, gradient) = noise_impl(p, 0x745a13, true);
        *output = gradient
            .extend(value)
            .to_array()
            .map(|v| half::f16::from_f32(v).to_bits());
    });
    let texture = r.ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("periodic nebular microstructure and gradients"),
        size: wgpu::Extent3d {
            width: SIDE as u32,
            height: SIDE as u32,
            depth_or_array_layers: SIDE as u32,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    r.ctx.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(&data),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(SIDE as u32 * 8),
            rows_per_image: Some(SIDE as u32),
        },
        texture.size(),
    );
    Microstructure {
        view: texture.create_view(&Default::default()),
        sampler: r.ctx.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("periodic microstructure trilinear"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        }),
    }
}

#[derive(Clone)]
struct Grid<T> {
    dims: [usize; 3],
    half: Vec3,
    data: Vec<T>,
}
impl<T: Copy + Default> Grid<T> {
    fn new(dims: [usize; 3], half: Vec3) -> Self {
        Self {
            dims,
            half,
            data: vec![T::default(); dims.iter().product()],
        }
    }
    fn index(&self, x: usize, y: usize, z: usize) -> usize {
        (z * self.dims[1] + y) * self.dims[0] + x
    }
    fn coords(&self, i: usize) -> [usize; 3] {
        [
            i % self.dims[0],
            (i / self.dims[0]) % self.dims[1],
            i / (self.dims[0] * self.dims[1]),
        ]
    }
    fn position(&self, i: usize) -> Vec3 {
        let c = self.coords(i);
        (Vec3::new(c[0] as f32, c[1] as f32, c[2] as f32) + 0.5)
            / Vec3::new(
                self.dims[0] as f32,
                self.dims[1] as f32,
                self.dims[2] as f32,
            )
            * self.half
            * 2.0
            - self.half
    }
    fn cell(&self) -> Vec3 {
        self.half * 2.0
            / Vec3::new(
                self.dims[0] as f32,
                self.dims[1] as f32,
                self.dims[2] as f32,
            )
    }
    fn interpolation(&self, p: Vec3) -> ([usize; 8], [f32; 8]) {
        let q = ((p + self.half) / self.cell() - 0.5).clamp(
            Vec3::ZERO,
            Vec3::new(
                (self.dims[0] - 1) as f32,
                (self.dims[1] - 1) as f32,
                (self.dims[2] - 1) as f32,
            ),
        );
        let b = q.floor().as_uvec3();
        let f = q - q.floor();
        let mut indices = [0; 8];
        let mut weights = [0.0; 8];
        for k in 0..8 {
            let x = k & 1;
            let y = (k >> 1) & 1;
            let z = k >> 2;
            indices[k] = self.index(
                (b.x as usize + x).min(self.dims[0] - 1),
                (b.y as usize + y).min(self.dims[1] - 1),
                (b.z as usize + z).min(self.dims[2] - 1),
            );
            weights[k] = if x == 0 { 1.0 - f.x } else { f.x }
                * if y == 0 { 1.0 - f.y } else { f.y }
                * if z == 0 { 1.0 - f.z } else { f.z };
        }
        (indices, weights)
    }
}
impl Grid<Vec3> {
    fn centered(&self) -> Self {
        let mut out = self.clone();
        out.data.par_iter_mut().enumerate().for_each(|(i, v)| {
            let c = self.coords(i);
            for a in 0..3 {
                let mut prev = c;
                let before = if c[a] == 0 {
                    0.0
                } else {
                    prev[a] -= 1;
                    self.data[self.index(prev[0], prev[1], prev[2])][a]
                };
                v[a] = 0.5 * (before + self.data[i][a]);
            }
        });
        out
    }
    fn vorticity(&self) -> Self {
        let centered = self.centered();
        let h = self.cell();
        let mut out = self.clone();
        out.data.par_iter_mut().enumerate().for_each(|(i, v)| {
            let c = self.coords(i);
            let mut derivatives = [Vec3::ZERO; 3];
            for a in 0..3 {
                let mut lo = c;
                let mut hi = c;
                lo[a] = c[a].saturating_sub(1);
                hi[a] = (c[a] + 1).min(self.dims[a] - 1);
                derivatives[a] = (centered.data[self.index(hi[0], hi[1], hi[2])]
                    - centered.data[self.index(lo[0], lo[1], lo[2])])
                    / (2.0 * h[a]);
            }
            *v = Vec3::new(
                derivatives[1].z - derivatives[2].y,
                derivatives[2].x - derivatives[0].z,
                derivatives[0].y - derivatives[1].x,
            );
        });
        out
    }
    fn confinement(&self, i: usize) -> Vec3 {
        let c = self.coords(i);
        let h = self.cell();
        let mut gradient = Vec3::ZERO;
        for a in 0..3 {
            let mut lo = c;
            let mut hi = c;
            lo[a] = c[a].saturating_sub(1);
            hi[a] = (c[a] + 1).min(self.dims[a] - 1);
            gradient[a] = (self.data[self.index(hi[0], hi[1], hi[2])].length()
                - self.data[self.index(lo[0], lo[1], lo[2])].length())
                / (2.0 * h[a]);
        }
        gradient.normalize_or_zero().cross(self.data[i]) * h.min_element()
    }
    fn sample_centered(&self, p: Vec3) -> Vec3 {
        let (i, w) = self.interpolation(p);
        (0..8).map(|k| self.data[i[k]] * w[k]).sum()
    }
    fn sample(&self, p: Vec3) -> Vec3 {
        // Each component has its own staggered lattice: positive cell faces,
        // not cell centres. Sampling all three at p was a half-cell phase
        // error which made a projected velocity compress transported smoke.
        let mut v = Vec3::ZERO;
        for a in 0..3 {
            v[a] = self.sample_component(p, a);
        }
        v
    }
    fn sample_component(&self, p: Vec3, a: usize) -> f32 {
        let h = self.cell();
        let mut q = p;
        q[a] -= 0.5 * h[a];
        let (indices, weights) = self.interpolation(q);
        (0..8)
            .map(|k| self.data[indices[k]][a] * weights[k])
            .sum::<f32>()
            * ((p[a] + self.half[a]) / h[a]).clamp(0.0, 1.0)
    }
    fn divergence(&self) -> Vec<f32> {
        let h = self.cell();
        (0..self.data.len())
            .map(|i| {
                let c = self.coords(i);
                let v = self.data[i];
                let mut d = 0.0;
                for a in 0..3 {
                    let mut prev = c;
                    let before = if c[a] == 0 {
                        0.0
                    } else {
                        prev[a] -= 1;
                        self.data[self.index(prev[0], prev[1], prev[2])][a]
                    };
                    d += (v[a] - before) / h[a];
                }
                d
            })
            .collect()
    }
    fn project(&mut self, iterations: usize) {
        // Components live on positive cell faces. Closed normal boundaries;
        // Neumann pressure, forward gradient/backward divergence pair.
        for i in 0..self.data.len() {
            let c = self.coords(i);
            for a in 0..3 {
                if c[a] + 1 == self.dims[a] {
                    self.data[i][a] = 0.0;
                }
            }
        }
        let divergence = self.divergence();
        let cell_inverse = 1.0 / self.cell();
        let inv = cell_inverse * cell_inverse;
        let mut pressure = vec![0.0; self.data.len()];
        let diagonal: Vec<f32> = (0..self.data.len())
            .map(|i| {
                let c = self.coords(i);
                (0..3)
                    .map(|a| inv[a] * ((c[a] > 0) as u8 + (c[a] + 1 < self.dims[a]) as u8) as f32)
                    .sum()
            })
            .collect();
        // Jacobi left large, low-frequency divergence on elongated domains.
        // Diagonally preconditioned CG solves the same Neumann Poisson system
        // to a residual tolerance rather than amplifying those errors in dye.
        let mean =
            divergence.iter().map(|v| *v as f64).sum::<f64>() as f32 / divergence.len() as f32;
        let mut residual: Vec<f32> = divergence.iter().map(|d| mean - d).collect();
        let mut direction: Vec<f32> = residual.iter().zip(&diagonal).map(|(r, d)| r / d).collect();
        let mut product = vec![0.0; self.data.len()];
        let dot = |a: &[f32], b: &[f32]| {
            a.iter()
                .zip(b)
                .map(|(a, b)| *a as f64 * *b as f64)
                .sum::<f64>()
        };
        let initial = dot(&residual, &residual);
        let mut rz = dot(&residual, &direction);
        for _ in 0..iterations {
            if dot(&residual, &residual) <= initial * 1e-6 || rz < 1e-20 {
                break;
            }
            product.par_iter_mut().enumerate().for_each(|(i, p)| {
                let c = self.coords(i);
                let mut sum = diagonal[i] * direction[i];
                let sy = self.dims[0];
                let sz = sy * self.dims[1];
                if c[0] > 0 {
                    sum -= inv.x * direction[i - 1];
                }
                if c[0] + 1 < self.dims[0] {
                    sum -= inv.x * direction[i + 1];
                }
                if c[1] > 0 {
                    sum -= inv.y * direction[i - sy];
                }
                if c[1] + 1 < self.dims[1] {
                    sum -= inv.y * direction[i + sy];
                }
                if c[2] > 0 {
                    sum -= inv.z * direction[i - sz];
                }
                if c[2] + 1 < self.dims[2] {
                    sum -= inv.z * direction[i + sz];
                }
                *p = sum;
            });
            let alpha = (rz / dot(&direction, &product).max(1e-30)) as f32;
            pressure
                .par_iter_mut()
                .zip(residual.par_iter_mut())
                .enumerate()
                .for_each(|(i, (p, r))| {
                    *p += alpha * direction[i];
                    *r -= alpha * product[i];
                });
            let next_rz = residual
                .iter()
                .zip(&diagonal)
                .map(|(r, d)| *r as f64 * *r as f64 / *d as f64)
                .sum::<f64>();
            let beta = (next_rz / rz) as f32;
            direction
                .par_iter_mut()
                .enumerate()
                .for_each(|(i, d)| *d = residual[i] / diagonal[i] + beta * *d);
            rz = next_rz;
        }
        for i in 0..self.data.len() {
            let c = self.coords(i);
            for a in 0..3 {
                if c[a] + 1 < self.dims[a] {
                    let mut n = c;
                    n[a] += 1;
                    self.data[i][a] -=
                        (pressure[self.index(n[0], n[1], n[2])] - pressure[i]) * cell_inverse[a];
                }
            }
        }
    }
}
impl Grid<[f32; 4]> {
    fn sample(&self, p: Vec3) -> Vec4 {
        if p.abs().cmpgt(self.half).any() {
            return Vec4::ZERO;
        }
        let (i, w) = self.interpolation(p);
        (0..8)
            .map(|k| Vec4::from_array(self.data[i[k]]) * w[k])
            .sum()
    }
}
fn hash(mut x: u32) -> u32 {
    x = (x ^ (x >> 16)).wrapping_mul(0x7feb352d);
    x = (x ^ (x >> 15)).wrapping_mul(0x846ca68b);
    x ^ (x >> 16)
}
fn random(state: &mut u32) -> f32 {
    *state = hash(state.wrapping_add(0x9e3779b9));
    (*state >> 8) as f32 / 16777216.0
}
fn normal(state: &mut u32) -> f32 {
    (-2.0 * random(state).max(1e-6).ln()).sqrt() * (std::f32::consts::TAU * random(state)).cos()
}
#[inline]
fn noise(p: Vec3, seed: u32) -> (f32, Vec3) {
    noise_impl(p, seed, false)
}
#[inline]
fn noise_impl(p: Vec3, seed: u32, periodic: bool) -> (f32, Vec3) {
    let b = p.floor().as_ivec3();
    let f = p - p.floor();
    let t = f * f * (3.0 - 2.0 * f);
    let dt = 6.0 * f * (1.0 - f);
    let mut v = 0.0;
    let mut g = Vec3::ZERO;
    for k in 0..8 {
        let x = k & 1;
        let y = (k >> 1) & 1;
        let z = k >> 2;
        let mask = if periodic { 15 } else { u32::MAX };
        let h = hash(
            ((b.x + x) as u32 & mask)
                ^ hash((b.y + y) as u32 & mask)
                ^ hash(((b.z + z) as u32 & mask) ^ seed),
        );
        let value = (h >> 8) as f32 / 16777216.0;
        let w = Vec3::new(
            if x == 0 { 1.0 - t.x } else { t.x },
            if y == 0 { 1.0 - t.y } else { t.y },
            if z == 0 { 1.0 - t.z } else { t.z },
        );
        let d = Vec3::new(
            if x == 0 { -dt.x } else { dt.x },
            if y == 0 { -dt.y } else { dt.y },
            if z == 0 { -dt.z } else { dt.z },
        );
        v += value * w.x * w.y * w.z;
        g += value * Vec3::new(d.x * w.y * w.z, w.x * d.y * w.z, w.x * w.y * d.z);
    }
    (v, g)
}
fn curl(p: Vec3, seed: u32) -> Vec3 {
    let mut v = Vec3::ZERO;
    for octave in 0..3 {
        let scale = 0.35 * 2.1f32.powi(octave);
        let q = p * scale;
        let a = noise(q, seed + octave as u32 * 97).1;
        let b = noise(q + Vec3::splat(11.7), seed + 31).1;
        let c = noise(q + Vec3::splat(23.1), seed + 71).1;
        v += Vec3::new(c.y - b.z, a.z - c.x, b.x - a.y) * 0.65f32.powi(octave);
    }
    v
}
fn advect_velocity(old: &Grid<Vec3>, dt: f32) -> Grid<Vec3> {
    let h = old.cell();
    let mut forward = old.clone();
    let mut departures = vec![[Vec3::ZERO; 3]; old.data.len()];
    forward
        .data
        .par_iter_mut()
        .zip(departures.par_iter_mut())
        .enumerate()
        .for_each(|(i, (v, departures))| {
            let center = old.position(i);
            for a in 0..3 {
                let mut face = center;
                face[a] += 0.5 * h[a];
                let mid = face - old.sample(face) * (0.5 * dt);
                departures[a] = face - old.sample(mid) * dt;
                v[a] = old.sample_component(departures[a], a);
            }
        });
    let mut corrected = forward.clone();
    corrected
        .data
        .par_iter_mut()
        .enumerate()
        .for_each(|(i, v)| {
            let center = old.position(i);
            for a in 0..3 {
                let mut face = center;
                face[a] += 0.5 * h[a];
                let back_mid = face + old.sample(face) * (0.5 * dt);
                let reverse = forward.sample_component(face + old.sample(back_mid) * dt, a);
                let mut departure = departures[i][a];
                departure[a] -= 0.5 * h[a];
                let (indices, _) = old.interpolation(departure);
                let lo = indices
                    .iter()
                    .map(|i| old.data[*i][a])
                    .fold(f32::INFINITY, f32::min);
                let hi = indices
                    .iter()
                    .map(|i| old.data[*i][a])
                    .fold(f32::NEG_INFINITY, f32::max);
                // Limited MacCormack: recover rotational energy without creating
                // new extrema or the explosive folds of excessive confinement.
                v[a] = (forward.data[i][a] + 0.5 * (old.data[i][a] - reverse)).clamp(lo, hi);
            }
        });
    corrected
}
fn detail_velocity(n: &Nebula) -> Grid<Vec3> {
    let longest = n.half_size.max_element();
    let dims = n
        .half_size
        .to_array()
        .map(|a| ((192.0 * a / longest).ceil() as usize).clamp(16, 192));
    let mut potential: Grid<Vec3> = Grid::new(dims, n.half_size);
    let h = potential.cell();
    let half = n.half_size;
    potential
        .data
        .par_iter_mut()
        .enumerate()
        .for_each(|(i, v)| {
            let c = Vec3::new(
                (i % dims[0]) as f32,
                ((i / dims[0]) % dims[1]) as f32,
                (i / (dims[0] * dims[1])) as f32,
            );
            let center = (c + 0.5) * h - half;
            for a in 0..3 {
                let mut edge = center + h * 0.5;
                edge[a] = center[a];
                let fade = (1.0 - (edge / half).abs().max_element()).max(0.0).min(0.25) * 4.0;
                let q = edge * n.detail * 3.0 + Vec3::splat(a as f32 * 17.3);
                v[a] = ((noise(q, n.seed + 991).0 - 0.5)
                    + 0.3 * (noise(q * 2.1, n.seed + 1193).0 - 0.5))
                    * fade
                    * fade
                    * 0.035
                    * n.turbulence
                    / n.detail;
            }
        });
    let mut velocity = potential.clone();
    velocity.data.par_iter_mut().enumerate().for_each(|(i, v)| {
        let c = potential.coords(i);
        let mut derivatives = [Vec3::ZERO; 3];
        for a in 0..3 {
            let mut prev = c;
            let before = if c[a] == 0 {
                Vec3::ZERO
            } else {
                prev[a] -= 1;
                potential.data[potential.index(prev[0], prev[1], prev[2])]
            };
            derivatives[a] = (potential.data[i] - before) / h[a];
        }
        // Discrete curl on staggered edge/face lattices. Backward divergence
        // cancels exactly: upsampled eddies do not compress gas into sheets.
        *v = Vec3::new(
            derivatives[1].z - derivatives[2].y,
            derivatives[2].x - derivatives[0].z,
            derivatives[0].y - derivatives[1].x,
        );
    });
    velocity
}

fn filament_density(material: Vec3, cell: Vec3, n: &Nebula, p: &Particles) -> [f32; 4] {
    if p.remnant {
        return remnant_density(material, cell, n, p);
    }
    if p.storm {
        return storm_density(material, cell, n, p);
    }
    if p.spider {
        return spider_density(material, cell, n, p);
    }
    let r = material / n.half_size;
    let envelope = (1.0 - r.length_squared()).max(0.0).powi(2);
    if envelope < 1e-5 {
        return [0.0; 4];
    }
    let q = material * Vec3::new(0.18, 0.45, 0.55) * n.detail;
    let warp = Vec3::new(
        noise(q * 0.65, n.seed + 31).0,
        noise(q * 0.65 + Vec3::splat(11.0), n.seed + 71).0,
        noise(q * 0.65 + Vec3::splat(23.0), n.seed + 113).0,
    ) - 0.5;
    let w = q + warp * n.turbulence * 1.7;
    let mut threads = 0.0;
    for octave in 0..5 {
        let frequency = 2.0f32.powi(octave);
        let a = noise(w * frequency, n.seed + 811 + octave as u32 * 97).0 - 0.5;
        let b = noise(
            w * frequency + Vec3::new(17.3, 5.9, 11.1),
            n.seed + 971 + octave as u32 * 131,
        )
        .0 - 0.5;
        // Intersection of two scalar bands produces tubular filaments, not
        // broad isosurfaces. Filter their cross section to the voxel size;
        // preserve integrated density when widening an unresolved strand.
        let width = 0.055f32;
        let footprint = (cell * Vec3::new(0.18, 0.45, 0.55) * n.detail * frequency).length() * 0.35;
        let variance = width * width + footprint * footprint / 12.0;
        threads += (-0.5 * (a * a + b * b) / variance).exp()
            * (width * width / variance)
            * 0.78f32.powi(octave);
    }
    let cloud = noise(w * 0.75, n.seed + 311).0;
    let gas = envelope * (0.018 + threads * 9.0) * (0.25 + cloud * 1.5);
    let lane = material.y - 0.5 * (material.x * 0.35).sin() - warp.x * 2.5;
    let dust = envelope
        * ((-lane * lane * 1.7).exp() * (0.25 + cloud * cloud * 2.0)
            + ((noise(w * 1.3, n.seed + 503).0 - 0.55) * 8.0).clamp(0.0, 1.0) * 0.12)
        * (0.25 + 0.75 * ((material.z / n.half_size.z + 0.4) * 1.2).clamp(0.0, 1.0));
    let warm =
        ((material.x + 3.0) / 7.0).clamp(0.0, 1.0) * (0.7 - 0.18 * material.y).clamp(0.0, 1.0);
    [gas, dust, gas * warm, 0.0]
}

// A bounded compression proxy for distinct streams occupying the same voxel.
// Parallel transport does not excite it. Stream densities include their depth
// falloff: two ribbons crossing only in projection cannot create a hot knot.
// This is an artistic source model, not an MHD/thermal evolution solver.
fn stream_convergence(streams: &[(f32, Vec3)]) -> f32 {
    let mass: f32 = streams.iter().map(|s| s.0).sum();
    let mut encounters = 0.0;
    for (i, &(a, va)) in streams.iter().enumerate() {
        for &(b, vb) in &streams[i + 1..] {
            encounters += a * b * (1.0 - va.dot(vb)).clamp(0.0, 2.0);
        }
    }
    (encounters / mass.max(1e-8)).clamp(0.0, 2.0)
}

fn filtered_front(distance: f32, width: f32, footprint: f32) -> f32 {
    let variance = width * width + footprint * footprint / 12.0;
    (-0.5 * distance * distance / variance).exp() * width / variance.sqrt()
}

// Gaussian convolution of two intersecting scalar bands. The full covariance
// respects anisotropic voxels and the local band orientation, instead of
// blurring fine XY tubes by the largest (depth) dimension of every voxel.
fn filtered_tube(a: f32, b: f32, da: Vec3, db: Vec3, width: f32) -> f32 {
    let aa = width * width + da.length_squared() / 12.0;
    let bb = width * width + db.length_squared() / 12.0;
    let ab = da.dot(db) / 12.0;
    let det = (aa * bb - ab * ab).max(width.powi(4));
    (-0.5 * (bb * a * a - 2.0 * ab * a * b + aa * b * b).max(0.0) / det).exp() * width * width
        / det.sqrt()
}

fn remnant_density(material: Vec3, cell: Vec3, n: &Nebula, p: &Particles) -> [f32; 4] {
    let r = material / n.half_size;
    let fade = ((1.0 - r.length_squared()) / 0.38).clamp(0.0, 1.0);
    let envelope = fade * fade * (3.0 - 2.0 * fade);
    if envelope < 1e-5 {
        return [0.0; 4];
    }
    let broad = noise(material * Vec3::new(0.11, 0.16, 0.22), n.seed + 3001).0;
    let spine = 1.4 * (material.x * 0.17).sin() + material.x * 0.10;
    let spread = (p.dispersion / 1.1).sqrt().clamp(0.25, 2.0);
    let x = material.x;
    let stream = |y: f32, z: f32, width: f32, cx: f32, extent: f32| {
        (-0.5 * ((material.y - y) / (width * spread)).powi(2)
            - 0.5 * ((material.z - z) / (n.half_size.z * 0.48)).powi(2)
            - 0.5 * ((x - cx) / extent).powi(2))
        .exp()
    };
    let slope = 0.238 * (x * 0.17).cos() + 0.1;
    let streams = [
        (
            stream(spine + 2.3, 0.45 * (x * 0.31).sin(), 2.1, -1.5, 10.0),
            Vec3::new(1.0, slope, 0.14 * (x * 0.31).cos()).normalize(),
        ),
        (
            stream(spine - 3.0, -0.5 + 0.5 * (x * 0.25).sin(), 2.5, 0.0, 11.0),
            Vec3::new(-1.0, -slope, -0.125 * (x * 0.25).cos()).normalize(),
        ),
        (
            stream(
                -0.42 * x - 1.8,
                0.2 + 0.3 * (x * 0.36).sin(),
                1.35,
                -4.5,
                6.5,
            ),
            Vec3::new(1.0, -0.42, 0.108 * (x * 0.36).cos()).normalize(),
        ),
        (
            stream(0.55 * x - 1.0, 0.25 * (x * 0.4).sin(), 1.6, 4.0, 6.0),
            Vec3::new(-1.0, -0.55, -0.1 * (x * 0.4).cos()).normalize(),
        ),
    ];
    let stream_mass: f32 = streams.iter().map(|s| s.0).sum();
    let convergence = stream_convergence(&streams);

    // Nearly isotropic microstructure produces clouds and tubes rather than
    // pleats from a strongly stretched noise coordinate system. Broad warp
    // and projected advection provide the larger curling motion.
    let scale = Vec3::new(0.38, 0.41, 0.43) * n.detail;
    let q = material * scale;
    let warp_noise = [
        noise(q * 0.30, n.seed + 4037),
        noise(q * 0.30 + Vec3::splat(11.0), n.seed + 4049),
        noise(q * 0.30 + Vec3::splat(23.0), n.seed + 4061),
    ];
    let warp = Vec3::new(warp_noise[0].0, warp_noise[1].0, warp_noise[2].0) - 0.5;
    let q = q + warp * n.turbulence * 0.8;
    let mut threads = 0.0;
    let mut cloud = 0.0;
    for octave in 0..6 {
        let frequency = 2.0f32.powi(octave);
        let a = noise(q * frequency, n.seed + 4109 + octave as u32 * 97);
        let b = noise(
            q * frequency + Vec3::new(17.3, 5.9, 11.1),
            n.seed + 4203 + octave as u32 * 131,
        );
        let footprint = |g: Vec3| {
            (g + (warp_noise[0].1 * g.x + warp_noise[1].1 * g.y + warp_noise[2].1 * g.z)
                * (n.turbulence * 0.24))
                * scale
                * frequency
                * cell
        };
        threads += filtered_tube(a.0 - 0.5, b.0 - 0.5, footprint(a.1), footprint(b.1), 0.016)
            * 0.84f32.powi(octave);
        cloud += b.0 * 0.53f32.powi(octave) * 0.47;
    }
    let pocket = gas_distribution(material, n.seed + 503, p.gas_clumping);
    let cloud_mass = (cloud * 1.85).clamp(0.0, 1.6).powf(p.dust_clumping);

    // The expanding ellipsoid is roughened in angular space: coherent torn
    // arcs survive at different radii and depths, with gaps rather than a
    // uniform glowing ring. The reverse shock traces a second broken front.
    let delta = (material - n.core_position) / Vec3::new(1.6, 1.0, 0.65);
    let radius = delta.length();
    let direction = delta / radius.max(1e-5);
    let angular = noise(direction * 3.2 + Vec3::splat(4.7), n.seed + 4409).0;
    let fractures = noise(direction * 9.0 + Vec3::splat(13.1), n.seed + 4421).0;
    let shell_radius = p.shock_radius * (1.0 + (angular - 0.5) * 0.22 + (fractures - 0.5) * 0.045);
    let signed = radius - shell_radius;
    let footprint = (cell / Vec3::new(1.6, 1.0, 0.65)).length();
    let broken =
        ((angular - 0.41) * 5.0).clamp(0.0, 1.0) * ((fractures - 0.38) * 4.0).clamp(0.0, 1.0);
    let forward = filtered_front(signed, p.front_width * 1.8, footprint);
    let reverse = filtered_front(radius - shell_radius * 0.76, p.front_width * 1.1, footprint);
    let shock_mass = envelope * broken * (forward + reverse * 0.45) * (0.045 + 0.8 * threads);
    let cavity = 1.0 - ((signed + 0.65) / 1.3).clamp(0.0, 1.0);
    let scars = filtered_front(fractures - 0.51, 0.015, footprint / p.shock_radius)
        * ((radius / shell_radius - 0.35) * 2.0).clamp(0.0, 1.0);

    let mist = (0.20 + cloud_mass) * (0.45 + 0.55 * broad);
    let gas = envelope
        * p.vapour
        * pocket
        * (stream_mass * (0.008 + threads * 0.65 + cloud_mass * 0.045) + mist * 0.020)
        * (1.0 - cavity * 0.48)
        + shock_mass;
    let rift = material.y - spine + (noise(q * 0.7, n.seed + 4301).0 - 0.5) * 1.4;
    let dusty_rift = (-0.5 * (rift / 0.8).powi(2)).exp();
    let knots = ((cloud - 0.49) * 7.0).clamp(0.0, 1.0);
    let grain = ((noise(q * 2.0, n.seed + 4313).0 - 0.5) * 2.8).exp();
    let dust = envelope
        * cloud_mass
        * grain
        * (0.65 + 0.35 * r.z).clamp(0.2, 1.0)
        * (dusty_rift * 0.45 + stream_mass * knots * 0.16 + scars * 0.24)
        * (1.0 - cavity * 0.72);
    let warmth =
        ((streams[0].0 * 0.96 + streams[1].0 * 0.04 + streams[2].0 * 0.68 + streams[3].0 * 0.20)
            / stream_mass.max(1e-8)
            + (broad - 0.5) * 0.25)
            .clamp(0.02, 0.98);
    // Store emissive power as an associated mass moment, not a naked gain
    // interpolated across empty voxels (which would make transparency halos).
    let excited = gas * p.convergence * convergence.powi(2) * (0.12 + threads * 4.0)
        + shock_mass * p.shock * 1.5;
    [gas, dust, gas * warmth, excited]
}

fn storm_density(material: Vec3, cell: Vec3, n: &Nebula, p: &Particles) -> [f32; 4] {
    // A separate composition source: broad branching streams, a curved dust
    // rift and fine tubes aligned with the local stream. These are 3D fields
    // transported by the same projected velocity as the other presets.
    let r = material / n.half_size;
    let fade = ((1.0 - r.length_squared()) / 0.34).clamp(0.0, 1.0);
    let envelope = fade * fade * (3.0 - 2.0 * fade);
    if envelope < 1e-5 {
        return [0.0; 4];
    }
    let broad = noise(material * Vec3::new(0.11, 0.16, 0.12), n.seed + 3001).0;
    let spine =
        1.4 * (material.x * 0.17).sin() + material.x * 0.10 + (broad - 0.5) * n.turbulence * 1.3;
    let side = material.y - spine;
    let spread = (p.dispersion / 1.1).sqrt().clamp(0.25, 2.0);
    let upper = (-0.5 * ((side - 2.2) / (2.1 * spread)).powi(2)).exp();
    let lower = (-0.5 * ((side + 2.3) / (2.8 * spread)).powi(2)).exp();
    let branch_y = -3.0 - (material.x + 4.0) * 0.32;
    let branch = (-0.5 * ((material.y - branch_y) / (1.35 * spread)).powi(2)
        - 0.5 * ((material.x + 6.0) / 5.5).powi(2))
    .exp();
    let streams = upper * 0.95 + lower * 0.72 + branch * 0.65;
    let q = Vec3::new(material.x * 0.12, side * 0.43, material.z * 0.16) * n.detail;
    let warp = Vec3::new(
        noise(q * 0.5, n.seed + 3037).0,
        noise(q * 0.5 + Vec3::splat(11.0), n.seed + 3049).0,
        noise(q * 0.5 + Vec3::splat(23.0), n.seed + 3061).0,
    ) - 0.5;
    let q = q + warp * n.turbulence * 0.65;
    let mut threads = 0.0;
    let mut cloud = 0.0;
    for octave in 0..6 {
        let frequency = 2.0f32.powi(octave);
        let a = noise(q * frequency, n.seed + 3109 + octave as u32 * 97).0;
        let b = noise(
            q * frequency + Vec3::new(17.3, 5.9, 11.1),
            n.seed + 3203 + octave as u32 * 131,
        )
        .0;
        let width = 0.030f32;
        // Include the slope and roughness of the curved stream in the bound
        // on the pixel/voxel footprint, retaining unresolved tube mass.
        let footprint = (cell * Vec3::new(0.32, 0.43, 0.16) * n.detail * frequency).length() * 0.45;
        let variance = width * width + footprint * footprint / 12.0;
        threads +=
            (-0.5 * ((a - 0.5).powi(2) + (b - 0.5).powi(2)) / variance).exp() * width * width
                / variance
                * 0.80f32.powi(octave);
        cloud += b * 0.53f32.powi(octave) * 0.47;
    }
    let pocket = gas_distribution(material, n.seed + 503, p.gas_clumping);
    let cloud_mass = (cloud * 1.85).clamp(0.0, 1.6).powf(p.dust_clumping);
    let rift_width = 0.45 + 0.65 * (-0.5 * ((material.x - 1.0) / 5.0).powi(2)).exp();
    let rift = side + (noise(q * 0.7, n.seed + 3301).0 - 0.5) * 0.85;
    let grain = ((noise(q * 2.0, n.seed + 3313).0 - 0.5) * 3.0).exp();
    let dusty_rift = (-0.5 * (rift / rift_width).powi(2)).exp();
    // Molecular knots occur throughout the streams. Their depth bias puts
    // absorbing matter in front of some gas while leaving other gas exposed.
    let knots = ((cloud - 0.49) * 7.0).clamp(0.0, 1.0);
    let depth_bias = (0.65 + 0.35 * r.z).clamp(0.2, 1.0);
    let dust =
        envelope * cloud_mass * grain * depth_bias * (dusty_rift * 0.90 + streams * knots * 0.22);
    let sigma = (p.front_width.powi(2) + cell.max_element().powi(2) / 12.0).sqrt();
    let skin =
        (-0.5 * ((rift.abs() - rift_width * 1.65) / sigma).powi(2)).exp() * p.front_width / sigma;
    let to_core = material - n.core_position;
    let cleared = 1.0 - 0.8 * (-0.5 * to_core.truncate().length_squared() / 2.0).exp();
    let haze = envelope
        * streams
        * pocket
        * p.vapour
        * cleared
        * (0.003 + threads * 1.5 + cloud_mass * 0.012);
    let front = envelope * skin * (0.12 + threads * 1.8) * (0.3 + cloud);
    let gas = haze + front;
    let warm = (0.45 + side * 0.15 + material.x * 0.025 + (broad - 0.5) * 0.18).clamp(0.02, 0.98);
    [gas, dust, gas * warm, front]
}

fn spider_envelope(material: Vec3, n: &Nebula) -> f32 {
    // A finite cloud needs its own silhouette when viewed from a distance.
    // The texture box is a storage boundary, not the outline of the nebula.
    // Two overlapping lobes retain the warm concentrations on either side;
    // their noisy, feathered perimeter is transported in material space.
    let r = material / n.half_size;
    let a = ((r.truncate() - glam::Vec2::new(-0.20, -0.035)) / glam::Vec2::new(0.68, 0.78))
        .length()
        - 1.0;
    let b =
        ((r.truncate() - glam::Vec2::new(0.44, 0.37)) / glam::Vec2::new(0.43, 0.53)).length() - 1.0;
    let outline = a.min(b);
    if outline > 0.16 || r.abs().max_element() >= 1.0 {
        return 0.0;
    }
    let edge = if outline < -0.40 {
        1.0
    } else {
        let q = r * Vec3::new(3.1, 3.7, 0.6);
        let rough = (noise(q, n.seed + 2203).0 - 0.5) * 0.18
            + (noise(q * 2.3 + Vec3::splat(8.7), n.seed + 2297).0 - 0.5) * 0.075;
        let t = ((0.03 - outline - rough) / 0.30).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let fade = |x: f32| {
        let t = ((1.0 - x.abs()) / 0.12).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    edge * fade(r.x) * fade(r.y) * fade(r.z)
}

fn spider_density(material: Vec3, cell: Vec3, n: &Nebula, p: &Particles) -> [f32; 4] {
    // Separate molecular matter, a thin ionized skin and dilute gas. A union
    // of broken-out cavities replaces the old smooth bowl and eight ribbons.
    // The source is procedural in material space; all four density moments
    // are advected and filtered together, never as a camera-facing picture.
    let envelope = spider_envelope(material, n);
    if envelope < 1e-5 {
        return [0.0; 4];
    }
    let d = material - n.core_position;
    let q = material * Vec3::new(0.30, 0.36, 0.12) * n.detail;
    let mut rough = 0.0;
    let mut cloud = 0.0;
    let mut threads = 0.0;
    let mut wisps = 0.0;
    for octave in 0..6 {
        let f = 2.0f32.powi(octave);
        let a = noise(q * f, n.seed + 811 + octave as u32 * 97).0;
        let b = noise(
            q * f + Vec3::new(17.3, 5.9, 11.1),
            n.seed + 971 + octave as u32 * 131,
        )
        .0;
        // Small-scale perturbations roughen the cloud boundary rather than
        // folding a single stretched coordinate system into parallel pleats.
        rough += (a - 0.5) * 0.53f32.powi(octave);
        cloud += b * 0.55f32.powi(octave) * 0.46;
        let width = 0.035f32;
        let footprint = (cell * Vec3::new(0.30, 0.36, 0.12) * n.detail * f).length() * 0.35;
        let variance = width * width + footprint * footprint / 12.0;
        let distance2 = (a - 0.5).powi(2) + (b - 0.5).powi(2);
        threads +=
            (-0.5 * distance2 / variance).exp() * (width * width / variance) * 0.85f32.powi(octave);
        // Narrower tubes in the warm gas break up the inflated pink masses.
        // Their footprint broadens unresolved sections without replacing them
        // by full-width sheets or boosting their integrated mass.
        let wisp_width = 0.022f32;
        let wisp_variance = wisp_width * wisp_width + footprint * footprint / 12.0;
        wisps += (-0.5 * distance2 / wisp_variance).exp()
            * (wisp_width * wisp_width / wisp_variance)
            * 0.85f32.powi(octave);
    }
    let ellipsoid =
        |offset: Vec3, radii: Vec3| (((d - offset) / radii).length() - 1.0) * radii.x.min(radii.y);
    let depth = n.half_size.z * 3.5;
    let cavity = ellipsoid(Vec3::new(-0.5, 0.2, 0.0), Vec3::new(3.6, 2.8, depth))
        .min(ellipsoid(
            Vec3::new(-4.0, -3.4, 0.0),
            Vec3::new(4.1, 2.5, depth),
        ))
        .min(ellipsoid(
            Vec3::new(-1.0, 3.9, 0.0),
            Vec3::new(1.5, 2.7, depth),
        ))
        .min(ellipsoid(
            Vec3::new(10.0, 4.0, 0.0),
            Vec3::new(6.2, 5.0, depth),
        ));
    let cavity = cavity + rough * n.turbulence * 2.1;
    // Porous molecular clouds create additional dark bays, compact knots and
    // sharp illuminated boundaries throughout the field, not only one ring.
    let signed = cavity.min((cloud - 0.39) * 8.0);
    let molecular = ((signed + 0.03) / 0.35).clamp(0.0, 1.0);
    let molecular = molecular * molecular * (3.0 - 2.0 * molecular);
    let clump = (cloud * 1.7).clamp(0.0, 1.5).powf(p.dust_clumping);
    let grain = ((noise(q * 4.0, n.seed + 1597).0 - 0.5) * 4.0
        + (noise(q * 11.0, n.seed + 1723).0 - 0.5) * 2.0
        - 0.2)
        .exp();
    let dust = envelope * molecular * clump * grain * (0.16 + 0.38 * threads);
    let sigma = (p.front_width.powi(2) + cell.max_element().powi(2) / 12.0).sqrt();
    let skin = (-0.5 * (signed / sigma).powi(2)).exp() * (p.front_width / sigma);
    let front = envelope * skin * (0.08 + 2.2 * threads) * (0.4 + cloud);
    let cleared = ((cavity + 0.25) / 0.6).clamp(0.0, 1.0);
    let warm_patch = (-0.075 * d.x + 0.12 * d.y + 0.48 + rough * 0.5).clamp(0.08, 1.0);
    let warm = ((d.truncate().length() - 2.0) / 11.0 + rough * 0.7).clamp(0.0, 1.0) * warm_patch;
    let gas_threads = threads + warm * (wisps - threads);
    // Sparse translucent tendrils persist inside the evacuated bays. A dilute
    // envelope around the molecular boundary softens the silhouette through
    // genuine volume emission/scattering, without blurring its narrow skin.
    let vapour = (-0.5 * (cavity / p.dispersion).powi(2)).exp() * cloud * cloud;
    let haze = envelope
        * p.vapour
        * gas_distribution(material, n.seed, p.gas_clumping)
        * (0.0005
            + gas_threads * 0.10
            + vapour * (0.025 + gas_threads * 0.25)
            + cleared * (0.003 + gas_threads * 0.24 + 0.03 * clump * molecular));
    let gas = haze + front;
    // Associated colour and front moments retain their fractions at empty
    // voxel boundaries, avoiding coloured alpha fringes after interpolation.
    [gas, dust, gas * warm, front]
}

fn gas_distribution(material: Vec3, seed: u32, clumping: f32) -> f32 {
    // Intermittent source mass on scales independent of microfilament detail.
    // A log-density envelope produces coherent rarefactions and dense pockets;
    // increasing fine noise alone would leave equal gas everywhere. It is
    // transported in material space with the other moments, not screen noise.
    let broad = noise(material * Vec3::new(0.10, 0.14, 0.08), seed + 1919).0;
    let medium = noise(
        material * Vec3::new(0.24, 0.22, 0.15) + Vec3::splat(13.7),
        seed + 2017,
    )
    .0;
    let log_density =
        (broad * 0.7 + medium * 0.3 - 0.5) * clumping * 6.0 - 0.20 * clumping * clumping;
    log_density.clamp(-4.0, 2.0).exp()
}
fn simulate_filaments(n: &Nebula, p: &Particles, mut result: Grid<[f32; 4]>) -> Grid<[f32; 4]> {
    let longest = n.half_size.max_element();
    let dims = n
        .half_size
        .to_array()
        .map(|a| ((128.0 * a / longest).ceil() as usize).clamp(16, 128));
    let mut flow: Grid<Vec3> = Grid::new(dims, n.half_size);
    let mut displacement: Grid<Vec3> = Grid::new(dims, n.half_size);
    for i in 0..flow.data.len() {
        for a in 0..3 {
            let mut face = flow.position(i);
            face[a] += 0.5 * flow.cell()[a];
            flow.data[i][a] = curl(face * n.detail * 0.5, n.seed)[a] * n.turbulence;
        }
    }
    flow.project(100);
    let steps = (p.time / 0.125).ceil() as usize;
    let dt = p.time / steps.max(1) as f32;
    for _ in 0..steps {
        let old = flow.clone();
        flow = advect_velocity(&old, dt);
        let omega = old.vorticity();
        flow.data
            .par_iter_mut()
            .enumerate()
            .for_each(|(i, v)| *v += omega.confinement(i) * n.turbulence * 0.45 * dt);
        flow.project(80);
        let previous = displacement.clone();
        displacement
            .data
            .par_iter_mut()
            .enumerate()
            .for_each(|(i, d)| {
                let x = previous.position(i);
                let mid = x - (flow.sample(x) + p.wind) * (0.5 * dt);
                let departure = x - (flow.sample(mid) + p.wind) * dt;
                *d = departure - x + previous.sample_centered(departure);
            });
    }
    let cell = result.cell();
    let decay = (-p.dissipation * p.time).exp();
    let dims = result.dims;
    let half = result.half;
    result.data.par_iter_mut().enumerate().for_each(|(i, v)| {
        let c = Vec3::new(
            (i % dims[0]) as f32,
            ((i / dims[0]) % dims[1]) as f32,
            (i / (dims[0] * dims[1])) as f32,
        );
        let x = (c + 0.5) * cell - half;
        *v = filament_density(x + displacement.sample_centered(x), cell, n, p).map(|a| a * decay);
    });
    result
}
#[derive(Clone, Copy)]
struct Particle {
    position: Vec3,
    mass: f32,
    dust: bool,
    warmth: f32,
    birth: usize,
    birth_fraction: f32,
    temperature: f32,
}
fn density_dimensions(n: &Nebula, p: &Particles) -> [usize; 3] {
    let longest = n.half_size.max_element();
    let mut dims = n
        .half_size
        .to_array()
        .map(|a| ((p.resolution as f32 * a / longest).ceil() as usize).clamp(16, 1536));
    if p.remnant && p.filaments && !p.smoke {
        dims[2] = ((dims[2] as f32 * p.depth_sampling).ceil() as usize).max(16);
    }
    // Two RGBA16F volumes fit in 512 MiB, leaving room for Studio and render
    // buffers on the 4 GiB K3100M. Aspect-correct grids retain finer filaments.
    const MAX_VOXELS: usize = 32 * 1024 * 1024;
    if dims.iter().product::<usize>() > MAX_VOXELS {
        let scale = (MAX_VOXELS as f64 / dims.iter().product::<usize>() as f64).cbrt();
        dims = dims.map(|d| ((d as f64 * scale).floor() as usize).max(16));
    }
    // Minimum axis sizes can defeat uniform rescaling in a very thin domain.
    while dims.iter().product::<usize>() > MAX_VOXELS {
        let axis = (0..3).max_by_key(|a| dims[*a]).unwrap();
        dims[axis] -= 1;
    }
    dims
}
fn simulate(n: &Nebula, p: &Particles) -> Grid<[f32; 4]> {
    let started = Instant::now();
    let longest = n.half_size.max_element();
    let dims = density_dimensions(n, p);
    let mut result: Grid<[f32; 4]> = Grid::new(dims, n.half_size);
    if p.filaments && !p.smoke {
        return simulate_filaments(n, p, result);
    }
    let steps = (p.time / 0.125).ceil() as usize;
    if p.smoke && steps == 0 {
        return result;
    }
    let flow_resolution = if p.smoke { 96.0 } else { 128.0 };
    let flow_dims = n
        .half_size
        .to_array()
        .map(|a| ((flow_resolution * a / longest).ceil() as usize).clamp(16, 128));
    let mut flow: Grid<Vec3> = Grid::new(flow_dims, n.half_size);
    let dt = p.time / steps.max(1) as f32;
    for i in 0..flow.data.len() {
        if !p.smoke {
            let center = flow.position(i);
            for a in 0..3 {
                let mut face = center;
                face[a] += 0.5 * flow.cell()[a];
                flow.data[i][a] = curl(face * n.detail * 0.5, n.seed)[a] * n.turbulence;
            }
        }
    }
    flow.project(64);
    let detail = if p.smoke && n.turbulence > 0.0 {
        Some(detail_velocity(n))
    } else {
        None
    };
    let total_volume = (n.half_size * 2.0).element_product();
    // A nozzle emits a mass flux, independent of the simulation box size.
    // The old full-domain mass concentrated into a tiny emitter produced an
    // almost opaque skin, so every rolled vortex looked like a solid pleat.
    let base_mass = if p.smoke {
        std::f32::consts::TAU * p.emitter_radius.powi(2) * p.inlet_speed.max(0.1) * p.time * 2.0
            / p.count as f32
    } else {
        total_volume / p.count as f32
    };
    let mut particles: Vec<Particle> = (0..p.count)
        .into_par_iter()
        .map(|i| {
            let mut rng = hash(i ^ n.seed.wrapping_mul(131));
            let dust = !p.smoke && random(&mut rng) < 0.17;
            let mut pos = if p.smoke {
                Vec3::new(
                    normal(&mut rng) * p.emitter_radius,
                    -n.half_size.y * 0.75 + normal(&mut rng) * 0.08,
                    normal(&mut rng) * p.emitter_radius,
                )
            } else {
                Vec3::new(
                    (random(&mut rng) * 2.0 - 1.0) * n.half_size.x,
                    normal(&mut rng)
                        * if dust {
                            n.half_size.y * 0.085
                        } else {
                            n.half_size.y * 0.34
                        },
                    normal(&mut rng) * n.half_size.z * 0.44
                        + if dust { n.half_size.z * 0.28 } else { 0.0 },
                )
            };
            pos = pos.clamp(-n.half_size * 0.99, n.half_size * 0.99);
            let envelope = (1.0 - (pos / n.half_size).length_squared())
                .max(0.0)
                .powi(2);
            let source = noise(pos * Vec3::new(0.4, 0.7, 0.25) * n.detail, n.seed + 113).0;
            let source2 = noise(pos * 2.5, n.seed + 311).0;
            let mass = if p.smoke {
                base_mass
            } else {
                base_mass * envelope * (0.025 + source.powi(4) * 5.0) * (0.3 + source2 * 1.4)
            };
            let warmth =
                ((pos.x + 4.0) / 10.0).clamp(0.0, 1.0) * (0.7 - 0.12 * pos.y).clamp(0.0, 1.0);
            Particle {
                position: pos,
                mass,
                dust,
                warmth,
                birth: if p.smoke {
                    (i as usize * steps) / p.count as usize
                } else {
                    0
                },
                // A continuous emitter must not inject one zero-thickness
                // sheet per solver step. Keep substep birth time as well as
                // a volumetric source, avoiding layered/cloth-like smoke.
                birth_fraction: if p.smoke {
                    (i as f32 * steps as f32 / p.count as f32).fract()
                } else {
                    0.0
                },
                temperature: if dust { 0.2 } else { 1.0 },
            }
        })
        .collect();
    for step in 0..steps {
        let old = flow.clone();
        let advected = advect_velocity(&old, dt);
        let omega = old.vorticity();
        let mut thermal = vec![[0.0f32; 2]; flow.data.len()];
        if p.buoyancy != 0.0 {
            for a in particles.iter().filter(|a| a.birth <= step) {
                let (indices, weights) = old.interpolation(a.position);
                for k in 0..8 {
                    thermal[indices[k]][0] += a.mass * weights[k];
                    thermal[indices[k]][1] += a.mass * weights[k] * a.temperature;
                }
            }
        }
        flow.data.par_iter_mut().enumerate().for_each(|(i, v)| {
            let x = old.position(i);
            let rho = thermal[i][0] / old.cell().element_product();
            let temperature = thermal[i][1] / thermal[i][0].max(1e-10);
            // Temperature is an excess over ambient: a sparsely populated
            // cell must not receive the same buoyancy as a dense hot jet.
            let buoyancy = p.buoyancy * temperature * (1.0 - (-rho).exp())
                - 0.02 * rho.min(10.0) * p.buoyancy.abs();
            let force = Vec3::Y * buoyancy + omega.confinement(i) * n.turbulence * 0.65;
            *v = advected.data[i] + force * dt;
            if p.smoke {
                let r = p.emitter_radius;
                let h = x.y + n.half_size.y * 0.75;
                let nozzle = (-0.5 * (x.x * x.x + x.z * x.z) / (r * r) - h * h / 0.08).exp();
                if nozzle > 1e-5 {
                    let pulse = 1.0 + 0.18 * ((step as f32 + 0.5) * dt * 4.7).sin();
                    let jitter = curl(x * 2.0 + Vec3::splat(step as f32 * dt * 0.4), n.seed + 719)
                        * n.turbulence
                        * 0.25;
                    *v = v.lerp(Vec3::Y * p.inlet_speed * pulse + jitter, nozzle * 0.8);
                }
            }
        });
        flow.project(80);
        particles
            .par_iter_mut()
            .filter(|a| a.birth <= step)
            .for_each(|a| {
                let particle_dt = dt
                    * if a.birth == step {
                        1.0 - a.birth_fraction
                    } else {
                        1.0
                    };
                let v = flow.sample(a.position)
                    + p.wind
                    + detail
                        .as_ref()
                        .map(|d| d.sample(a.position))
                        .unwrap_or(Vec3::ZERO);
                let mid = a.position + v * (particle_dt * 0.5);
                a.position += (flow.sample(mid)
                    + p.wind
                    + detail.as_ref().map(|d| d.sample(mid)).unwrap_or(Vec3::ZERO))
                    * particle_dt;
                a.position = a.position.clamp(-n.half_size * 0.999, n.half_size * 0.999);
                // Open-boundary wind must not accumulate matter on the wall.
                if a.position.abs().cmpge(n.half_size * 0.999).any() {
                    a.mass = 0.0;
                }
                a.mass *= (-p.dissipation * particle_dt).exp();
                a.temperature *= (-0.18 * particle_dt).exp();
            });
    }
    let advection_seconds = started.elapsed().as_secs_f64();
    let cell = result.cell();
    let voxel_volume = cell.element_product();
    // CIC deposition followed by a separable Gaussian reconstructs a volume,
    // rather than disconnected opaque particle kernels. It costs 8 deposits
    // per particle, instead of up to 1331, and filters parallel grid rows.
    for a in particles {
        if a.mass == 0.0 {
            continue;
        }
        let q = (a.position + n.half_size) / cell - 0.5;
        let base = q.floor().as_ivec3();
        let fraction = q - q.floor();
        let mut axes = [[(0usize, 0.0f32); 2]; 3];
        for axis in 0..3 {
            let mut sum = 0.0;
            for d in 0..2 {
                let j = (base[axis] as isize + d as isize).clamp(0, dims[axis] as isize - 1);
                let w = if d == 0 {
                    1.0 - fraction[axis]
                } else {
                    fraction[axis]
                };
                sum += w;
                axes[axis][d] = (j as usize, w);
            }
            for (_, w) in &mut axes[axis] {
                *w /= sum.max(1e-10);
            }
        }
        let mass = a.mass / voxel_volume;
        for &(z, wz) in &axes[2] {
            for &(y, wy) in &axes[1] {
                for &(x, wx) in &axes[0] {
                    let i = result.index(x, y, z);
                    let d = mass * wx * wy * wz;
                    if a.dust {
                        result.data[i][1] += d;
                    } else {
                        result.data[i][0] += d;
                        result.data[i][2] += d * a.warmth;
                    }
                }
            }
        }
    }
    gaussian_filter(&mut result, p.radius);
    eprintln!(
        "volume: advection {:.2}s, reconstruction {:.2}s",
        advection_seconds,
        started.elapsed().as_secs_f64() - advection_seconds
    );
    // Keep the colour moment associated with gas density during trilinear
    // filtering. Interpolating an unassociated fraction against empty voxels
    // would create cool/purple halos around otherwise warm, thin filaments.
    result
}
fn gaussian_filter(grid: &mut Grid<[f32; 4]>, radius: f32) {
    let reach = (2.5 * radius).ceil() as isize;
    // Account for CIC's variance (1/6 voxel^2) to retain the requested width.
    let variance = (radius * radius - 1.0 / 6.0).max(0.08);
    let kernel: Vec<f32> = (-reach..=reach)
        .map(|i| (-0.5 * (i * i) as f32 / variance).exp())
        .collect();
    let mut next = grid.data.clone();
    for axis in 0..3 {
        let stride: usize = grid.dims[..axis].iter().product();
        let length = grid.dims[axis] as isize;
        let norms: Vec<f32> = (0..length)
            .map(|c| {
                (-reach..=reach)
                    .filter(|d| c + d >= 0 && c + d < length)
                    .map(|d| kernel[(d + reach) as usize])
                    .sum()
            })
            .collect();
        next.par_iter_mut().enumerate().for_each(|(i, v)| {
            let c = ((i / stride) % grid.dims[axis]) as isize;
            let mut sum = Vec4::ZERO;
            for d in -reach..=reach {
                if c + d < 0 || c + d >= length {
                    continue;
                }
                let j = (i as isize + d * stride as isize) as usize;
                // Normalise each source, not each destination: mass must be
                // preserved at the boundary as well as inside the volume.
                sum += Vec4::from_array(grid.data[j])
                    * (kernel[(d + reach) as usize] / norms[(c + d) as usize]);
            }
            *v = sum.to_array();
        });
        std::mem::swap(&mut grid.data, &mut next);
    }
}
fn extinction(v: Vec4, n: &Nebula) -> f32 {
    let ionized = n
        .particles
        .as_ref()
        .is_some_and(|p| (p.spider || p.storm || p.remnant) && p.filaments && !p.smoke);
    v.x * n.density * (if ionized { 0.08 } else { 0.35 } + n.scattering) * gas_opacity(n)
        + v.y * n.dust
}
fn gas_opacity(n: &Nebula) -> f32 {
    n.particles
        .as_ref()
        .filter(|p| p.remnant && p.filaments && !p.smoke)
        .map_or(1.0, |p| p.gas_opacity)
}
fn transmittance(grid: &Grid<[f32; 4]>, n: &Nebula, origin: Vec3, target: Vec3, steps: u32) -> f32 {
    let delta = target - origin;
    let mut depth = 0.0;
    for j in 0..steps {
        // Resolve the optical skin nearest the shading point. A uniform
        // midpoint march skipped it on fine grids, bleaching dense smoke.
        let a = (j as f32 / steps as f32).powi(2);
        let b = ((j + 1) as f32 / steps as f32).powi(2);
        depth +=
            extinction(grid.sample(origin + delta * (a + b) * 0.5), n) * (b - a) * delta.length();
        if depth > 12.0 {
            return 0.0;
        }
    }
    (-depth).exp()
}
fn directional_visibility(grid: &Grid<[f32; 4]>, n: &Nebula, dir: Vec3) -> Vec<f32> {
    let h = grid.cell();
    let rates = (dir / h).abs();
    let axis = if rates.x >= rates.y && rates.x >= rates.z {
        0
    } else if rates.y >= rates.z {
        1
    } else {
        2
    };
    let sign = if dir[axis] > 0.0 { 1isize } else { -1 };
    let step = h[axis] / dir[axis].abs();
    let mut tau = vec![0.0f32; grid.data.len()];
    let other: Vec<usize> = (0..3).filter(|a| *a != axis).collect();
    let width = grid.dims[other[0]];
    let height = grid.dims[other[1]];
    let mut slice = vec![0.0; width * height];
    // Short characteristics: every voxel gets a shadow sample; no 64-step
    // rays skipping microfilaments. Slices are independent internally and
    // ordered toward the light. Interpolate optical depth, not alpha/RGB.
    for s in 0..grid.dims[axis] {
        let plane = if sign > 0 { grid.dims[axis] - 1 - s } else { s };
        slice.par_iter_mut().enumerate().for_each(|(j, v)| {
            let mut c = [0; 3];
            c[axis] = plane;
            c[other[0]] = j % width;
            c[other[1]] = j / width;
            let i = grid.index(c[0], c[1], c[2]);
            let sigma = extinction(Vec4::from_array(grid.data[i]), n);
            let previous = plane as isize + sign;
            let edge_distance = boundary_distance(grid.position(i), dir, grid.half);
            if edge_distance < 0.5 * step || previous < 0 || previous >= grid.dims[axis] as isize {
                *v = edge_distance.min(0.5 * step) * sigma;
                return;
            }
            let q0 = c[other[0]] as f32 + dir[other[0]] * step / h[other[0]];
            let q1 = c[other[1]] as f32 + dir[other[1]] * step / h[other[1]];
            let b0 = q0.floor() as isize;
            let b1 = q1.floor() as isize;
            let f0 = q0 - q0.floor();
            let f1 = q1 - q1.floor();
            let mut prior = 0.0;
            for k in 0..4 {
                let x = b0 + (k & 1);
                let y = b1 + ((k >> 1) & 1);
                if x < 0 || y < 0 || x >= width as isize || y >= height as isize {
                    continue;
                }
                let mut p = c;
                p[axis] = previous as usize;
                p[other[0]] = x as usize;
                p[other[1]] = y as usize;
                let idx = grid.index(p[0], p[1], p[2]);
                let w =
                    if k & 1 == 0 { 1.0 - f0 } else { f0 } * if k & 2 == 0 { 1.0 - f1 } else { f1 };
                prior +=
                    w * (tau[idx] + 0.5 * step * extinction(Vec4::from_array(grid.data[idx]), n));
            }
            *v = (prior + 0.5 * step * sigma).min(16.0);
        });
        for (j, v) in slice.iter().enumerate() {
            let mut c = [0; 3];
            c[axis] = plane;
            c[other[0]] = j % width;
            c[other[1]] = j / width;
            let i = grid.index(c[0], c[1], c[2]);
            tau[i] = *v;
        }
    }
    tau.par_iter_mut().for_each(|v| *v = (-*v).exp());
    tau
}
fn boundary_distance(origin: Vec3, dir: Vec3, half: Vec3) -> f32 {
    let mut t = f32::INFINITY;
    for a in 0..3 {
        if dir[a].abs() > 1e-6 {
            t = t.min((dir[a].signum() * half[a] - origin[a]) / dir[a]);
        }
    }
    t.max(0.0)
}
fn illuminate(
    grid: &Grid<[f32; 4]>,
    n: &Nebula,
    p: &Particles,
    sun: Option<Vec3>,
) -> Vec<[f32; 4]> {
    let sun_visibility = sun.map(|d| directional_visibility(grid, n, d));
    let light: Vec<[f32; 4]> = (0..grid.data.len())
        .into_par_iter()
        .map(|i| {
            if grid.data[i][0] + grid.data[i][1] < 1e-5 {
                return [1.0, 1.0, 0.0, 0.0];
            }
            let origin = grid.position(i);
            let a = if n.core_energy > 0.0 {
                transmittance(grid, n, origin, n.core_position, p.shadow_steps)
            } else {
                1.0
            };
            let b = sun_visibility.as_ref().map(|v| v[i]).unwrap_or(1.0);
            [a, b, 0.0, 0.0]
        })
        .collect();
    if p.multiple_scattering == 0.0 {
        return light;
    }
    let mut result = light.clone();
    let mut next = result.clone();
    let distance = grid.cell().min_element() * 2.0;
    // A bounded diffusion closure, rather than claiming unbiased multiple
    // scattering. Absorption remains in the local transport/albedo at each
    // iteration. Four cached orders supply low-frequency interior fill.
    for _ in 0..4 {
        next.par_iter_mut().enumerate().for_each(|(i, v)| {
            let density = Vec4::from_array(grid.data[i]);
            let sigma = extinction(density, n);
            let dust_scattering = if (p.spider || p.storm || p.remnant) && p.filaments && !p.smoke {
                density.y * n.dust * 0.15
            } else {
                0.0
            };
            let albedo = ((density.x * n.density * n.scattering * gas_opacity(n)
                + dust_scattering)
                / sigma.max(1e-6))
            .min(0.95);
            let c = grid.coords(i);
            let mut sum = Vec3::ZERO;
            let mut count: f32 = 0.0;
            for a in 0..3 {
                for sign in [-1isize, 1] {
                    let j = c[a] as isize + sign;
                    if j < 0 || j >= grid.dims[a] as isize {
                        continue;
                    }
                    let mut q = c;
                    q[a] = j as usize;
                    let j = grid.index(q[0], q[1], q[2]);
                    sum += Vec3::new(
                        result[j][0] + result[j][2],
                        result[j][1] + result[j][3],
                        0.0,
                    );
                    count += 1.0;
                }
            }
            let indirect = sum / count.max(1.0) * albedo * (1.0 - (-sigma * distance).exp()) * 0.5;
            *v = [light[i][0], light[i][1], indirect.x, indirect.y];
        });
        std::mem::swap(&mut result, &mut next);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn periodic_atlas_field_matches_gradients_and_wraps_in_three_dimensions() {
        let p = Vec3::new(2.31, -3.24, 7.61);
        let (value, gradient) = noise_impl(p, 0x745a13, true);
        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
            let wrapped = noise_impl(p + axis * 16.0, 0x745a13, true);
            assert!((wrapped.0 - value).abs() < 1e-5);
            assert!((wrapped.1 - gradient).length() < 1e-5);
            let epsilon = 0.001;
            let derivative = (noise_impl(p + axis * epsilon, 0x745a13, true).0
                - noise_impl(p - axis * epsilon, 0x745a13, true).0)
                / (2.0 * epsilon);
            assert!((derivative - gradient.dot(axis)).abs() < 0.001);
        }
    }
    #[test]
    fn continuous_smoke_has_no_emitted_mass_at_time_zero() {
        let n = Nebula::default();
        let p = Particles {
            count: 4096,
            resolution: 48,
            time: 0.0,
            smoke: true,
            ..Default::default()
        };
        assert!(simulate(&n, &p).data.iter().all(|v| *v == [0.0; 4]));
    }
    #[test]
    fn volume_keys_separate_view_edits_lighting_and_simulation() {
        let mut scene = Scene::example();
        scene.atmosphere.nebula = Some(Nebula::default());
        let key = particle_key(&scene);
        let n = scene.atmosphere.nebula.as_mut().unwrap();
        let simulation = simulation_key(n, n.particles.as_ref().unwrap());
        n.emission *= 2.0;
        n.cool_color = [0.2; 3];
        n.stars = 0.0;
        n.steps = 96;
        assert_eq!(key, particle_key(&scene));
        let n = scene.atmosphere.nebula.as_mut().unwrap();
        n.dust *= 2.0;
        assert_eq!(simulation, simulation_key(n, n.particles.as_ref().unwrap()));
        assert_ne!(key, particle_key(&scene));
        let n = scene.atmosphere.nebula.as_mut().unwrap();
        n.particles.as_mut().unwrap().time += 0.5;
        assert_ne!(simulation, simulation_key(n, n.particles.as_ref().unwrap()));
    }
    #[test]
    fn pressure_projection_reduces_divergence() {
        let mut g = Grid::new([16, 12, 8], Vec3::splat(2.0));
        for i in 0..g.data.len() {
            g.data[i] = g.position(i).sin();
        }
        let before: f64 = g.divergence().iter().map(|d| (*d as f64).powi(2)).sum();
        g.project(160);
        let after: f64 = g.divergence().iter().map(|d| (*d as f64).powi(2)).sum();
        assert!(after < before * 2e-6, "{before} -> {after}");
    }
    #[test]
    fn staggered_interpolation_reproduces_linear_face_velocities() {
        let mut g: Grid<Vec3> = Grid::new([18, 14, 12], Vec3::new(3.0, 2.0, 1.0));
        for i in 0..g.data.len() {
            g.data[i] = g.position(i) + g.cell() * 0.5;
        }
        for p in [
            Vec3::ZERO,
            Vec3::new(0.13, -0.27, 0.38),
            Vec3::new(-1.17, 0.83, -0.46),
        ] {
            assert!(
                (g.sample(p) - p).length() < 2e-6,
                "{} != {}",
                g.sample(p),
                p
            );
        }
    }
    #[test]
    fn directional_shadow_resolves_a_single_voxel_optical_skin() {
        let n = Nebula::default();
        let mut grid = Grid::new([16, 12, 8], Vec3::splat(2.0));
        grid.data.fill([0.5, 0.0, 0.0, 0.0]);
        let tr = directional_visibility(&grid, &n, Vec3::X);
        for i in 0..tr.len() {
            let expected =
                (-extinction(Vec4::new(0.5, 0.0, 0.0, 0.0), &n) * (2.0 - grid.position(i).x)).exp();
            assert!((tr[i] - expected).abs() < 1e-6);
        }
        grid.data.fill([0.0; 4]);
        let idx = grid.index(8, 6, 4);
        grid.data[idx] = [20.0, 0.0, 0.0, 0.0];
        let tr = directional_visibility(&grid, &n, Vec3::X);
        assert!(
            (tr[idx] - (-20.0 * n.density * (0.35 + n.scattering) * grid.cell().x * 0.5).exp())
                .abs()
                < 1e-6
        );
        assert_eq!(tr[grid.index(9, 6, 4)], 1.0);
        assert!(tr[grid.index(7, 6, 4)] < tr[idx]);
    }
    #[test]
    fn smoke_mass_flux_is_independent_of_box_and_particle_budget() {
        let n = Nebula {
            half_size: Vec3::new(3.0, 4.0, 2.0),
            ..Default::default()
        };
        let p = Particles {
            count: 4096,
            resolution: 48,
            time: 0.25,
            smoke: true,
            buoyancy: 0.0,
            dissipation: 0.0,
            ..Default::default()
        };
        let expected =
            std::f32::consts::TAU * p.emitter_radius.powi(2) * p.inlet_speed * p.time * 2.0;
        for (half, count) in [(n.half_size, 4096), (n.half_size * 1.5, 8192)] {
            let g = simulate(
                &Nebula {
                    half_size: half,
                    ..n.clone()
                },
                &Particles { count, ..p.clone() },
            );
            let mass = g.data.iter().map(|v| v[0] + v[1]).sum::<f32>() * g.cell().element_product();
            assert!(
                (mass - expected).abs() / expected < 1e-4,
                "{mass} vs {expected}"
            );
        }
    }
    #[test]
    fn continuous_filaments_are_budget_independent_and_move_with_flow() {
        let n = Nebula {
            half_size: Vec3::new(4.0, 2.0, 0.8),
            ..Default::default()
        };
        let p = Particles {
            count: 4096,
            resolution: 48,
            time: 0.0,
            filaments: true,
            ..Default::default()
        };
        let initial = simulate(&n, &p);
        assert_eq!(
            initial.data,
            simulate(
                &n,
                &Particles {
                    count: 8192,
                    ..p.clone()
                }
            )
            .data
        );
        let evolved = simulate(&n, &Particles { time: 0.25, ..p });
        assert_ne!(initial.data, evolved.data);
        assert!(evolved
            .data
            .iter()
            .all(|v| v.iter().all(|c| c.is_finite() && *c >= 0.0) && v[2] <= v[0] + 1e-5));
    }
    #[test]
    fn fine_eddy_velocity_is_divergence_free() {
        let n = Nebula {
            half_size: Vec3::new(2.0, 3.0, 1.5),
            ..Default::default()
        };
        let g = detail_velocity(&n);
        let error = g
            .divergence()
            .iter()
            .map(|d| *d as f64 * *d as f64)
            .sum::<f64>()
            / g.data.len() as f64;
        assert!(
            error.sqrt() < 1e-5,
            "detail field RMS divergence {}",
            error.sqrt()
        );
        assert!(g.data.iter().any(|v| v.length() > 0.01));
    }
    #[test]
    fn spider_clears_the_stellar_cavity_and_rebuilds_when_it_moves() {
        let mut n = Nebula {
            core_position: Vec3::ZERO,
            half_size: Vec3::new(14.0, 8.0, 0.9),
            ..Default::default()
        };
        let p = Particles {
            filaments: true,
            spider: true,
            ..Default::default()
        };
        let cell = Vec3::splat(0.03);
        let center = spider_density(Vec3::ZERO, cell, &n, &p);
        let samples: Vec<_> = (0..1000)
            .map(|i| spider_density(Vec3::new(i as f32 * 0.01, 0.0, 0.0), cell, &n, &p))
            .collect();
        assert!(center[0].is_finite() && center[0] >= 0.0);
        assert_eq!(center[1], 0.0);
        assert_eq!(center[3], 0.0);
        assert!(samples.iter().any(|v| v[1] > 0.01 && v[3] < v[0] * 0.01));
        assert!(samples.iter().any(|v| v[3] > 0.05));
        assert!(samples
            .iter()
            .all(|v| v.iter().all(|a| a.is_finite() && *a >= 0.0) && v[2] <= v[0] && v[3] <= v[0]));
        let before = simulation_key(&n, &p);
        n.core_position.x += 1.0;
        assert_ne!(before, simulation_key(&n, &p));
        let ordinary = Particles { spider: false, ..p };
        let before = simulation_key(&n, &ordinary);
        n.core_position.x += 1.0;
        assert_eq!(before, simulation_key(&n, &ordinary));
    }
    #[test]
    fn thin_fronts_preserve_integrated_emission_and_colours_when_filtered() {
        let n = Nebula {
            half_size: Vec3::new(19.0, 12.0, 1.0),
            core_position: Vec3::ZERO,
            detail: 2.0,
            ..Default::default()
        };
        let p = Particles {
            filaments: true,
            spider: true,
            front_width: 0.025,
            ..Default::default()
        };
        // Integrate through the cavity rim with fine/coarse footprints. The
        // antialiasing kernel may broaden a skin but must not bleach its flux.
        let integral = |width: f32| {
            (0..20000)
                .map(|i| {
                    spider_density(
                        Vec3::new(i as f32 * 0.0005, 0.0, 0.0),
                        Vec3::splat(width),
                        &n,
                        &p,
                    )[3] as f64
                        * 0.0005
                })
                .sum::<f64>()
        };
        let fine = integral(0.01);
        let coarse = integral(0.06);
        assert!(
            (fine - coarse).abs() / fine < 0.15,
            "front flux fine {fine}, coarse {coarse}"
        );
        let mut scene = Scene::example();
        scene.atmosphere.nebula = Some(Nebula {
            particles: Some(p),
            ..n
        });
        let key = particle_key(&scene);
        scene
            .atmosphere
            .nebula
            .as_mut()
            .unwrap()
            .particles
            .as_mut()
            .unwrap()
            .cluster_stars = 1.0;
        assert_eq!(
            key,
            particle_key(&scene),
            "star brightness must not rebake density/shadows"
        );
    }
    #[test]
    fn gas_source_has_coherent_dense_pockets_and_rarefactions() {
        let mut values = Vec::new();
        for y in -60..60 {
            for x in -95..95 {
                let position = Vec3::new(x as f32 * 0.2, y as f32 * 0.2, 0.0);
                assert_eq!(gas_distribution(position, 417, 0.0), 1.0);
                let v = gas_distribution(position, 417, 2.4);
                assert!(v.is_finite() && v > 0.0);
                values.push(v);
                // Macro pockets must vary gradually, independent of fine detail.
                assert!((v - gas_distribution(position + Vec3::X * 0.001, 417, 2.4)).abs() < 0.02);
            }
        }
        let lo = values.iter().copied().fold(f32::INFINITY, f32::min);
        let hi = values.iter().copied().fold(0.0, f32::max);
        assert!(lo < 0.15 && hi > 2.0, "gas density envelope {lo}..{hi}");
    }
    #[test]
    fn distant_spider_has_a_finite_feathered_outline_inside_the_grid() {
        let n = Nebula {
            half_size: Vec3::new(19.0, 12.0, 1.0),
            seed: 417,
            ..Default::default()
        };
        assert_eq!(spider_envelope(Vec3::ZERO, &n), 1.0);
        // The corners must stay empty even before the texture's safety fade.
        for x in [-0.85, 0.85] {
            for y in [-0.85, 0.85] {
                assert_eq!(spider_envelope(n.half_size * Vec3::new(x, y, 0.0), &n), 0.0);
            }
        }
        let mut feathered = 0;
        for i in 0..2000 {
            let p = Vec3::new(-n.half_size.x + i as f32 * 0.02, 0.0, 0.0);
            let v = spider_envelope(p, &n);
            assert!((0.0..=1.0).contains(&v));
            assert!((v - spider_envelope(p + Vec3::X * 0.001, &n)).abs() < 0.002);
            feathered += usize::from(v > 0.05 && v < 0.95);
        }
        assert!(
            feathered > 100,
            "outer gas must dissolve over a broad boundary"
        );
    }
    #[test]
    fn branching_storm_has_finite_associated_moments_and_an_absorbing_rift() {
        let n = Nebula {
            half_size: Vec3::new(19.0, 12.0, 1.0),
            seed: 28361,
            detail: 4.0,
            turbulence: 1.2,
            ..Default::default()
        };
        let p = Particles {
            filaments: true,
            storm: true,
            ..Default::default()
        };
        let cell = Vec3::splat(0.04);
        for i in 0..500 {
            let position = Vec3::new(
                (i % 50) as f32 * 0.6 - 15.0,
                (i / 50) as f32 * 1.8 - 8.0,
                0.0,
            );
            let value = filament_density(position, cell, &n, &p);
            assert!(value.iter().all(|v| v.is_finite() && *v >= 0.0));
            assert!(value[2] <= value[0] && value[3] <= value[0]);
        }
        for position in [n.half_size, -n.half_size, Vec3::X * 20.0] {
            assert_eq!(filament_density(position, cell, &n, &p), [0.0; 4]);
        }
        // Integrate across streams: the dust must concentrate on the rift,
        // while gas exists on both sides instead of forming a uniform slab.
        let mut rift_dust = 0.0;
        let mut outside_dust = 0.0;
        let mut gas = [0.0; 2];
        for i in -200..200 {
            let x = i as f32 * 0.06;
            let broad = noise(
                Vec3::new(x, 0.0, 0.0) * Vec3::new(0.11, 0.16, 0.12),
                n.seed + 3001,
            )
            .0;
            let y = 1.4 * (x * 0.17).sin() + x * 0.10 + (broad - 0.5) * n.turbulence * 1.3;
            rift_dust += storm_density(Vec3::new(x, y, 0.0), cell, &n, &p)[1];
            outside_dust += storm_density(Vec3::new(x, y + 4.5, 0.0), cell, &n, &p)[1];
            for (side, offset) in [-2.5, 2.5].into_iter().enumerate() {
                gas[side] += storm_density(Vec3::new(x, y + offset, 0.0), cell, &n, &p)[0];
            }
        }
        assert!(
            rift_dust > outside_dust * 3.0,
            "rift {rift_dust}, outside {outside_dust}"
        );
        assert!(gas.iter().all(|v| *v > 1.0), "gas on the two banks {gas:?}");
        let key = simulation_key(&n, &p);
        assert_ne!(
            key,
            simulation_key(
                &n,
                &Particles {
                    storm: false,
                    ..p.clone()
                }
            )
        );
        let mut moved = n.clone();
        moved.core_position.x += 1.0;
        assert_ne!(key, simulation_key(&moved, &p));
    }

    #[test]
    fn convergences_require_overlapping_nonparallel_streams() {
        assert_eq!(stream_convergence(&[(1.0, Vec3::X), (1.0, Vec3::X)]), 0.0);
        assert_eq!(stream_convergence(&[(1.0, Vec3::X), (0.0, -Vec3::X)]), 0.0);
        assert!((stream_convergence(&[(1.0, Vec3::X), (1.0, -Vec3::X)]) - 1.0).abs() < 1e-6);
        let same_depth = stream_convergence(&[(1.0, Vec3::X), (1.0, Vec3::Y)]);
        // Identical projected centreline, separated by six tube radii in Z.
        let separated = stream_convergence(&[(1.0, Vec3::X), ((-18.0f32).exp(), Vec3::Y)]);
        assert!(separated < same_depth * 1e-6);
        assert_eq!(stream_convergence(&[(0.0, Vec3::X), (0.0, -Vec3::X)]), 0.0);
    }

    #[test]
    fn anisotropic_volume_resolves_xy_detail_within_the_same_memory_budget() {
        let mut n = Nebula {
            half_size: Vec3::new(19.0, 12.0, 3.5),
            ..Nebula::default()
        };
        let mut p = Particles {
            resolution: 1536,
            filaments: true,
            remnant: true,
            ..Particles::default()
        };
        let isotropic = density_dimensions(&n, &p);
        p.depth_sampling = 0.30;
        let detailed = density_dimensions(&n, &p);
        assert!(detailed[0] as f32 > isotropic[0] as f32 * 1.45);
        assert!(detailed[1] as f32 > isotropic[1] as f32 * 1.45);
        assert!(detailed[2] >= 48);
        assert!(detailed.iter().product::<usize>() <= 32 * 1024 * 1024);
        n.half_size = Vec3::new(500.0, 500.0, 0.01);
        assert!(density_dimensions(&n, &p).iter().product::<usize>() <= 32 * 1024 * 1024);
    }

    #[test]
    fn oriented_tubes_and_additive_gas_preserve_mass_and_dust_extinction() {
        let width = 0.016;
        // A tube along Z is unaffected by the length of the Z cell, even in
        // a shallow grid. Its two cross-section gradients lie in XY.
        let a = Vec3::X * 0.05;
        let b = Vec3::Y * 0.05;
        assert_eq!(
            filtered_tube(0.01, 0.02, a, b, width),
            filtered_tube(0.01, 0.02, a * Vec3::new(1.0, 1.0, 100.0), b, width)
        );
        let step = 0.005;
        let mut mass = 0.0;
        for y in -40..40 {
            for x in -40..40 {
                mass += filtered_tube(
                    (x as f32 + 0.5) * step,
                    (y as f32 + 0.5) * step,
                    a,
                    b,
                    width,
                ) * step
                    * step;
            }
        }
        assert!((mass / (std::f32::consts::TAU * width * width) - 1.0).abs() < 0.001);
        let mut scene = Scene::example();
        let n = Nebula {
            particles: Some(Particles {
                filaments: true,
                remnant: true,
                gas_opacity: 0.0,
                ..Particles::default()
            }),
            ..Nebula::default()
        };
        assert_eq!(extinction(Vec4::new(3.0, 0.2, 0.0, 0.0), &n), 0.2 * n.dust);
        scene.atmosphere.nebula = Some(n);
        let key = particle_key(&scene);
        let n = scene.atmosphere.nebula.as_mut().unwrap();
        let simulation = simulation_key(n, n.particles.as_ref().unwrap());
        n.particles.as_mut().unwrap().spectral_detail = 1.0;
        assert_eq!(simulation, simulation_key(n, n.particles.as_ref().unwrap()));
        assert_eq!(key, particle_key(&scene));
        let n = scene.atmosphere.nebula.as_mut().unwrap();
        n.particles.as_mut().unwrap().gas_opacity = 1.0;
        assert_eq!(simulation, simulation_key(n, n.particles.as_ref().unwrap()));
        assert_ne!(key, particle_key(&scene));
    }

    #[test]
    fn remnant_excitation_preserves_opacity_and_filtered_shock_power() {
        let n = Nebula {
            half_size: Vec3::new(19.0, 12.0, 3.5),
            ..Nebula::default()
        };
        let mut p = Particles {
            remnant: true,
            filaments: true,
            convergence: 0.0,
            shock: 0.0,
            ..Particles::default()
        };
        let mut excited_mass = 0.0;
        for y in -10..10 {
            for x in -16..16 {
                let position = Vec3::new(x as f32, y as f32, 0.3);
                let dark = remnant_density(position, Vec3::splat(0.05), &n, &p);
                p.convergence = 5.0;
                p.shock = 2.0;
                let lit = remnant_density(position, Vec3::splat(0.05), &n, &p);
                p.convergence = 0.0;
                p.shock = 0.0;
                assert_eq!(
                    &dark[..3],
                    &lit[..3],
                    "excitation must not add gas/dust opacity"
                );
                assert!(lit.iter().all(|v| v.is_finite() && *v >= 0.0));
                assert!(lit[2] <= lit[0]);
                assert_eq!(dark[3], 0.0);
                excited_mass += lit[3];
            }
        }
        assert!(excited_mass > 0.1);
        assert_eq!(
            remnant_density(n.half_size, Vec3::splat(0.05), &n, &p),
            [0.0; 4]
        );
        let integrate = |step: f32| {
            let count = (4.0 / step) as usize;
            (0..count)
                .map(|i| filtered_front(-2.0 + (i as f32 + 0.5) * step, 0.03, step) * step)
                .sum::<f32>()
        };
        assert!((integrate(0.005) / integrate(0.075) - 1.0).abs() < 0.01);
        let key = simulation_key(&n, &p);
        p.shock_radius += 1.0;
        assert_ne!(key, simulation_key(&n, &p));
        let mut moved = n.clone();
        moved.core_position.x += 1.0;
        assert_ne!(simulation_key(&n, &p), simulation_key(&moved, &p));
    }
    #[test]
    fn particles_are_deterministic_and_reconstruction_preserves_mass() {
        let n = Nebula::default();
        let p = Particles {
            count: 4096,
            resolution: 48,
            time: 0.25,
            ..Default::default()
        };
        let a = simulate(&n, &p);
        let b = simulate(&n, &p);
        assert_eq!(a.data, b.data);
        assert!(a
            .data
            .iter()
            .all(|v| v.iter().all(|c| c.is_finite() && *c >= 0.0)));
        assert!(a.data.iter().all(|v| v[2] <= v[0] + 1e-5));
        let sum: f32 = a.data.iter().map(|v| v[0] + v[1]).sum::<f32>() * a.cell().element_product();
        assert!(sum > 0.0);
        let finer = simulate(
            &n,
            &Particles {
                resolution: 72,
                ..p.clone()
            },
        );
        let finer_mass: f32 =
            finer.data.iter().map(|v| v[0] + v[1]).sum::<f32>() * finer.cell().element_product();
        assert!(
            (sum - finer_mass).abs() / sum < 1e-4,
            "mass changed with reconstruction grid: {sum} -> {finer_mass}"
        );
        let mut n2 = n.clone();
        n2.seed += 1;
        assert_ne!(a.data, simulate(&n2, &p).data);
    }
    #[test]
    fn homogeneous_beer_shadow_is_exact_and_empty_grid_is_clear() {
        let n = Nebula::default();
        let mut g = Grid::new([8; 3], Vec3::splat(2.0));
        g.data.fill([0.5, 0.2, 0.0, 0.0]);
        let tr = transmittance(&g, &n, Vec3::ZERO, Vec3::X, 32);
        assert!((tr - (-extinction(Vec4::new(0.5, 0.2, 0.0, 0.0), &n)).exp()).abs() < 1e-6);
        g.data.fill([0.0; 4]);
        assert_eq!(transmittance(&g, &n, Vec3::ZERO, Vec3::X, 32), 1.0);
    }
}
