// Continuous aeolian terrain. Heights and shading normals use the same field;
// mineral-scale sand detail is evaluated only at accepted surface hits.
fn dune_profile(xz: vec2f) -> vec3f {
    let d = u.atmosphere.dunes;
    let k = 2.0 * PI / max(d.y, 1.0);
    let q = rot2(xz, d.z) * k;
    let seed = u32(d.w);
    let warp = finish_noise(vec3f(q.x * 0.27, 0.0, q.y * 0.27), seed);
    let variation = finish_noise(vec3f(q.x * 0.42 + 17.0, 0.0, q.y * 0.42 - 4.0), seed + 51u);
    let phase = q.x + 2.0 * warp.w + 0.55 * sin(q.y * 0.48);
    let phase_gradient = vec2f(1.0, 0.264 * cos(q.y * 0.48)) + 0.54 * warp.xz;
    let height = d.x * (0.55 + 0.32 * sin(phase) + 0.10 * sin(2.0 * phase) + 0.22 * variation.w);
    let gradient = d.x * ((0.32 * cos(phase) + 0.20 * cos(2.0 * phase)) * phase_gradient + 0.0924 * variation.xz) * k;
    return vec3f(rot2(gradient, -d.z), u.atmosphere.ground.y + height);
}

fn ground_intersection(ro: vec3f, rd: vec3f, pixel_cone: f32, t_end: f32) -> f32 {
    if (u.atmosphere.ground.x == 0.0) { return t_end; }
    let d = u.atmosphere.dunes;
    if (d.x <= 0.0) {
        if (abs(rd.y) < 1e-7) { return t_end; }
        let t = (u.atmosphere.ground.y - ro.y) / rd.y;
        return select(t_end, t, t > 1e-5 && t < t_end);
    }
    let bottom = u.atmosphere.ground.y;
    let top = bottom + 1.20 * d.x;
    var t = 0.0;
    var end = t_end;
    if (abs(rd.y) > 1e-7) {
        let a = (bottom - ro.y) / rd.y;
        let b = (top - ro.y) / rd.y;
        t = max(0.0, min(a, b));
        end = min(end, max(a, b));
    } else if (ro.y < bottom || ro.y > top) { return t_end; }
    // Analytic upper bound on the profile slope (including the noise warp).
    let slope = 9.5 * d.x / max(d.y, 1.0);
    let speed = max(abs(rd.y) + slope * length(rd.xz), 1e-5);
    for (var i = 0u; i < 512u; i++) {
        if (t >= end) { break; }
        let p = ro + rd * t;
        let distance = abs(p.y - dune_profile(p.xz).z);
        let epsilon = max(0.00015, t * pixel_cone * 0.15);
        if (distance < epsilon && t > 1e-5) { return t; }
        t += max(distance * 0.90 / speed, 0.0001);
    }
    // Grazing rays can spend the entire conservative budget above a long
    // ridge. Finish the remaining bounded interval with a sampled bracket,
    // then refine its first detected crossing; never expose a hole in the sky.
    if (t < end) {
        let step = max(0.05, (end - t) / 511.0);
        var previous = t;
        for (var i = 0u; i < 512u; i++) {
            let p = ro + rd * t;
            if (p.y <= dune_profile(p.xz).z) {
                var a = previous; var b = t;
                for (var j = 0u; j < 12u; j++) {
                    let middle = (a + b) * 0.5;
                    let probe = ro + rd * middle;
                    if (probe.y > dune_profile(probe.xz).z) { a = middle; } else { b = middle; }
                }
                return (a + b) * 0.5;
            }
            if (t >= end) { break; }
            previous = t;
            t = min(end, t + step);
        }
    }
    return t_end;
}

fn ground_normal(p: vec3f, rd: vec3f) -> vec3f {
    var n = vec3f(0.0, 1.0, 0.0);
    if (u.atmosphere.dunes.x > 0.0) {
        let profile = dune_profile(p.xz);
        n = safe_normalize(vec3f(-profile.x, 1.0, -profile.y));
    }
    return select(n, -n, dot(n, rd) > 0.0);
}

// The aperture transports camera and BSDF rays to an independent procedural
// landscape. Its surfaces continue through the same multi-bounce GGX integrator.
// The remote coordinate interval is reserved only while the feature is enabled.
const WORLD_OFFSET: vec3f = vec3f(0.0, 0.0, -16384.0);
fn in_otherworld(p: vec3f) -> bool {
    return u.atmosphere.portal.w > 0.0 && p.z < -8192.0;
}
fn portal_intersection(ro: vec3f, rd: vec3f, end: f32) -> f32 {
    let gate = u.atmosphere.portal;
    if (gate.w <= 0.0 || in_otherworld(ro) || rd.z >= -1e-7 || ro.z <= gate.z) { return end; }
    let t = (gate.z - ro.z) / rd.z;
    let p = ro + t * rd - gate.xyz;
    return select(end, t, t > 1e-5 && t < end && dot(p.xy, p.xy) < gate.w * gate.w);
}

