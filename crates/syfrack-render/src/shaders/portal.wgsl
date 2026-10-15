// A thin emissive/transmissive energy sheet. Camera, BSDF and next-event rays
// evaluate the same frozen field; its blue spill is transported, not composited.
// Time is a scene parameter and remains fixed throughout a progressive render.
fn portal_membrane(point: vec3f, footprint: f32) -> vec4f {
    let portal = u.atmosphere.portal;
    let uv = (point.xy - portal.xy) / max(portal.w, 1e-4);
    let radius = length(uv);
    let time = u.atmosphere.portal_world.w;
    let turbulence = u.atmosphere.portal_color.w;
    let width = footprint / max(portal.w, 1e-4);
    let first = finish_fbm(vec3f(uv * 3.1, time * 0.06), width * 3.1, 412u);
    let second = finish_fbm(vec3f(uv.yx * 4.3 + vec2f(5.3, 9.1), time * 0.045), width * 4.3, 961u);
    let domain = uv + vec2f(first.w - 0.5, second.w - 0.5) * 0.65 * turbulence;
    let r = max(length(domain), 1e-5);
    let angle = atan2(domain.y, domain.x);
    let phase = r * 36.0 - angle * 2.0 + first.w * 5.0 - time * 0.35;
    let crossing = domain.y * 31.0 + 3.0 * sin(domain.x * 8.0 + time * 0.09);
    let visible = exp(-pow(width * 55.0, 2.0));
    let filaments = mix(0.093, pow(0.5 + 0.5 * cos(phase), 36.0), visible)
        * smoothstep(0.22, 0.65, second.w);
    let branches = mix(0.080, exp(-abs(sin(crossing)) * 16.0), visible) * 0.25;
    let rim = exp(-max(0.0, 1.0 - radius) * 44.0);
    let cloud = 0.06 + 0.12 * first.w * second.w;
    // The aperture remains a view into a world, with energy concentrated at
    // its boundary and a few translucent streamers across the central vista.
    let envelope = 0.12 + 0.68 * smoothstep(0.28, 0.95, radius);
    let density = (filaments + branches * 0.45) * envelope + 0.85 * rim;
    let emission = u.atmosphere.portal_world.z
        * (vec3f(0.004, 0.025, 0.30) * cloud + vec3f(0.025, 0.65, 3.5) * density);
    let transmission = clamp(0.94 - density * 0.38, 0.25, 0.94);
    return vec4f(emission, transmission);
}

fn portal_pdf(origin: vec3f, point: vec3f) -> f32 {
    let delta = point - origin;
    let distance2 = dot(delta, delta);
    let cosine = (origin.z - point.z) / max(sqrt(distance2), 1e-6);
    if (cosine <= 0.0 || u.atmosphere.portal.w <= 0.0) { return 0.0; }
    return distance2 / max(PI * u.atmosphere.portal.w * u.atmosphere.portal.w * cosine, 1e-8);
}

fn portal_direct(p: vec3f, s: Surface, geometric_n: vec3f, v: vec3f,
    p_spec: f32, cone: f32, last: bool, cell: u32, strength: f32,
    in_fog: bool, rng: ptr<function, u32>) -> vec3f {
    let gate = u.atmosphere.portal;
    if (gate.w <= 0.0 || u.atmosphere.portal_world.z <= 0.0 || p.z <= gate.z) { return vec3f(0.0); }
    let radius = gate.w * sqrt(rand(rng));
    let angle = 2.0 * PI * rand(rng);
    let point = gate.xyz + vec3f(radius * cos(angle), radius * sin(angle), 0.0);
    let delta = point - p;
    let distance = length(delta);
    if (distance < 1e-5) { return vec3f(0.0); }
    let direction = delta / distance;
    let nl = dot(s.n, direction);
    if (nl <= 0.0 || dot(geometric_n, direction) <= 0.0) { return vec3f(0.0); }
    if (!scene_visible(p, direction, cone, shadow_endpoint(distance))) { return vec3f(0.0); }
    let pl = portal_pdf(p, point);
    var pb = 0.0;
    if (!last) { pb = continuation_pdf(s, geometric_n, v, direction, p_spec, cell, strength); }
    var transmittance = 1.0;
    if (in_fog) { transmittance = fog_transmittance(p, direction, distance, rng); }
    return brdf_eval(s, v, direction) * nl * portal_membrane(point, 0.0).xyz
        * transmittance * select(mis_power(pl, pb), 1.0, last) / max(pl, 1e-8);
}
