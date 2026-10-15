// Equirectangular linear RGB plus luminance/solid-angle CDF.
// A uniform component covers bilinear-filtered bright edges and CDF roundoff.
@group(0) @binding(6) var<storage, read> environment_texels: array<vec4f>;
const KIND_HDRI: u32 = 3u;
const HDRI_UNIFORM_MIX: f32 = 0.0625;

fn hdri_local(light: Light, dir: vec3f) -> vec3f {
    return vec3f(light.a.x * dir.x + light.a.y * dir.z, dir.y,
        -light.a.y * dir.x + light.a.x * dir.z);
}

fn hdri_world(light: Light, dir: vec3f) -> vec3f {
    return vec3f(light.a.x * dir.x - light.a.y * dir.z, dir.y,
        light.a.y * dir.x + light.a.x * dir.z);
}

fn hdri_uv(light: Light, dir: vec3f) -> vec2f {
    let local = hdri_local(light, dir);
    return vec2f(fract(atan2(local.z, local.x) / (2.0 * PI) + 1.0),
        acos(clamp(local.y, -1.0, 1.0)) / PI);
}

fn hdri_cell_area(light: Light, row: u32) -> f32 {
    let step = PI / f32(light.kind.w);
    return (2.0 * PI / f32(light.kind.z)) * 2.0
        * sin((f32(row) + 0.5) * step) * sin(step * 0.5);
}

fn hdri_texel(light: Light, x: i32, y: i32) -> vec3f {
    let width = i32(light.kind.z);
    let column = u32(((x % width) + width) % width);
    let row = u32(clamp(y, 0, i32(light.kind.w) - 1));
    return environment_texels[light.kind.y + row * light.kind.z + column].xyz;
}

fn hdri_lookup(light: Light, dir: vec3f) -> vec3f {
    let position = hdri_uv(light, dir) * vec2f(f32(light.kind.z), f32(light.kind.w)) - 0.5;
    let base = vec2i(floor(position));
    let fraction = fract(position);
    let top = mix(hdri_texel(light, base.x, base.y),
        hdri_texel(light, base.x + 1, base.y), fraction.x);
    let bottom = mix(hdri_texel(light, base.x, base.y + 1),
        hdri_texel(light, base.x + 1, base.y + 1), fraction.x);
    return mix(top, bottom, fraction.y) * light.color.x;
}

fn hdri_pdf(light: Light, dir: vec3f) -> f32 {
    if (light.a.z > 0.0) { return 1.0 / (4.0 * PI); }
    let uv = hdri_uv(light, dir);
    let col = min(u32(uv.x * f32(light.kind.z)), light.kind.z - 1u);
    let row = min(u32(uv.y * f32(light.kind.w)), light.kind.w - 1u);
    let index = row * light.kind.z + col;
    var before = 0.0;
    if (index > 0u) { before = environment_texels[light.kind.y + index - 1u].w; }
    let mass = max(environment_texels[light.kind.y + index].w - before, 0.0);
    return (1.0 - HDRI_UNIFORM_MIX) * mass / hdri_cell_area(light, row)
        + HDRI_UNIFORM_MIX / (4.0 * PI);
}

fn hdri_sample(light: Light, r0: f32, r1: f32, r2: f32) -> vec3f {
    var phi = 2.0 * PI * r1;
    var cos_theta = 1.0 - 2.0 * r2;
    var sin_squared = max(0.0, 1.0 - cos_theta * cos_theta);
    if (light.a.z <= 0.0 && r0 >= HDRI_UNIFORM_MIX) {
        let cdf_sample = (r0 - HDRI_UNIFORM_MIX) / (1.0 - HDRI_UNIFORM_MIX);
        var lo = 0u;
        var hi = light.kind.z * light.kind.w;
        while (lo < hi) {
            let mid = lo + (hi - lo) / 2u;
            if (environment_texels[light.kind.y + mid].w <= cdf_sample) {
                lo = mid + 1u;
            } else { hi = mid; }
        }
        let index = min(lo, light.kind.z * light.kind.w - 1u);
        let col = index % light.kind.z;
        let row = index / light.kind.z;
        phi = 2.0 * PI * (f32(col) + r1) / f32(light.kind.z);
        let top = PI * f32(row) / f32(light.kind.w);
        let bottom = PI * f32(row + 1u) / f32(light.kind.w);
        let cap = 2.0 * sin(top * 0.5) * sin(top * 0.5)
            + r2 * 2.0 * sin((top + bottom) * 0.5) * sin((bottom - top) * 0.5);
        let lower_cap = 2.0 * cos(bottom * 0.5) * cos(bottom * 0.5)
            + (1.0 - r2) * 2.0 * sin((top + bottom) * 0.5) * sin((bottom - top) * 0.5);
        if (row < light.kind.w / 2u) {
            cos_theta = 1.0 - cap;
            sin_squared = cap * (2.0 - cap);
        } else {
            cos_theta = lower_cap - 1.0;
            sin_squared = lower_cap * (2.0 - lower_cap);
        }
    }
    let sin_theta = sqrt(max(0.0, sin_squared));
    return hdri_world(light, vec3f(sin_theta * cos(phi), cos_theta, sin_theta * sin(phi)));
}

fn hdri_emission(dir: vec3f, previous_pdf: f32, use_mis: bool) -> vec3f {
    var L = vec3f(0.0);
    for (var i = 0u; i < u.light_count; i++) {
        let light = u.lights[i];
        if (light.kind.x != KIND_HDRI) { continue; }
        var weight = 1.0;
        if (use_mis) { weight = mis_power(previous_pdf, hdri_pdf(light, dir)); }
        L += hdri_lookup(light, dir) * weight;
    }
    return L;
}
