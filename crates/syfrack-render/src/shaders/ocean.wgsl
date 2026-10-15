// Intersectable ocean: irregular, enveloped gravity-wave spectrum, analytic derivatives,
// footprint-filtered capillary normals, crest/shore foam and absorbing water.
// Inspired by GPU Gems 1 chapter 1; dielectric transport follows PBRT 4 ch.9.
// This is a height field, not a fluid solver. Shore distance is the scene DE.
const OCEAN_INTERFACE: u32 = 1u;
const OCEAN_BED: u32 = 2u;

fn ocean_enabled() -> bool { return u.seascape.ocean_wave.x > 0.0; }

// x = elevation above mean sea, yz = exact gradient, w = crest indicator.
fn ocean_wave(p: vec2f) -> vec4f {
    if (u.seascape.ocean_wave.y <= 0.0) { return vec4f(0.0); }
    var height = 0.0;
    var gradient = vec2f(0.0);
    var crest = 0.0;
    // Expose fixed, direct uniform loads to drivers rather than dynamically
    // indexing a local copy of the spectrum in each ray evaluation.
    for (var i = 0u; i < 12u; i++) {
        let mode = u.seascape.ocean_spectrum[i];
        let a = mode.z;
        let phase = dot(p, mode.xy) + mode.w;
        let sharp = 0.25 * u.seascape.ocean_wave.w;
        let envelope_k = vec2f(-mode.y,mode.x)*0.137;
        let envelope_phase = dot(p,envelope_k)+f32(i)*5.173+mode.w*0.071;
        let envelope = 0.65+0.35*sin(envelope_phase);
        let envelope_gradient = 0.35*cos(envelope_phase)*envelope_k;
        // The second harmonic sharpens the leading crest without folding XZ.
        let wave = sin(phase)+sharp*sin(2.0*phase);
        height += a * envelope * wave;
        gradient += a * (envelope*mode.xy*(cos(phase)+2.0*sharp*cos(2.0*phase))+envelope_gradient*wave);
        crest += a * envelope * smoothstep(0.45, 0.95, sin(phase));
    }
    return vec4f(height, gradient, crest / max(0.5 * u.seascape.ocean_wave.y, 1e-6));
}

fn ocean_slope_bound() -> f32 {
    return u.seascape.ocean_bounds.y;
}

fn ocean_height(p: vec2f) -> f32 {
    return u.seascape.settings.y + ocean_wave(p).x;
}

// Infinite hashed domain (no UV wrapping); quintic interpolation and its exact
// gradient. The random cells have no common repeat period with the wave modes.
fn ocean_noise(p: vec2f, seed: u32) -> vec3f {
    let c=vec2i(floor(p)); let f=fract(p);
    let t=f*f*f*(f*(f*6.0-15.0)+10.0);
    let dt=30.0*f*f*(f-1.0)*(f-1.0);
    let a=terrain_hash(vec3i(c.x,0,c.y),seed);
    let b=terrain_hash(vec3i(c.x+1,0,c.y),seed);
    let d=terrain_hash(vec3i(c.x,0,c.y+1),seed);
    let e=terrain_hash(vec3i(c.x+1,0,c.y+1),seed);
    return vec3f(mix(b-a,e-d,t.y)*dt.x,mix(d-a,e-b,t.x)*dt.y,mix(mix(a,b,t.x),mix(d,e,t.x),t.y));
}

fn ocean_normal(p: vec2f, footprint: f32, micro: bool) -> vec3f {
    var gradient = ocean_wave(p).yz;
    if (micro && u.seascape.ocean_wave.y > 0.0) {
        var k = 1.0 / max(u.seascape.ocean_wave.z * 0.09, 0.12);
        var a = u.seascape.ocean_detail.z * 0.32;
        for (var i = 0u; i < 3u; i++) {
            let angle = u.seascape.ocean_detail.x + 0.75 + f32(i) * 1.937;
            let d = vec2f(cos(angle), sin(angle));
            let e = vec2f(-d.y,d.x);
            let q=vec2f(dot(p,d)*k,dot(p,e)*k*0.63)+vec2f(u.seascape.ocean_detail.y*0.11,17.17*f32(i));
            let microfield=ocean_noise(q,u.seed+701u+i*137u);
            gradient += (d*microfield.x+e*microfield.y*0.63)*a*exp(-pow(footprint*k*2.0,2.0));
            k *= 2.37;
            a *= 0.52;
        }
    }
    return safe_normalize(vec3f(-gradient.x, 1.0, -gradient.y));
}

