// SyFrack shared shader prelude: constants, uniform block, graph helpers.
// The distance field `de_sample` is generated from the form graph
// (crates/syfrack-render/src/graph_wgsl.rs) and inserted after this prelude.
// Layout must match crates/syfrack-render/src/uniforms.rs exactly.

const PI: f32 = 3.14159265358979;
const T_MAX: f32 = 200.0;
const KIND_SUN: u32 = 0u;
const KIND_AREA: u32 = 1u;
const KIND_ENV: u32 = 2u;
const KIND_TUBE: u32 = 4u;
const COLOR_NONE: u32 = 65535u;
const COLOR_SLOT_ITER: u32 = 4u;
const COLOR_SLOT_NORMAL: u32 = 5u;
const COLOR_SLOT_ORBIT: u32 = 6u;

fn bulb_angle_square(a: vec2f) -> vec2f {
    return vec2f(a.x*a.x-a.y*a.y,2.0*a.x*a.y);
}
// Shared direct spherical powers for the two most-used Mandelbulb exponents.
// The graph fingerprint separates this family from continuously varying powers.
fn bulb_power68(p: vec3f,r: f32,power: f32) -> vec4f {
    let rho=length(p.xy);
    let theta=vec2f(clamp(p.z/r,-1.0,1.0),rho/r);
    var phi=vec2f(1.0,0.0);
    if (rho>0.0) { phi=p.xy/rho; }
    let t2=bulb_angle_square(theta); let f2=bulb_angle_square(phi);
    let t4=bulb_angle_square(t2); let f4=bulb_angle_square(f2);
    let r2=r*r; let r4=r2*r2;
    var t=bulb_angle_square(t4); var f=bulb_angle_square(f4);
    var radial_previous=r4*r2*r;
    if (power==6.0) {
        t=vec2f(t2.x*t4.x-t2.y*t4.y,t2.x*t4.y+t2.y*t4.x);
        f=vec2f(f2.x*f4.x-f2.y*f4.y,f2.x*f4.y+f2.y*f4.x);
        radial_previous=r4*r;
    }
    return vec4f(vec3f(t.y*f.x,t.y*f.y,t.x)*(radial_previous*r),power*radial_previous);
}

struct Camera {
    origin: vec4f,
    forward: vec4f,
    right: vec4f,
    up: vec4f,
    params: vec4f, // tan_half_fov, aspect, lens_radius, focus_distance
}

struct Material {
    albedo: vec4f, // rgb, w = roughness
    params: vec4f, // metallic, specular f0, 0, 0
}

struct Finish {
    albedo: vec4f,
    params: vec4f,
    emission: vec4f,
    detail_kind: vec4u,
    detail_mapping: vec4f,
    detail_tint: vec4f,
}

struct Light {
    kind: vec4u,
    a: vec4f,
    b: vec4f,
    c: vec4f,
    color: vec4f,
}

struct Fractal {
    kind: vec4u, // 4 (graph), iteration product, 0, 0
    p0: vec4f,   // w = step scale (safety / Lipschitz bound)
    p1: vec4f,
}

struct Palette {
    kind: vec4u,
    mapping: vec4f,
    stops: array<vec4f, 16>,
}
struct Coloring {
    kind_count: vec4u,
    ramp: Palette,
    weights: vec4f,
    extra: vec4f,
}

struct Atmosphere {
    a: vec4f, // fog_density, fog_height, fog_falloff, anisotropy
    b: vec4f, // fog colour, cavity_darkness
    ground: vec4f, // enabled, height, finish index, reserved
    dunes: vec4f, // amplitude, wavelength, wind radians, seed
    portal: vec4f, // center xyz, aperture radius (zero disables)
    portal_world: vec4f, // spatial scale, entry altitude, energy, frozen time
    portal_color: vec4f, // world sky tint, membrane turbulence
}

struct Nebula {
    center: vec4f,
    extent: vec4f,
    cool: vec4f,
    warm: vec4f,
    core: vec4f,
    settings: vec4f,
    quality: vec4u,
    scattering: vec4f,
}

struct Optics {
    surface: vec4f,
    absorption: vec4f,
    subsurface: vec4f,
}

struct PlanarFractal {
    kind: vec4u,
    constants: vec4f,
    mapping: vec4f,
    settings: vec4f,
    detail: vec4f,
}
struct FractalTexture {
    kind: vec4u, // channel, octaves, iterations, ramp stop count
    mapping: vec4f, // offset xyz, scale
    rotation: vec4f, // Euler radians xyz, contrast
    effects: vec4f, // colour, roughness, bump, enabled
    ramp: Palette,
    weights: vec4f,
    extra: vec4f,
    parallax: vec4f, // depth, layer budget, reserved
    fan: vec4f, // period XZ, rays, enabled
    mask: vec4f, // geometry trap, target, width, enabled
    planar: PlanarFractal,
}

struct Seascape {
    settings: vec4f,
    detail: vec4f,
    zenith: vec4f,
    horizon: vec4f,
    water: vec4f,
    ocean_wave: vec4f,
    ocean_detail: vec4f,
    ocean_optics: vec4f,
    ocean_absorption: vec4f,
    ocean_foam: vec4f,
    ocean_bed: vec4f,
    ocean_bounds: vec4f,
    ocean_spectrum: array<vec4f, 12>,
}

