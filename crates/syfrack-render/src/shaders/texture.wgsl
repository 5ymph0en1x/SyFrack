// The form compiler emits texture_de_sample using this independent buffer.
@group(0) @binding(5) var<uniform> texture_params: array<vec4f, 2048>;
@group(0) @binding(17) var texture_params_overflow: texture_2d<f32>;

fn texture_gp_row(i: u32) -> vec4f {
    if (i < 2048u) { return texture_params[i]; }
    let row = i - 2048u;
    let width = textureDimensions(texture_params_overflow).x;
    return textureLoad(texture_params_overflow, vec2u(row % width, row / width), 0);
}

fn texture_gp(i: u32) -> f32 {
    return texture_gp_row(i / 4u)[i % 4u];
}

fn texture_value(p: vec3f) -> f32 { return pattern_value(u.texture,p); }

fn texture_mask(sample: DeSample) -> f32 {
    let mask = u.texture.mask;
    if (mask.w == 0.0) { return 1.0; }
    return 1.0 - smoothstep(mask.z * 0.5, mask.z, abs(sample.traps[u32(mask.x)] - mask.y));
}

fn texture_color(value: f32) -> vec3f {
    return palette_sample(u.texture.ramp, value);
}

fn texture_filtered_value(p: vec3f, footprint: f32) -> f32 {
    let fade = smoothstep(0.25, 0.75, footprint * u.texture.mapping.w);
    if (fade >= 1.0) { return 0.5; }
    return mix(texture_value(p), 0.5, fade);
}

// Ray/height-field intersection in the tangent plane. The field itself remains
// volumetric, so the same graph works across curved surfaces without UV seams.
fn texture_parallax(p: vec3f, n: vec3f, v: vec3f, footprint: f32) -> vec3f {
    let depth = u.texture.parallax.x / u.texture.mapping.w;
    if (depth <= 0.0 || depth < footprint * 0.25) { return p; }
    let nv = max(dot(n, v), 0.15);
    let direction = (v - n * dot(n, v)) / nv + n;
    let layers = u32(mix(4.0, u.texture.parallax.y, 1.0 - clamp(nv, 0.0, 1.0)));
    let step = 1.0 / f32(max(layers, 4u));
    var previous = texture_value(p) - 1.0;
    var layer = 0.0;
    for (var i = 0u; i < max(layers, 4u); i++) {
        layer += step;
        let q = p - direction * depth * layer;
        let crossing = layer + texture_value(q) - 1.0;
        if (crossing >= 0.0) {
            let fraction = clamp(-previous / max(crossing - previous, 1e-6), 0.0, 1.0);
            return p - direction * depth * (layer - step + step * fraction);
        }
        previous = crossing;
    }
    return p - direction * depth;
}

fn texture_normal(p: vec3f, n: vec3f, footprint: f32) -> vec3f {
    let fade = smoothstep(0.25, 0.75, footprint * u.texture.mapping.w);
    if (fade >= 1.0) { return n; }
    // The pixel footprint filters subpixel relief, keeping distant normals stable.
    let h = max(footprint, 0.005 / u.texture.mapping.w);
    let k = vec2f(1.0, -1.0);
    let gradient = (k.xyy * texture_value(p + k.xyy * h)
        + k.yyx * texture_value(p + k.yyx * h)
        + k.yxy * texture_value(p + k.yxy * h)
        + k.xxx * texture_value(p + k.xxx * h)) / (4.0 * h);
    var tangent = (gradient - n * dot(n, gradient)) * u.texture.effects.z
        * (1.0 - fade) / u.texture.mapping.w;
    // Keep the shading normal within 45 degrees of the geometric normal.
    tangent /= max(1.0, length(tangent));
    return safe_normalize(n - tangent);
}