struct OceanHit { t: f32, kind: u32 }

fn ocean_intersection(ro: vec3f, rd: vec3f, t_end: f32) -> OceanHit {
    var out = OceanHit(t_end, 0u);
    if (!ocean_enabled()) { return out; }
    let sea = u.seascape;
    let bed_y = sea.settings.y - sea.ocean_optics.z;
    if (abs(rd.y) > 1e-7) {
        let bed_t = (bed_y - ro.y) / rd.y;
        if (bed_t > 1e-4 && bed_t < out.t) { out = OceanHit(bed_t, OCEAN_BED); }
    }
    let bound = sea.ocean_bounds.x;
    var start = 1e-4;
    var end = out.t;
    if (abs(rd.y) > 1e-7) {
        let t0 = (sea.settings.y - bound - ro.y) / rd.y;
        let t1 = (sea.settings.y + bound - ro.y) / rd.y;
        start = max(start, min(t0, t1));
        end = min(end, max(t0, t1));
    } else if (abs(ro.y - sea.settings.y) > bound + 1e-5) {
        return out;
    }
    if (start > end || end < 1e-4) { return out; }
    // Global Lipschitz bound prevents a ray jumping over a crest. Intersections
    // do not depend on image resolution or shading normal filtering.
    let speed_bound = max(abs(rd.y) + ocean_slope_bound() * length(rd.xz), 1e-6);
    let curvature_bound = u.seascape.ocean_bounds.z * dot(rd.xz, rd.xz);
    // Very steep author-defined waves need a larger search budget at grazing
    // angles. Keep it uniform-dependent to avoid static unrolling on Kepler.
    let steepness = PI * sea.ocean_wave.y / sea.ocean_wave.z;
    let iterations = 128u + u32(clamp(steepness - 0.3, 0.0, 1.0) * 896.0);
    var t = start;
    for (var i = 0u; i < iterations; i++) {
        let p = ro + rd * t;
        let field = ocean_wave(p.xz);
        let error = p.y - sea.settings.y - field.x;
        if (abs(error) < 2e-5) { return OceanHit(t, OCEAN_INTERFACE); }
        // A Taylor remainder bound accelerates convergence without discarding
        // the first intersection. Unlike an unconstrained Newton step, both
        // candidate distances certify the entire interval before the next ray point.
        let derivative = abs(rd.y - dot(field.yz, rd.xz));
        let taylor_step = 2.0 * abs(error) / max(derivative + sqrt(derivative * derivative + 2.0 * curvature_bound * abs(error)), 1e-6);
        let step = max(abs(error) / speed_bound, taylor_step);
        t += max(step * 0.95, 1e-5);
        if (t > end) { break; }
    }
    return out;
}

fn ocean_inside(p: vec3f) -> bool {
    return ocean_enabled() && p.y < ocean_height(p.xz);
}

// Exact unpolarised dielectric Fresnel, including total internal reflection.
// eta = incident / transmitted index, the convention of WGSL refract().
fn ocean_fresnel(cosine: f32, eta: f32) -> f32 {
    let ci = clamp(abs(cosine), 0.0, 1.0);
    let st2 = eta * eta * (1.0 - ci * ci);
    if (st2 >= 1.0) { return 1.0; }
    let ct = sqrt(max(0.0, 1.0 - st2));
    let rs = (eta * ci - ct) / max(eta * ci + ct, 1e-6);
    let rp = (ci - eta * ct) / max(ci + eta * ct, 1e-6);
    return 0.5 * (rs * rs + rp * rp);
}

fn ocean_extinction() -> vec3f {
    return u.seascape.ocean_absorption.xyz + vec3f(u.seascape.ocean_absorption.w);
}

fn ocean_transmittance(distance: f32) -> vec3f {
    return exp(-ocean_extinction() * max(distance, 0.0));
}

// Single-scattering source approximation; deliberately no invented caustics.
fn ocean_inscatter(transmittance: vec3f) -> vec3f {
    let sea = u.seascape;
    let albedo = sea.ocean_absorption.w / max(ocean_extinction(), vec3f(1e-6));
    return sea.water.xyz * sea.settings.z * albedo * (1.0 - transmittance);
}

