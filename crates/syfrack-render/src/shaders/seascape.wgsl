// Distant procedural world. Sun disks and illumination share the scene's Sun
// lights. The sea is an analytic horizontal surface beyond the finite form.
fn coast_hash(p: vec2f) -> f32 {
    var q = fract(vec3f(p.x, p.y, p.x) * 0.1031);
    q += vec3f(dot(q, q.yzx + vec3f(33.33)));
    return fract((q.x + q.y) * q.z);
}

fn coast_noise(p: vec2f) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let w = f * f * (vec2f(3.0) - 2.0 * f);
    return mix(mix(coast_hash(i), coast_hash(i + vec2f(1.0, 0.0)), w.x),
        mix(coast_hash(i + vec2f(0.0, 1.0)), coast_hash(i + vec2f(1.0)), w.x), w.y);
}

fn coast_fbm(p: vec2f) -> f32 {
    var q = p;
    var value = 0.0;
    var weight = 0.5;
    for (var i = 0u; i < 5u; i++) {
        value += weight * coast_noise(q);
        q = vec2f(1.6 * q.x - 1.2 * q.y, 1.2 * q.x + 1.6 * q.y) + vec2f(7.1, 3.4);
        weight *= 0.5;
    }
    return value;
}

fn coast_sky(dir: vec3f) -> vec3f {
    let sea = u.seascape;
    let elevation = max(dir.y, 0.0);
    var sky = mix(sea.horizon.xyz, sea.zenith.xyz, pow(clamp(elevation / 0.65, 0.0, 1.0), 0.55));
    let pink_band = exp(-pow((elevation - 0.13) / 0.09, 2.0));
    sky += vec3f(0.24, 0.035, 0.085) * pink_band;
    var warm = vec3f(0.0);
    for (var i = 0u; i < u.light_count; i++) {
        let sun = u.lights[i];
        if (sun.kind.x != KIND_SUN) { continue; }
        let a = safe_normalize(vec3f(dir.x, 0.0, dir.z));
        let b = safe_normalize(vec3f(sun.a.x, 0.0, sun.a.z));
        let glow = exp(14.0 * (dot(a, b) - 1.0)) * exp(-abs(dir.y - sun.a.y) * 5.0);
        warm += sun.color.xyz * glow * 0.9;
    }
    sky += warm;
    // Horizontally stretched, multiscale clouds on a distant sky plane.
    let q = dir.xz / max(elevation + 0.13, 0.13);
    let cloud_field = coast_fbm(vec2f(q.x * 0.75 + 8.0, q.y * 2.8 + 4.0));
    let cloud = smoothstep(0.55 - sea.detail.y * 0.2, 0.75 - sea.detail.y * 0.1, cloud_field)
        * sea.detail.y * smoothstep(0.01, 0.12, elevation);
    let cloud_light = mix(vec3f(0.075, 0.055, 0.13), sea.horizon.xyz * 0.7 + warm * 0.7, exp(-elevation * 3.0));
    sky = mix(sky, cloud_light, cloud * 0.75);
    return max(sky, vec3f(0.0)) * sea.settings.z;
}

fn coast_water_normal(p: vec2f, footprint: f32) -> vec3f {
    var gradient = vec2f(0.0);
    var amplitude = u.seascape.settings.w;
    var frequency = u.seascape.detail.x;
    for (var i = 0u; i < 6u; i++) {
        let angle = f32(i) * 2.39996 + 0.32;
        let direction = vec2f(cos(angle), sin(angle));
        // Analytic footprint filtering removes distant wave aliasing.
        let wave_weight = exp(-pow(footprint * frequency, 2.0));
        let phase = dot(p, direction) * frequency + f32(i) * 1.73;
        gradient += direction * cos(phase) * amplitude * wave_weight;
        frequency *= 1.9;
        amplitude *= 0.62;
    }
    return safe_normalize(vec3f(-gradient.x, 1.0, -gradient.y));
}

fn coast_radiance(ro: vec3f, rd: vec3f, pixel_cone: f32, previous_pdf: f32, from_bsdf: bool) -> vec3f {
    let sea = u.seascape;
    if (rd.y >= -1e-6 || ro.y <= sea.settings.y) {
        return coast_sky(rd) + sun_radiance(rd, previous_pdf, from_bsdf);
    }
    let distance = (sea.settings.y - ro.y) / rd.y;
    let p = ro + rd * distance;
    let n = coast_water_normal(p.xz, max(distance * pixel_cone, 0.01));
    let nv = max(dot(n, -rd), 0.0);
    let f = 0.02 + 0.98 * pow(1.0 - nv, 5.0);
    let reflected = reflect(rd, n);
    var water = coast_sky(reflected) * f + sea.water.xyz * (1.0 - f) * sea.settings.z;
    for (var i = 0u; i < u.light_count; i++) {
        let sun = u.lights[i];
        if (sun.kind.x != KIND_SUN) { continue; }
        let h = safe_normalize(sun.a.xyz - rd);
        let nh = max(dot(n, h), 0.0);
        let alpha = 0.06 + 0.08 * sea.settings.w;
        let d = ggx_d(nh, alpha * alpha);
        let nl = max(dot(n, sun.a.xyz), 0.0);
        let spec = (0.02 + 0.98 * pow(1.0 - max(dot(h, -rd), 0.0), 5.0))
            * d * smith_g1(max(nv, 0.001), alpha * alpha) * smith_g1(max(nl, 0.001), alpha * alpha)
            / max(4.0 * nv, 0.001);
        water += sun.color.xyz * min(spec, 24.0);
    }
    let haze = (1.0 - exp(-distance * 0.0007 * sea.detail.z)) * 0.6;
    return mix(water, coast_sky(vec3f(rd.x, 0.0, rd.z)), haze);
}
