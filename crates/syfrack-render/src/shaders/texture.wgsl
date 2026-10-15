// The form compiler emits texture_de_sample using this independent buffer.
@group(0) @binding(5) var<uniform> texture_params: array<vec4f, 2048>;

fn texture_gp(i: u32) -> f32 {
    return texture_params[i / 4u][i % 4u];
}

fn texture_value(p: vec3f) -> f32 {
    let tex = u.texture;
    var q = rotate_euler_xyz(p * tex.mapping.w, tex.rotation.xyz) + tex.mapping.xyz;
    var fan_angle = 0.0;
    var fan_radius = 0.0;
    if (tex.fan.w > 0.0) {
        q = rotate_euler_xyz(p, tex.rotation.xyz) + tex.mapping.xyz;
        let row = floor(q.z / tex.fan.y);
        let x = modf_euclid(q.x + modf_euclid(row, 2.0) * tex.fan.x * 0.5, tex.fan.x) - tex.fan.x * 0.5;
        let z = modf_euclid(q.z, tex.fan.y) + 0.03;
        fan_angle = atan2(x, z);
        fan_radius = length(vec2f(x, z));
        q = vec3f(fan_angle * tex.fan.z / PI, q.y, fan_radius) * tex.mapping.w;
    }
    var total = 0.0;
    var weights = 0.0;
    var weight = 1.0;
    for (var octave = 0u; octave < tex.kind.y; octave++) {
        var sample: DeSample;
        if (tex.planar.kind.x != 0u) { sample = planar_sample(tex.planar,q.xy); } else { sample = texture_de_sample(q); }
        var value = 0.5;
        if (tex.kind.x == 7u) {
            value = dot(tex.weights,sample.traps) + tex.extra.x*f32(sample.iterations)/max(f32(tex.kind.z),1.0) + tex.extra.y*0.5;
            let gain = tex.extra.z;
            if (gain > 0.0) { value = sign(value)*log(1.0+gain*abs(value))/log(1.0+gain); }
        } else if (tex.kind.x < 4u) {
            value = sample.traps[tex.kind.x];
        } else if (tex.kind.x == COLOR_SLOT_ITER) {
            value = f32(sample.iterations) / max(f32(tex.kind.z), 1.0);
        } else {
            value = 0.5 + 0.5 * sample.distance / (1.0 + abs(sample.distance));
        }
        // Covers NaN and infinities, as in the CPU sampler.
        if (!(value >= -3.4028235e38 && value <= 3.4028235e38)) { value = 0.5; }
        total += select(clamp(value,0.0,1.0),value,tex.kind.x == 7u) * weight;
        weights += weight;
        q = q * 2.0 + vec3f(0.73, 1.17, 0.91);
        weight *= 0.5;
    }
    let fractal = total / max(weights, 1e-6);
    var value = fractal;
    if (tex.fan.w > 0.0) {
        let spoke = pow(1.0 - abs(sin(fan_angle * tex.fan.z + 0.16 * (fractal - 0.5))), 4.0);
        let arc = pow(1.0 - abs(sin(fan_radius * tex.mapping.w * PI + 0.35 * fractal)), 6.0);
        value = 0.16 + 0.28 * fractal + 0.42 * spoke + 0.22 * arc;
    }
    let result = (value-0.5)*tex.rotation.w+0.5;
    return select(clamp(result,0.0,1.0),result,tex.kind.x == 7u);
}

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