struct SurfaceVariation {
    kind: vec4u, // slot, enabled, reserved
    finish: vec4f, // roughness endpoints, metallic endpoints
}

struct Uniforms {
    image_size: vec2u,
    tile_origin: vec2u,
    tile_size: vec2u,
    sample_start: u32,
    sample_count: u32,
    seed: u32,
    max_bounces: u32,
    max_steps: u32,
    light_count: u32,
    flags: u32,
    guiding_flags: u32,
    guiding_record_round: u32,
    guiding_training_end: u32,
    fractal: Fractal,
    coloring: Coloring,
    atmosphere: Atmosphere,
    camera: Camera,
    material: Material,
    lights: array<Light, 96>,
    texture: FractalTexture,
    surface: SurfaceVariation,
    seascape: Seascape,
    finish_count: vec4u,
    finishes: array<Finish, 8>,
    nebula: Nebula,
    optics: array<Optics, 9>,
}

@group(0) @binding(0) var<uniform> u: Uniforms;

fn ray_extent() -> f32 {
    return max(max(T_MAX,u.fractal.p0.z), select(0.0, 4000.0, u.atmosphere.dunes.x > 0.0));
}

// ---------------------------------------------------------------- Graph helpers

@group(0) @binding(4) var<uniform> graph_params: array<vec4f, 2048>;

struct DeSample {
    distance: f32,
    traps: vec4f, // four trap slots normalised to [0, 1]; 1 when unwritten
    iterations: u32,
    material: u32,
    flags: u32, // bit 0: conservative bound, requiring exact evaluation near a hit
}

fn cylinder_distance(p: vec3f, radius: f32, half_height: f32) -> f32 {
    let q = vec2f(length(p.xz) - radius, abs(p.y) - half_height);
    return length(max(q, vec2f(0.0))) + min(max(q.x, q.y), 0.0);
}

fn gp(i: u32) -> f32 {
    return graph_params[i / 4u][i % 4u];
}

// glam's `v.try_normalize().unwrap_or(Vec3::Y)`: rcp = 1 / length must be
// finite and > 0, and the result is v * rcp (not v / length).
fn safe_normalize(v: vec3f) -> vec3f {
    let l = length(v);
    let rcp = 1.0 / l;
    if (!(l > 0.0) || !(rcp > 0.0) || !(rcp <= 3.4028235e38)) { return vec3f(0.0, 1.0, 0.0); }
    return v * rcp;
}

fn rot2(v: vec2f, a: f32) -> vec2f {
    let s = sin(a);
    let c = cos(a);
    return vec2f(c * v.x - s * v.y, s * v.x + c * v.y);
}

// Euclidean remainder (WGSL `%` truncates toward zero).
fn modf_euclid(x: f32, m: f32) -> f32 {
    return x - m * floor(x / m);
}

fn modf_euclid3(x: vec3f, m: vec3f) -> vec3f {
    return x - m * floor(x / m);
}

// glam Quat::from_euler(EulerRot::XYZ, a.x, a.y, a.z) * p = Rx(Ry(Rz p)):
// Z first, then Y, then X.
fn rotate_euler_xyz(p: vec3f, a: vec3f) -> vec3f {
    var q = p;
    q = vec3f(cos(a.z) * q.x - sin(a.z) * q.y, sin(a.z) * q.x + cos(a.z) * q.y, q.z);
    q = vec3f(cos(a.y) * q.x + sin(a.y) * q.z, q.y, -sin(a.y) * q.x + cos(a.y) * q.z);
    q = vec3f(q.x, cos(a.x) * q.y - sin(a.x) * q.z, sin(a.x) * q.y + cos(a.x) * q.z);
    return q;
}

fn box_distance(p: vec3f, half: vec3f) -> f32 {
    let q = abs(p) - half;
    return length(max(q, vec3f(0.0))) + min(max(q.x, max(q.y, q.z)), 0.0);
}