fn ocean_foam(p: vec3f, footprint: f32) -> f32 {
    let o = u.seascape;
    if (o.ocean_optics.x <= 0.0) { return 0.0; }
    let waves = ocean_wave(p.xz);
    let steepness = PI * o.ocean_wave.y / o.ocean_wave.z;
    let foam_patch = ocean_noise(p.xz*0.29+vec2f(3.1,o.ocean_detail.y*0.07),u.seed+307u);
    let crest = smoothstep(0.43, 0.67, waves.w) * o.ocean_wave.w * smoothstep(0.10, 0.28, steepness)
        * smoothstep(0.42,0.78,foam_patch.z);
    var shore = 0.0;
    if (o.ocean_optics.y > 0.0) {
        var sample = de_sample(p);
        if (sample.flags != 0u && sample.distance < o.ocean_optics.y * 1.5) {
            sample = de_sample_exact(p);
        }
        if (sample.flags == 0u) {
            let distance = max(sample.distance, 0.0);
            let edge = 1.0 - smoothstep(0.0, o.ocean_optics.y, distance);
            let wash = 0.5+0.5*ocean_noise(p.xz*1.37+vec2f(waves.x,o.ocean_detail.y*0.17),u.seed+401u).z;
            shore = edge * wash;
        }
    }
    // Resolved cellular gaps at close range; their mean survives minification.
    let warp=vec2f(foam_patch.x,foam_patch.y)*0.9;
    let q = p.xz * 3.71 + warp + vec2f(o.ocean_detail.y * 0.17, -o.ocean_detail.y * 0.11);
    let resolved = exp(-pow(footprint * 3.71, 2.0));
    let lace=ocean_noise(q,u.seed+509u).z*0.72+ocean_noise(q*2.31,u.seed+619u).z*0.28;
    let bubbles = mix(0.68, smoothstep(0.28, 0.70, lace), resolved);
    return clamp(max(crest, shore) * bubbles * o.ocean_optics.x * 1.65, 0.0, 1.0);
}

fn ocean_surface(p: vec3f, n: vec3f, v: vec3f, footprint: f32, kind: u32, underwater: bool) -> Surface {
    var s: Surface;
    s.n = n;
    s.diffuse = u.seascape.ocean_bed.xyz;
    s.f0 = vec3f(0.04);
    s.alpha = 0.8 * 0.8;
    s.emission = vec3f(0.0);
    s.water = 0.0;
    s.eta = 1.0;
    s.transmission = 0.0;
    s.transmission_pdf = 0.0;
    if (kind == OCEAN_INTERFACE) {
        let foam = ocean_foam(p, footprint);
        let facing = select(1.0, -1.0, underwater);
        s.n = ocean_normal(p.xz, footprint, true) * facing;
        s.diffuse = u.seascape.ocean_foam.xyz * foam;
        s.eta = select(1.0 / u.seascape.ocean_optics.w, u.seascape.ocean_optics.w, underwater);
        let f0 = pow((u.seascape.ocean_optics.w - 1.0) / (u.seascape.ocean_optics.w + 1.0), 2.0);
        s.f0 = vec3f(f0);
        let wind_patch=ocean_noise(p.xz*0.19,u.seed+809u).z;
        let roughness = mix(u.seascape.ocean_detail.w*(0.82+0.4*wind_patch), 0.6, foam);
        s.alpha = roughness * roughness;
        s.water = 1.0;
        s.transmission = (1.0 - foam) * (1.0 - ocean_fresnel(dot(n, v), s.eta));
        if (s.transmission > 0.0) { s.transmission_pdf = clamp(s.transmission, 0.05, 0.95); }
    }
    return s;
}

// Straight shadow paths through water with extinction/Fresnel loss. This is
// an approximation to refracted NEE and does not focus light into caustics.
fn ocean_shadow_transmittance(ro: vec3f, rd: vec3f, distance: f32) -> vec3f {
    if (!ocean_enabled()) { return vec3f(1.0); }
    var origin = ro;
    var remaining = distance;
    var inside = ocean_inside(origin);
    var tr = vec3f(1.0);
    for (var i = 0u; i < 8u; i++) {
        let h = ocean_intersection(origin, rd, remaining);
        if (inside) { tr *= ocean_transmittance(h.t); }
        if (h.kind == 0u) { return tr; }
        if (h.kind == OCEAN_BED) { return vec3f(0.0); }
        let p = origin + rd * h.t;
        let n = ocean_normal(p.xz, 0.0, false) * select(1.0, -1.0, inside);
        let eta = select(1.0 / u.seascape.ocean_optics.w, u.seascape.ocean_optics.w, inside);
        tr *= 1.0 - ocean_fresnel(dot(n, -rd), eta);
        remaining -= h.t + 1e-3;
        if (remaining <= 0.0) { return tr; }
        origin = p + rd * 1e-3;
        inside = ocean_inside(origin);
    }
    return vec3f(0.0);
}