fn otherworld_height(xz: vec2f) -> f32 {
    let q = xz * 0.007;
    var n = 0.0;
    var frequency = 1.0;
    var weight = 0.5;
    for (var i = 0u; i < 5u; i++) {
        let value = finish_noise(vec3f(q.x * frequency, 1.3, q.y * frequency), 701u + i * 79u).w;
        n += weight * (1.0 - abs(2.0 * value - 1.0));
        frequency *= 2.07;
        weight *= 0.5;
    }
    let valley = xz.x + 35.0 * sin(xz.y * 0.012 + 0.5);
    let walls = smoothstep(9.0, 90.0, abs(valley));
    let distance = smoothstep(25.0, 180.0, -xz.y);
    return 4.0 + 170.0 * pow(n, 3.0) * (0.18 + 0.82 * walls) * distance;
}

fn otherworld_trace(ro: vec3f, rd: vec3f, cone: f32, end: f32) -> Hit {
    var h: Hit;
    h.hit = false; h.t = end; h.steps = 0u; h.exhausted = false;
    h.light_index = NO_LIGHT; h.ocean_kind = 0u; h.ground = false;
    h.sample = DeSample(0.0, vec4f(1.0), 0u, 0u, 0u);
    let origin = ro - WORLD_OFFSET;
    var t = 0.002;
    // The field is conservatively stepped; the below-surface crossing is
    // refined as well, including grazing rays and deep mountain silhouettes.
    var previous = t;
    for (var i = 0u; i < 320u; i++) {
        let p = origin + t * rd;
        if (t >= min(end, 2500.0) || (p.y > 180.0 && rd.y > 0.0)) { break; }
        let gap = p.y - otherworld_height(p.xz);
        if (gap < max(0.002, t * cone * 0.12)) {
            if (gap < 0.0 && i > 0u) {
                var a = previous; var b = t;
                for (var j = 0u; j < 9u; j++) {
                    let m = 0.5 * (a + b); let probe = origin + rd * m;
                    if (probe.y > otherworld_height(probe.xz)) { a = m; } else { b = m; }
                }
                t = 0.5 * (a + b);
            }
            h.hit = true; h.t = t; h.steps = i + 1u;
            return h;
        }
        previous = t;
        t += max(0.002, gap * 0.11);
    }
    return h;
}
fn otherworld_normal(p: vec3f) -> vec3f {
    let q = p - WORLD_OFFSET;
    let e = 0.06;
    let dx = otherworld_height(q.xz + vec2f(e, 0.0)) - otherworld_height(q.xz - vec2f(e, 0.0));
    let dz = otherworld_height(q.xz + vec2f(0.0, e)) - otherworld_height(q.xz - vec2f(0.0, e));
    return safe_normalize(vec3f(-dx, 2.0 * e, -dz));
}
fn otherworld_sky(dir: vec3f) -> vec3f {
    let tint = u.atmosphere.portal_color.xyz;
    let height = max(dir.y, 0.0);
    var sky = tint * mix(0.38, 0.012, smoothstep(0.0, 0.7, height));
    let cloud = finish_fbm(vec3f(dir.x * 4.0, dir.y * 9.0, dir.z * 4.0), 0.0, 819u).w;
    sky += tint * smoothstep(0.48, 0.75, cloud) * 0.18;
    let moon = safe_normalize(vec3f(-0.30, 0.23, -1.0));
    let cosine = dot(dir, moon);
    let radius = 0.065;
    if (cosine > cos(radius)) {
        let delta = (dir - moon * cosine) / sin(radius);
        let nz = sqrt(max(0.0, 1.0 - dot(delta, delta)));
        let normal = safe_normalize(delta - moon * nz);
        let phase = max(dot(normal, safe_normalize(vec3f(-0.8, 0.5, 0.3))), 0.0);
        let craters = finish_fbm(normal * 23.0, 0.0, 191u).w;
        sky = mix(tint * 0.12, vec3f(0.14, 0.55, 1.4), phase) * (0.6 + 0.5 * craters);
    }
    sky += tint * 0.035 * exp(-max(0.0, acos(clamp(cosine, -1.0, 1.0)) - radius) * 32.0);
    return sky;
}
fn otherworld_surface(p: vec3f, n: vec3f, footprint: f32) -> Surface {
    let q = p - WORLD_OFFSET;
    let rock = finish_fbm(q * 0.8, footprint * 0.8, 1901u);
    var s: Surface;
    s.n = safe_normalize(n - (rock.xyz - n * dot(rock.xyz, n)) * 0.08);
    s.diffuse = mix(vec3f(0.025, 0.045, 0.07), vec3f(0.18, 0.30, 0.42), rock.w);
    s.f0 = vec3f(0.05); s.alpha = 0.42; s.emission = vec3f(0.0);
    s.water = 0.0; s.eta = 1.0; s.transmission = 0.0; s.transmission_pdf = 0.0;
    return s;
}
fn otherworld_direct(p: vec3f, s: Surface, geometric_n: vec3f, v: vec3f, cone: f32, rng: ptr<function, u32>) -> vec3f {
    let light_direction = safe_normalize(vec3f(-0.7, 0.45, 0.25));
    let dir = cone_sample(light_direction, cos(0.015), rand(rng), rand(rng));
    let nl = dot(s.n, dir);
    if (nl <= 0.0 || dot(geometric_n, dir) <= 0.0) { return vec3f(0.0); }
    if (otherworld_trace(p, dir, cone, 2500.0).hit) { return vec3f(0.0); }
    return brdf_eval(s, v, dir) * nl * vec3f(0.8, 2.4, 5.0);
}