// Seeded, unwrapped world-space noise. This helper is also used by DE probes.
fn terrain_hash(cell: vec3i, seed: u32) -> f32 {
    let c = bitcast<vec3u>(cell);
    var h = c.x * 0x8da6b343u ^ c.y * 0xd8163841u ^ c.z * 0xcb1ab31fu ^ seed;
    h = (h ^ (h >> 16u)) * 0x7feb352du;
    h = (h ^ (h >> 15u)) * 0x846ca68bu;
    h ^= h >> 16u;
    return f32(h >> 8u) / 16777216.0;
}
fn terrain_noise(p: vec3f, seed: u32) -> f32 {
    let cell = vec3i(floor(p)); let f = fract(p);
    let t = f*f*(3.0-2.0*f);
    let a = mix(terrain_hash(cell,seed),terrain_hash(cell+vec3i(1,0,0),seed),t.x);
    let b = mix(terrain_hash(cell+vec3i(0,1,0),seed),terrain_hash(cell+vec3i(1,1,0),seed),t.x);
    let c = mix(terrain_hash(cell+vec3i(0,0,1),seed),terrain_hash(cell+vec3i(1,0,1),seed),t.x);
    let d = mix(terrain_hash(cell+vec3i(0,1,1),seed),terrain_hash(cell+vec3i(1,1,1),seed),t.x);
    return mix(mix(a,b,t.y),mix(c,d,t.y),t.z);
}
fn rock_distance(p: vec3f, half: vec3f, erosion: f32, seed: u32) -> f32 {
    let h = min(half.x,min(half.y,half.z));
    let q = p * (1.15/h);
    let relief = 0.57*terrain_noise(q,seed) + 0.29*terrain_noise(q*2.03,seed+97u)
        + 0.14*terrain_noise(q*4.17,seed+193u) - 0.5;
    // Each cubic value noise has |gradient| <= 1.5*sqrt(3).
    // Ellipsoid DE is 1-Lipschitz; this divisor certifies the displaced field.
    return ((length(p/half)-1.0)*h - erosion*h*relief) / (1.0+5.22*erosion);
}

fn round_box_distance(p: vec3f, half: vec3f, radius: f32) -> f32 {
    let r = min(radius, min(half.x, min(half.y, half.z)));
    return box_distance(p, half - vec3f(r)) - r;
}

fn bulb_distance(p: vec3f, dr: f32) -> f32 {
    let r = max(length(p), 1e-12);
    return 0.5 * log(r) * r / dr;
}

// Smooth-combination payload: b + (a - b) * h per slot, except that an empty
// slot (>= 1e30) on one side takes the other side; both empty stay empty.
fn blend_traps(a: vec4f, b: vec4f, h: f32) -> vec4f {
    let ea = a >= vec4f(1e30);
    let eb = b >= vec4f(1e30);
    var r = b + (a - b) * h;
    r = select(r, a, eb);
    r = select(r, b, ea & !eb);
    return r;
}

fn de(p: vec3f) -> f32 {
    return de_sample(p).distance;
}

fn palette_mix(r: Palette, a: u32, b: u32, amount: f32) -> vec3f {
    var f = clamp(amount, 0.0, 1.0);
    if (r.kind.z != 0u) { f = f*f*(3.0-2.0*f); }
    return mix(r.stops[a].xyz, r.stops[b].xyz, f);
}
fn palette_sample(r: Palette, value: f32) -> vec3f {
    let n = r.kind.x;
    if (n == 0u) { return vec3f(1.0); }
    if (n == 1u) { return r.stops[0].xyz; }
    var t = value * r.mapping.x + r.mapping.y;
    if (r.kind.y != 0u) { t = t-floor(t); } else { t = clamp(t, 0.0, 1.0); }
    if (r.mapping.z != 1.0) { t = pow(t, r.mapping.z); }
    // Original interpolation for palettes without positions or cycling.
    if (r.kind.w == 0u && r.kind.y == 0u) {
        let x = t * f32(n-1u);
        let i = min(u32(floor(x)), n-2u);
        return palette_mix(r, i, i+1u, x-f32(i));
    }
    let first = r.stops[0].w;
    let last = r.stops[n-1u].w;
    if (r.kind.y != 0u && (t < first || t >= last)) {
        let x = select(t, t+1.0, t < first);
        return palette_mix(r, n-1u, 0u, (x-last)/max(1.0+first-last, 1e-8));
    }
    if (t <= first) { return r.stops[0].xyz; }
    if (t >= last) { return r.stops[n-1u].xyz; }
    for (var i=0u; i<15u; i++) {
        if (i+1u < n && t < r.stops[i+1u].w) {
            return palette_mix(r, i, i+1u, (t-r.stops[i].w)/(r.stops[i+1u].w-r.stops[i].w));
        }
    }
    return r.stops[n-1u].xyz;
}
fn ramp_sample(t: f32) -> vec3f {
    return palette_sample(u.coloring.ramp, t);
}

// A zero flag certifies an exact winning branch. An unevaluated branch with a
// larger lower bound cannot change that winner; otherwise evaluate the field fully.
fn surface_sample(p: vec3f) -> DeSample {
    var s = de_sample(p);
    if (s.flags != 0u) { s = de_sample_exact(p); }
    return s;
}

fn normal_at(p: vec3f, h: f32) -> vec3f {
    let stencil = array<vec3f, 4>(vec3f(1.0,-1.0,-1.0),
        vec3f(-1.0,-1.0,1.0), vec3f(-1.0,1.0,-1.0), vec3f(1.0));
    var gradient = vec3f(0.0);
    // A uniform loop keeps one copy of the scene evaluator in the shader,
    // instead of expanding it four times for the tetrahedral stencil.
    for (var i=0u; i<u.fractal.kind.z; i++) {
        let k = stencil[i];
        gradient += k * de(p + k * h);
    }
    return normalize(gradient);
}

fn orbit_angle(p: vec2f) -> f32 {
    if (all(p == vec2f(0.0))) { return 0.5; }
    return atan2(p.y, p.x)/(2.0*PI)+0.5;
}
