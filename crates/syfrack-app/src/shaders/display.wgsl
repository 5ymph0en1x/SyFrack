// Display pass: accumulator -> developed image. Mirrors syfrack-color stages
// 1..6 (exposure, balance, grade, tonemap, saturation, vignette). No grain,
// no dither: those belong to the exported file.

struct Params {
    size: vec2u,
    art: vec2f, // neon softness, reserved
    scalars: vec4f,   // exposure gain, saturation, vibrance, tonemap (0 agx, 1 reinhard)
    agx: vec4f,       // slope, power, offset, reinhard white
    balance: vec4f,   // gain rgb, aspect
    shadows: vec4f,   // offset rgb, gain
    midtones: vec4f,
    highlights: vec4f,
    vignette: vec4f,  // radius, softness, intensity, elliptical
}

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> accum: array<vec4f>;

struct VsOut {
    @builtin(position) pos: vec4f,
    @location(0) uv: vec2f,
}

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {
    // Fullscreen triangle.
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    var out: VsOut;
    out.pos = vec4f(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2f(x, y);
    return out;
}

fn luminance(c: vec3f) -> f32 {
    return dot(c, LUMA);
}

fn zone_weights(l: f32) -> vec3f {
    let lm = max(l, 0.0);
    let lt = lm / (1.0 + lm);
    let s = 1.0 - smoothstep(0.0, 0.35, lt);
    let h = smoothstep(0.45, 0.9, lt);
    let m = max(1.0 - s - h, 0.0);
    let sum = s + m + h;
    return vec3f(s, m, h) / sum;
}

fn agx_sigmoid(x: f32) -> f32 {
    let x2 = x * x;
    let x4 = x2 * x2;
    return 15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - 0.00232;
}

fn tonemap_agx(c: vec3f) -> vec3f {
    var v = AGX_IN * max(c, vec3f(1e-10));
    v = clamp((log2(v) - AGX_MIN_EV) / (AGX_MAX_EV - AGX_MIN_EV), vec3f(0.0), vec3f(1.0));
    v = vec3f(agx_sigmoid(v.x), agx_sigmoid(v.y), agx_sigmoid(v.z));
    v = pow(max(v * p.agx.x + p.agx.z, vec3f(0.0)), vec3f(p.agx.y));
    v = AGX_OUT * v;
    return pow(clamp(v, vec3f(0.0), vec3f(1.0)), vec3f(2.2));
}

fn tonemap_reinhard(c: vec3f) -> vec3f {
    let l = max(luminance(c), 1e-6);
    let w = p.agx.w;
    let ln = l * (1.0 + l / (w * w)) / (1.0 + l);
    return clamp(c * (ln / l), vec3f(0.0), vec3f(1.0));
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4f {
    let px = min(vec2u(in.uv * vec2f(p.size)), p.size - vec2u(1u));
    let a = accum[px.y * p.size.x + px.x];
    var c = a.xyz / max(a.w, 1.0);
    // 1. exposure, 2. balance
    c = c * p.scalars.x * p.balance.xyz;
    // 3. three-zone grade
    let w = zone_weights(luminance(c));
    let graded = w.x * (c * (1.0 + p.shadows.w) + p.shadows.xyz)
        + w.y * (c * (1.0 + p.midtones.w) + p.midtones.xyz)
        + w.z * (c * (1.0 + p.highlights.w) + p.highlights.xyz);
    c = max(graded, vec3f(0.0));
    // 4. tonemap
    if (p.scalars.w < 0.5) {
        c = tonemap_agx(c);
    } else if (p.scalars.w < 1.5) {
        c = tonemap_reinhard(c);
    } else {
        c = clamp(c,vec3f(0.0),vec3f(1.0));
    }
    // 5. saturation and vibrance
    let l = luminance(c);
    let mx = max(c.x, max(c.y, c.z));
    let mn = min(c.x, min(c.y, c.z));
    var current = 0.0;
    if (mx > 1e-6) { current = (mx - mn) / mx; }
    let s = p.scalars.y * (1.0 + p.scalars.z * (1.0 - current));
    c = max(vec3f(l) + (c - vec3f(l)) * s, vec3f(0.0));
    if (p.art.x > 0.0) {
        c=clamp(c,vec3f(0.0),vec3f(1.0));
        let peak=max(c.x,max(c.y,c.z));
        c=c*(1.0-0.5*p.art.x)+vec3f(peak*0.14*p.art.x);
    }
    // 6. vignette
    if (p.vignette.z > 0.0) {
        var u = (in.uv.x - 0.5) * 2.0;
        var t = (in.uv.y - 0.5) * 2.0;
        let aspect = p.balance.w;
        if (p.vignette.w < 0.5) {
            if (aspect >= 1.0) { u *= aspect; } else { t /= aspect; }
        }
        let r = sqrt(u * u + t * t);
        let f = smoothstep(p.vignette.x, p.vignette.x + max(p.vignette.y, 1e-3), r);
        c *= 1.0 - p.vignette.z * f;
    }
    return vec4f(c, 1.0);
}
