use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use syfrack_core::{
    Atmosphere, Coloring, Fractal, FractalTexture, Light, Nebula, Optics, PaletteInterpolation,
    Ramp, Scene, SurfaceVariation, TextureChannel, MAX_LIGHTS, MAX_PALETTE_STOPS,
};

pub const KIND_SUN: u32 = 0;
pub const KIND_AREA: u32 = 1;
pub const KIND_ENV: u32 = 2;
pub const KIND_HDRI: u32 = 3;

/// Colouring slots read by the shader: 0..=3 are trap slots of the graph.
pub const COLOR_NONE: u32 = 0xffff;
pub const COLOR_SLOT_ITER: u32 = 4;
pub const COLOR_SLOT_NORMAL: u32 = 5;

/// Every fractal is rendered through its form graph (kind 4):
/// kind = [4, iteration product, normal stencil count, 0]; p0.x is the pixel tolerance, p0.w is the step scale
/// (safety / Lipschitz bound); the parameters live in the graph buffer.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuFractal {
    pub kind: [u32; 4],
    pub p0: [f32; 4],
    pub p1: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuRamp {
    /// count, cyclic, smooth interpolation, explicit positions
    pub kind: [u32; 4],
    /// frequency, phase, gamma, reserved
    pub mapping: [f32; 4],
    /// linear RGB, position
    pub stops: [[f32; 4]; MAX_PALETTE_STOPS],
}
impl GpuRamp {
    pub fn from_ramp(r: &Ramp) -> Self {
        let mut stops = [[0.0; 4]; MAX_PALETTE_STOPS];
        for (i, (out, rgb)) in stops.iter_mut().zip(&r.stops).enumerate() {
            *out = [rgb[0], rgb[1], rgb[2], r.position(i)];
        }
        Self {
            kind: [
                r.stops.len() as u32,
                u32::from(r.cyclic),
                u32::from(r.interpolation == PaletteInterpolation::Smooth),
                u32::from(!r.positions.is_empty()),
            ],
            mapping: [r.frequency, r.phase, r.gamma, 0.0],
            stops,
        }
    }
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuColoring {
    pub kind_count: [u32; 4],
    pub ramp: GpuRamp,
    pub weights: [f32; 4],
    /// iteration weight, normal weight, logarithmic gain, reserved
    pub extra: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuAtmosphere {
    /// fog_density, fog_height, fog_falloff, anisotropy
    pub a: [f32; 4],
    /// fog colour rgb, cavity_darkness
    pub b: [f32; 4],
    /// enabled, height, finish index, reserved
    pub ground: [f32; 4],
    pub dunes: [f32; 4],
    pub portal: [f32; 4],
    pub portal_world: [f32; 4],
    pub portal_color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuNebula {
    pub center: [f32; 4],     // xyz, rotation radians
    pub extent: [f32; 4],     // half extents xyz, density
    pub cool: [f32; 4],       // colour rgb, emission
    pub warm: [f32; 4],       // colour rgb, dust extinction
    pub core: [f32; 4],       // local xyz, radius
    pub settings: [f32; 4],   // turbulence (remnant gas opacity), detail, core energy, stars
    pub quality: [u32; 4],    // seed, steps, enabled bit + 16-bit spectral gain, volume kind (0..4)
    pub scattering: [f32; 4], // collision coefficient, HG anisotropy, multiple, cluster
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuOptics {
    pub surface: [f32; 4],    // opacity, transmission, IOR, thin sheet
    pub absorption: [f32; 4], // Beer-Lambert sigma_a rgb, enabled
    pub subsurface: [f32; 4], // scattering albedo rgb, collision rate
}

impl GpuOptics {
    fn from_optics(optics: Option<&Optics>) -> Self {
        let Some(o) = optics else {
            return Self::zeroed();
        };
        let sigma = o
            .absorption_color
            .map(|c| -c.max(0.0001).ln() / o.absorption_distance);
        Self {
            surface: [o.opacity, o.transmission, o.ior, f32::from(o.thin_walled)],
            absorption: [sigma[0], sigma[1], sigma[2], o.absorption_distance],
            subsurface: [
                o.subsurface_color[0],
                o.subsurface_color[1],
                o.subsurface_color[2],
                o.subsurface / o.subsurface_radius,
            ],
        }
    }
}

impl GpuNebula {
    fn from_nebula(nebula: Option<&Nebula>) -> Self {
        let Some(n) = nebula else {
            return Self::zeroed();
        };
        Self {
            center: v4(n.center, n.angle.to_radians()),
            extent: v4(n.half_size, n.density),
            cool: [
                n.cool_color[0],
                n.cool_color[1],
                n.cool_color[2],
                n.emission,
            ],
            warm: [n.warm_color[0], n.warm_color[1], n.warm_color[2], n.dust],
            core: v4(n.core_position, n.core_radius),
            settings: [
                n.particles
                    .as_ref()
                    .filter(|p| p.remnant && p.filaments && !p.smoke)
                    .map_or(n.turbulence, |p| p.gas_opacity),
                n.detail,
                n.core_energy,
                n.stars,
            ],
            quality: [
                n.seed,
                n.steps,
                1 | (n
                    .particles
                    .as_ref()
                    .filter(|p| p.remnant && p.filaments && !p.smoke)
                    .map_or(0, |p| (p.spectral_detail * 65535.0).round() as u32)
                    << 1),
                n.particles.as_ref().map_or(0, |p| {
                    if p.remnant && p.filaments && !p.smoke {
                        if p.spectral_detail > 0.0 {
                            4
                        } else {
                            3
                        }
                    } else if (p.spider || p.storm) && p.filaments && !p.smoke {
                        2
                    } else {
                        1
                    }
                }),
            ],
            scattering: [
                n.scattering,
                n.anisotropy,
                n.particles.as_ref().map_or(0.0, |p| p.multiple_scattering),
                n.particles.as_ref().map_or(0.0, |p| p.cluster_stars),
            ],
        }
    }
}

impl GpuFractal {
    pub fn from_fractal(f: &Fractal) -> Self {
        let g = f.graph();
        Self {
            kind: [4, g.iteration_product(), 4, 0],
            p0: [1.0, 0.0, 0.0, g.step_scale()],
            p1: [0.0; 4],
        }
    }
}

impl GpuColoring {
    pub fn from_coloring(c: &Coloring) -> Self {
        let ramp = c.ramp().map_or(GpuRamp::zeroed(), GpuRamp::from_ramp);
        let (weights, extra) = match c {
            Coloring::Orbit { channels, .. } => (
                channels.traps,
                [
                    channels.iterations,
                    channels.normal,
                    channels.logarithmic,
                    0.0,
                ],
            ),
            _ => ([0.0; 4], [0.0; 4]),
        };
        Self {
            kind_count: [c.slot(), ramp.kind[0], 0, 0],
            ramp,
            weights,
            extra,
        }
    }
}

impl GpuAtmosphere {
    pub fn from_atmosphere(a: &Atmosphere) -> Self {
        Self {
            a: [a.fog_density, a.fog_height, a.fog_falloff, a.anisotropy],
            b: [
                a.fog_color[0],
                a.fog_color[1],
                a.fog_color[2],
                a.cavity_darkness,
            ],
            ground: a
                .ground
                .as_ref()
                .map_or([0.0; 4], |g| [1.0, g.height, g.finish as f32, 0.0]),
            dunes: a
                .ground
                .as_ref()
                .and_then(|g| g.dunes.as_ref())
                .map_or([0.0; 4], |d| {
                    [
                        d.amplitude,
                        d.wavelength,
                        d.wind_direction.to_radians(),
                        d.seed as f32,
                    ]
                }),
            portal: a
                .portal
                .as_ref()
                .map_or([0.0; 4], |p| [p.center.x, p.center.y, p.center.z, p.radius]),
            portal_world: a
                .portal
                .as_ref()
                .map_or([0.0; 4], |p| [p.scale, p.elevation, p.energy, p.time]),
            portal_color: a.portal.as_ref().map_or([0.0; 4], |p| {
                [p.color[0], p.color[1], p.color[2], p.turbulence]
            }),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuCamera {
    pub origin: [f32; 4],
    pub forward: [f32; 4],
    pub right: [f32; 4],
    pub up: [f32; 4],
    /// tan_half_fov, aspect, lens_radius, focus_distance
    pub params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuMaterial {
    /// rgb = albedo, w = roughness
    pub albedo: [f32; 4],
    /// metallic, specular F0, 0, 0
    pub params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuPlanar {
    pub kind: [u32; 4],
    pub constants: [f32; 4],
    pub mapping: [f32; 4],
    pub settings: [f32; 4],
    pub detail: [f32; 4],
}
impl GpuPlanar {
    pub fn from_planar(p: Option<&syfrack_core::PlanarFractal>) -> Self {
        use syfrack_core::PlanarFamily;
        let Some(p) = p else {
            return Self::zeroed();
        };
        let family = match p.family {
            PlanarFamily::Julia => 1,
            PlanarFamily::Mandelbrot => 2,
            PlanarFamily::BurningShip => 3,
            PlanarFamily::Newton => 4,
            PlanarFamily::Nova => 5,
            PlanarFamily::Phoenix => 6,
        };
        Self {
            kind: [family, p.power, p.iterations, 0],
            constants: [p.julia[0], p.julia[1], p.center[0], p.center[1]],
            mapping: [
                p.zoom,
                p.angle.to_radians(),
                p.inversion,
                p.twist.to_radians(),
            ],
            settings: [p.relaxation, p.memory, p.bailout, p.trap_radius],
            detail: [p.stripes, 0.0, 0.0, 0.0],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuTexture {
    /// Channel, octaves, graph iteration product, ramp stop count.
    pub kind: [u32; 4],
    /// Offset xyz, scale.
    pub mapping: [f32; 4],
    /// Euler radians xyz, contrast.
    pub rotation: [f32; 4],
    /// Colour strength, roughness strength, bump strength, enabled.
    pub effects: [f32; 4],
    pub ramp: GpuRamp,
    pub weights: [f32; 4],
    pub extra: [f32; 4],
    /// Parallax depth, layer budget, reserved.
    pub parallax: [f32; 4],
    /// Period XZ, ray count, enabled.
    pub fan: [f32; 4],
    /// Geometry trap slot, target value, width, enabled.
    pub mask: [f32; 4],
    pub planar: GpuPlanar,
}

impl GpuTexture {
    pub fn from_texture(texture: Option<&FractalTexture>) -> Self {
        let Some(t) = texture else {
            return Self::zeroed();
        };
        Self {
            kind: [
                t.channel.slot(),
                t.octaves,
                t.iteration_budget(),
                t.ramp.stops.len() as u32,
            ],
            mapping: v4(t.offset, t.scale),
            rotation: v4(t.rotation * (std::f32::consts::PI / 180.0), t.contrast),
            effects: [t.strength, t.roughness, t.bump, 1.0],
            ramp: GpuRamp::from_ramp(&t.ramp),
            weights: match t.channel {
                TextureChannel::Orbit { channels } => channels.traps,
                _ => [0.0; 4],
            },
            extra: match t.channel {
                TextureChannel::Orbit { channels } => [
                    channels.iterations,
                    channels.normal,
                    channels.logarithmic,
                    0.0,
                ],
                _ => [0.0; 4],
            },
            parallax: [t.parallax, t.parallax_steps as f32, 0.0, 0.0],
            fan: t
                .fan
                .as_ref()
                .map_or([0.0; 4], |f| [f.period[0], f.period[1], f.rays as f32, 1.0]),
            mask: t
                .mask
                .as_ref()
                .map_or([0.0; 4], |m| [m.slot as f32, m.value, m.width, 1.0]),
            planar: GpuPlanar::from_planar(t.planar.as_ref()),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuSurfaceVariation {
    pub kind: [u32; 4],
    /// Roughness endpoints, then metallic endpoints.
    pub finish: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuSeascape {
    /// Enabled, sea level, intensity, wave strength.
    pub settings: [f32; 4],
    /// Wave scale, cloud cover, haze, reserved.
    pub detail: [f32; 4],
    pub zenith: [f32; 4],
    pub horizon: [f32; 4],
    pub water: [f32; 4],
    /// Enabled, wave height, wavelength, crest sharpness.
    pub ocean_wave: [f32; 4],
    /// Wind angle in radians, time in seconds, ripples, roughness.
    pub ocean_detail: [f32; 4],
    /// Foam amount, shore width, depth, IOR.
    pub ocean_optics: [f32; 4],
    /// RGB absorption per metre, scattering per metre.
    pub ocean_absorption: [f32; 4],
    pub ocean_foam: [f32; 4],
    pub ocean_bed: [f32; 4],
    /// Height bound, global slope bound, global curvature bound, reserved.
    pub ocean_bounds: [f32; 4],
    /// k * direction XZ, amplitude, phase at the saved time.
    pub ocean_spectrum: [[f32; 4]; 12],
}

impl GpuSeascape {
    fn from_seascape(sea: Option<&syfrack_core::Seascape>) -> Self {
        sea.map_or(Self::zeroed(), |s| {
            let mut out = Self {
                settings: [1.0, s.sea_level, s.intensity, s.wave_strength],
                detail: [s.wave_scale, s.clouds, s.haze, 0.0],
                zenith: scaled(s.sky_zenith, 1.0),
                horizon: scaled(s.sky_horizon, 1.0),
                water: scaled(s.water_color, 1.0),
                ..Self::zeroed()
            };
            if let Some(o) = &s.surface {
                out.ocean_wave = [1.0, o.wave_height, o.wavelength, o.crest_sharpness];
                out.ocean_detail = [
                    o.wind_direction.to_radians(),
                    o.time,
                    o.ripple_strength,
                    o.roughness,
                ];
                out.ocean_optics = [o.foam_amount, o.shore_width, o.depth, o.ior];
                out.ocean_absorption = [
                    o.absorption[0],
                    o.absorption[1],
                    o.absorption[2],
                    o.scattering,
                ];
                out.ocean_foam = scaled(o.foam_color, 1.0);
                out.ocean_bed = scaled(o.seabed_color, 1.0);
                // Irregular spectral bands; neither wavelength nor direction follows
                // a geometric lattice. Slow transverse envelopes form wave packets.
                let bands: [f32; 12] = [
                    0.73, 0.93, 1.071, 1.29, 1.55, 1.89, 2.3, 2.79, 3.41, 4.2, 5.19, 6.43,
                ];
                let weight: f32 = bands.iter().map(|k| k.powf(-1.65)).sum();
                let mut slope = 0.0;
                let mut curvature = 0.0;
                for (i, mode) in out.ocean_spectrum.iter_mut().enumerate() {
                    let frequency = std::f32::consts::TAU * bands[i] / o.wavelength;
                    let amplitude = 0.5 * o.wave_height * bands[i].powf(-1.65) / weight;
                    let angle =
                        o.wind_direction.to_radians() + (i as f32 * 7.137 + 0.61).sin() * 0.72;
                    *mode = [
                        angle.cos() * frequency,
                        angle.sin() * frequency,
                        amplitude,
                        (i as f32 * 3.917 + 1.13).sin() * 17.0 - (9.81 * frequency).sqrt() * o.time,
                    ];
                    let sharp = 0.25 * o.crest_sharpness;
                    slope +=
                        amplitude * frequency * (1.0 + 2.0 * sharp + 0.35 * 0.137 * (1.0 + sharp));
                    curvature += amplitude
                        * frequency
                        * frequency
                        * (1.0
                            + 4.0 * sharp
                            + 2.0 * 0.35 * 0.137 * (1.0 + 2.0 * sharp)
                            + 0.35 * 0.137 * 0.137 * (1.0 + sharp));
                }
                out.ocean_bounds = [
                    0.5 * o.wave_height * (1.0 + 0.25 * o.crest_sharpness),
                    slope,
                    curvature,
                    12.0,
                ];
            }
            out
        })
    }
}

impl GpuSurfaceVariation {
    fn from_surface(surface: Option<&SurfaceVariation>) -> Self {
        match surface {
            Some(s) => Self {
                kind: [s.slot, 1, 0, 0],
                finish: [s.roughness[0], s.roughness[1], s.metallic[0], s.metallic[1]],
            },
            None => Self::zeroed(),
        }
    }
}

/// Sun: a = (direction, cos(angular radius)), color = rgb * irradiance.
/// Area: a = centre, b = full width edge, c = full height edge, color = radiance.
/// Environment: a = zenith, b = horizon, c = ground (all premultiplied by intensity).
/// HDRI: kind = [3, texel offset, width, height], a.xy = cos/sin rotation,
/// a.z = uniform-only sampling, color.x = intensity.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuLight {
    pub kind: [u32; 4],
    pub a: [f32; 4],
    pub b: [f32; 4],
    pub c: [f32; 4],
    pub color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuFinish {
    pub albedo: [f32; 4],
    pub params: [f32; 4],
    pub emission: [f32; 4],
    pub detail_kind: [u32; 4],
    pub detail_mapping: [f32; 4],
    pub detail_tint: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Uniforms {
    pub image_size: [u32; 2],
    pub tile_origin: [u32; 2],
    pub tile_size: [u32; 2],
    pub sample_start: u32,
    pub sample_count: u32,
    pub seed: u32,
    pub max_bounces: u32,
    pub max_steps: u32,
    pub light_count: u32,
    pub flags: u32,
    pub guiding_flags: u32,
    pub guiding_record_round: u32,
    pub guiding_training_end: u32,
    pub fractal: GpuFractal,
    pub coloring: GpuColoring,
    pub atmosphere: GpuAtmosphere,
    pub camera: GpuCamera,
    pub material: GpuMaterial,
    pub lights: [GpuLight; MAX_LIGHTS],
    pub texture: GpuTexture,
    pub surface: GpuSurfaceVariation,
    pub seascape: GpuSeascape,
    pub finish_count: [u32; 4],
    pub finishes: [GpuFinish; 8],
    pub nebula: GpuNebula,
    pub optics: [GpuOptics; 9],
}

fn v4(v: Vec3, w: f32) -> [f32; 4] {
    [v.x, v.y, v.z, w]
}

fn scaled(c: [f32; 3], k: f32) -> [f32; 4] {
    [c[0] * k, c[1] * k, c[2] * k, 0.0]
}

impl GpuLight {
    pub fn from_light(light: &Light) -> Self {
        match light {
            Light::Hdri {
                intensity,
                rotation,
                ..
            } => {
                let radians = rotation.to_radians();
                Self {
                    kind: [KIND_HDRI, 0, 0, 0],
                    a: [radians.cos(), radians.sin(), 0.0, 0.0],
                    color: [*intensity, 0.0, 0.0, 0.0],
                    ..Self::zeroed()
                }
            }
            Light::Sun {
                direction,
                color,
                intensity,
                angular_size,
            } => Self {
                kind: [KIND_SUN, 0, 0, 0],
                a: v4(
                    direction.normalize_or(Vec3::Y),
                    (angular_size * 0.5).to_radians().cos(),
                ),
                b: [0.0; 4],
                c: [0.0; 4],
                color: scaled(color.to_linear(), *intensity),
            },
            Light::Area {
                position,
                normal,
                size,
                color,
                intensity,
            }
            | Light::Projector {
                position,
                normal,
                size,
                color,
                intensity,
                ..
            } => {
                let n = normal.normalize_or(Vec3::Y);
                let helper = if n.y.abs() > 0.999 { Vec3::Z } else { Vec3::Y };
                let u = helper.cross(n).normalize();
                let v = n.cross(u);
                let profile = if let Light::Projector {
                    inner_angle,
                    outer_angle,
                    ..
                } = light
                {
                    [
                        inner_angle.to_radians().cos(),
                        outer_angle.to_radians().cos(),
                        outer_angle.to_radians().tan(),
                    ]
                } else {
                    [0.0; 3]
                };
                Self {
                    kind: [
                        KIND_AREA,
                        u32::from(matches!(light, Light::Projector { .. })),
                        0,
                        0,
                    ],
                    a: v4(*position, profile[0]),
                    b: v4(u * size[0], profile[1]),
                    c: v4(v * size[1], profile[2]),
                    color: scaled(color.to_linear(), *intensity),
                }
            }
            Light::Tube {
                start,
                end,
                radius,
                color,
                intensity,
            } => Self {
                kind: [4, 0, 0, 0],
                a: v4(*start, *radius),
                b: v4(*end, start.distance(*end)),
                c: [0.0; 4],
                color: scaled(color.to_linear(), *intensity),
            },
            Light::Environment {
                zenith,
                horizon,
                ground,
                intensity,
                ..
            } => Self {
                kind: [KIND_ENV, 0, 0, 0],
                a: scaled(zenith.to_linear(), *intensity),
                b: scaled(horizon.to_linear(), *intensity),
                c: scaled(ground.to_linear(), *intensity),
                color: [0.0; 4],
            },
        }
    }
}

impl Uniforms {
    pub fn from_scene(
        scene: &Scene,
        width: u32,
        height: u32,
        max_bounces: u32,
        max_steps: u32,
    ) -> Self {
        let basis = scene.camera.basis();
        let mut camera = GpuCamera {
            origin: v4(basis.origin, 0.0),
            forward: v4(basis.forward, 0.0),
            right: v4(basis.right, 0.0),
            up: v4(basis.up, 0.0),
            params: [
                basis.tan_half_fov,
                width as f32 / height as f32,
                basis.lens_radius,
                basis.focus_distance,
            ],
        };
        let m = &scene.material;
        if scene.dimension == syfrack_core::Dimension::TwoD {
            let (s, c) = scene.canvas.rotation.to_radians().sin_cos();
            camera.origin = [scene.canvas.center[0], scene.canvas.center[1], c, s];
            camera.params[0] = scene.canvas.zoom;
        }
        let material = GpuMaterial {
            albedo: [m.albedo[0], m.albedo[1], m.albedo[2], m.roughness],
            params: [m.metallic, m.specular, 0.0, 0.0],
        };
        let mut fractal = GpuFractal::from_fractal(&scene.fractal);
        // Preserve the legacy minimum while allowing complete architectural
        // environments beyond the old fixed 200-unit tracing distance.
        fractal.p0[2] =
            (2.0 * scene.fractal.graph().bounding_radius_hint + basis.origin.length()).max(200.0);
        let mut lights = [GpuLight::zeroed(); MAX_LIGHTS];
        for (slot, light) in lights.iter_mut().zip(&scene.lights) {
            *slot = GpuLight::from_light(light);
        }
        let mut finishes = [GpuFinish::zeroed(); 8];
        for (slot, f) in finishes.iter_mut().zip(&m.finishes) {
            *slot = GpuFinish {
                albedo: [f.albedo[0], f.albedo[1], f.albedo[2], f.roughness],
                params: [f.metallic, f.specular, 0.0, 0.0],
                emission: [f.emission[0], f.emission[1], f.emission[2], 0.0],
                detail_kind: [0; 4],
                detail_mapping: [0.0; 4],
                detail_tint: [0.0; 4],
            };
            if let Some(d) = &f.detail {
                use syfrack_core::FinishPattern::*;
                let pattern = match d.pattern {
                    VeinedStone => 1,
                    Patina => 2,
                    BrushedMetal => 3,
                    Basalt => 4,
                    Ripples => 5,
                    Rosettes => 6,
                    CoastalRock => 7,
                    Sand => 8,
                    Dust => 9,
                    Concrete => 10,
                    WeatheredConcrete => 11,
                    WetConcrete => 12,
                };
                slot.detail_kind = [pattern, d.seed, 0, 0];
                slot.detail_mapping = [d.scale, d.strength, d.bump, d.roughness_delta];
                slot.detail_tint = [d.tint[0], d.tint[1], d.tint[2], 0.0];
            }
        }
        Self {
            image_size: [width, height],
            tile_origin: [0, 0],
            tile_size: [width, height],
            sample_start: 0,
            sample_count: 0,
            seed: (scene.seed as u32) ^ ((scene.seed >> 32) as u32),
            max_bounces,
            max_steps,
            light_count: scene.lights.len().min(MAX_LIGHTS) as u32,
            flags: u32::from(scene.dimension == syfrack_core::Dimension::TwoD),
            guiding_flags: 0,
            guiding_record_round: 0,
            guiding_training_end: 0,
            fractal,
            coloring: GpuColoring::from_coloring(&scene.material.coloring),
            atmosphere: GpuAtmosphere::from_atmosphere(&scene.atmosphere),
            camera,
            material,
            lights,
            texture: GpuTexture::from_texture(scene.material.texture.as_ref()),
            surface: GpuSurfaceVariation::from_surface(scene.material.surface.as_ref()),
            seascape: GpuSeascape::from_seascape(scene.atmosphere.seascape.as_ref()),
            finish_count: [m.finishes.len().min(8) as u32, 0, 0, 0],
            finishes,
            nebula: GpuNebula::from_nebula(scene.atmosphere.nebula.as_ref()),
            optics: std::array::from_fn(|i| {
                GpuOptics::from_optics(if i == 0 {
                    m.optics.as_ref()
                } else {
                    m.finishes.get(i - 1).and_then(|f| f.optics.as_ref())
                })
            }),
        }
    }

    pub fn set_tile(
        &mut self,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
        sample_start: u32,
        sample_count: u32,
    ) {
        self.tile_origin = [x, y];
        self.tile_size = [w, h];
        self.sample_start = sample_start;
        self.sample_count = sample_count;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use syfrack_core::{Color, Light, Scene};

    #[test]
    fn layout_matches_the_wgsl_struct() {
        use std::mem::{offset_of, size_of};
        assert_eq!(size_of::<GpuFractal>(), 48);
        assert_eq!(size_of::<GpuColoring>(), 336);
        assert_eq!(size_of::<GpuAtmosphere>(), 112);
        assert_eq!(size_of::<GpuCamera>(), 80);
        assert_eq!(size_of::<GpuMaterial>(), 32);
        assert_eq!(size_of::<GpuTexture>(), 512);
        assert_eq!(size_of::<GpuSeascape>(), 384);
        assert_eq!(size_of::<GpuSurfaceVariation>(), 32);
        assert_eq!(size_of::<GpuLight>(), 80);
        assert_eq!(offset_of!(Uniforms, flags), 48);
        assert_eq!(offset_of!(Uniforms, fractal), 64);
        assert_eq!(offset_of!(Uniforms, coloring), 112);
        assert_eq!(offset_of!(Uniforms, atmosphere), 448);
        assert_eq!(offset_of!(Uniforms, camera), 560);
        assert_eq!(offset_of!(Uniforms, material), 640);
        assert_eq!(offset_of!(Uniforms, lights), 672);
        assert_eq!(offset_of!(Uniforms, texture), 1312 + (MAX_LIGHTS - 8) * 80);
        assert_eq!(offset_of!(Uniforms, surface), 1824 + (MAX_LIGHTS - 8) * 80);
        assert_eq!(offset_of!(Uniforms, seascape), 1856 + (MAX_LIGHTS - 8) * 80);
        assert_eq!(size_of::<GpuFinish>(), 96);
        assert_eq!(
            offset_of!(Uniforms, finish_count),
            2240 + (MAX_LIGHTS - 8) * 80
        );
        assert_eq!(offset_of!(Uniforms, finishes), 2256 + (MAX_LIGHTS - 8) * 80);
        assert_eq!(size_of::<GpuNebula>(), 128);
        assert_eq!(offset_of!(Uniforms, nebula), 3024 + (MAX_LIGHTS - 8) * 80);
        assert_eq!(size_of::<GpuOptics>(), 48);
        assert_eq!(offset_of!(Uniforms, optics), 3152 + (MAX_LIGHTS - 8) * 80);
        assert_eq!(size_of::<Uniforms>(), 3584 + (MAX_LIGHTS - 8) * 80);
        assert!(size_of::<Uniforms>() <= 16384);
    }

    #[test]
    fn example_scene_converts() {
        let u = Uniforms::from_scene(&Scene::example(), 640, 480, 4, 256);
        assert_eq!(u.image_size, [640, 480]);
        assert_eq!(u.light_count, 2);
        assert_eq!(u.max_bounces, 4);
        assert_eq!(u.lights[0].kind[0], KIND_SUN);
        assert_eq!(u.lights[1].kind[0], KIND_ENV);
        assert_eq!(u.fractal.kind, [4, 1, 4, 0]);
        assert_eq!(u.fractal.p0, [1.0, 0.0, 205.0, 1.0]);
        assert_eq!(u.coloring.kind_count[0], COLOR_NONE);
        assert_eq!(u.atmosphere.a[0], 0.0);
        assert!((u.camera.params[1] - 640.0 / 480.0).abs() < 1e-6);
    }

    #[test]
    fn spectral_gain_is_packed_without_changing_opacity_or_other_profiles() {
        let mut n = Nebula::default();
        let p = n.particles.as_mut().unwrap();
        p.filaments = true;
        p.remnant = true;
        p.gas_opacity = 0.0;
        p.spectral_detail = 0.7;
        let gpu = GpuNebula::from_nebula(Some(&n));
        assert_eq!(gpu.quality[3], 4);
        assert_eq!(gpu.quality[2] & 1, 1);
        assert!(((gpu.quality[2] >> 1) as f32 / 65535.0 - 0.7).abs() < 1e-5);
        assert_eq!(gpu.settings[0], 0.0);
        n.particles.as_mut().unwrap().spectral_detail = 0.0;
        assert_eq!(GpuNebula::from_nebula(Some(&n)).quality[3], 3);
        n.particles.as_mut().unwrap().remnant = false;
        n.particles.as_mut().unwrap().spectral_detail = 1.0;
        assert_eq!(GpuNebula::from_nebula(Some(&n)).quality[2], 1);
    }

    #[test]
    fn fractal_and_coloring_encode() {
        use syfrack_core::{Coloring, Ramp};
        let mut s = Scene::example_for("menger").unwrap();
        s.set_param("fractal.nodes[2].offset.z", 1.2).unwrap();
        s.material.coloring = Coloring::Trap {
            slot: COLOR_SLOT_ITER,
            ramp: Ramp {
                stops: vec![[0.1, 0.2, 0.3], [0.4, 0.5, 0.6]],
                ..Ramp::default()
            },
        };
        s.atmosphere.fog_density = 0.25;
        s.atmosphere.cavity_darkness = 0.5;
        let u = Uniforms::from_scene(&s, 8, 8, 2, 64);
        assert_eq!(u.fractal.kind, [4, 5, 4, 0]);
        assert_eq!(
            u.fractal.p0,
            [1.0, 0.0, 200.0, s.fractal.graph().step_scale()]
        );
        assert_eq!(u.fractal.p1, [0.0; 4]);
        assert_eq!(u.coloring.kind_count, [COLOR_SLOT_ITER, 2, 0, 0]);
        assert_eq!(u.coloring.ramp.stops[1], [0.4, 0.5, 0.6, 1.0]);
        assert_eq!(u.coloring.ramp.stops[3], [0.0; 4]);
        assert_eq!(u.atmosphere.a[0], 0.25);
        assert_eq!(u.atmosphere.b[3], 0.5);
    }

    #[test]
    fn coloring_slots_and_step_scale_encode() {
        use syfrack_core::{Coloring, Fractal, Graph, Node, NodeKind, Ramp};
        let ramp = Ramp {
            stops: vec![[0.0; 3], [1.0; 3]],
            ..Ramp::default()
        };
        let slot = |c: Coloring| GpuColoring::from_coloring(&c).kind_count[0];
        assert_eq!(slot(Coloring::None), COLOR_NONE);
        assert_eq!(
            slot(Coloring::Trap {
                slot: 3,
                ramp: ramp.clone()
            }),
            3
        );
        assert_eq!(
            slot(Coloring::Trap {
                slot: COLOR_SLOT_NORMAL,
                ramp
            }),
            COLOR_SLOT_NORMAL
        );
        // A Twist lowers the step scale below the graph's safety factor.
        let mut g = Graph::new_minimal();
        g.safety = 0.8;
        g.nodes.push(Node {
            id: 3,
            kind: NodeKind::Twist { rate: 0.5 },
            name: "t".into(),
            pos: [0.0; 2],
            enabled: true,
        });
        g.links = vec![
            syfrack_core::Link {
                from: (0, 0),
                to: (3, 0),
            },
            syfrack_core::Link {
                from: (3, 0),
                to: (1, 0),
            },
            syfrack_core::Link {
                from: (1, 0),
                to: (2, 0),
            },
        ];
        let f = GpuFractal::from_fractal(&Fractal::Graph(g.clone()));
        assert_eq!(f.p0[3], g.step_scale());
        assert!(f.p0[3] < 0.8);
    }

    #[test]
    fn sun_direction_is_normalised_and_cone_cosine_stored() {
        let l = GpuLight::from_light(&Light::Sun {
            direction: Vec3::new(0.0, 2.0, 0.0),
            color: Color::Linear([1.0, 0.5, 0.25]),
            intensity: 2.0,
            angular_size: 2.0,
        });
        assert_eq!(l.a[..3], [0.0, 1.0, 0.0]);
        assert!((l.a[3] - 1f32.to_radians().cos()).abs() < 1e-6);
        assert_eq!(l.color[..3], [2.0, 1.0, 0.5]);
    }

    #[test]
    fn area_light_edges_span_its_size() {
        let l = GpuLight::from_light(&Light::Area {
            position: Vec3::ZERO,
            normal: Vec3::Y,
            size: [2.0, 4.0],
            color: Color::Linear([1.0, 1.0, 1.0]),
            intensity: 1.0,
        });
        let u = Vec3::from_slice(&l.b[..3]);
        let v = Vec3::from_slice(&l.c[..3]);
        assert!((u.length() - 2.0).abs() < 1e-5);
        assert!((v.length() - 4.0).abs() < 1e-5);
        assert!(u.dot(v).abs() < 1e-5);
        assert!(
            (u.cross(v).normalize() - Vec3::Y).length() < 1e-5,
            "u × v must point along the normal"
        );
    }

    #[test]
    fn set_tile_updates_only_tile_fields() {
        let mut u = Uniforms::from_scene(&Scene::example(), 64, 64, 2, 64);
        u.set_tile(16, 32, 16, 16, 8, 4);
        assert_eq!(u.tile_origin, [16, 32]);
        assert_eq!(u.tile_size, [16, 16]);
        assert_eq!(u.sample_start, 8);
        assert_eq!(u.sample_count, 4);
        assert_eq!(u.image_size, [64, 64]);
    }
}
