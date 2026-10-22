// Shared analytic pattern sampling; used by material, creation and control.
fn pattern_value(tex: FractalTexture, p: vec3f) -> f32 {
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
