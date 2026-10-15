// Distance over which the medium (fog) acts along any ray.
fn fog_extent() -> f32 { return ray_extent(); }

// SyFrack path tracer body; prepended with common.wgsl at build time.

@group(0) @binding(1) var<storage, read_write> accum: array<vec4f>;
@group(0) @binding(3) var<storage, read_write> aux: array<vec4f>;
@group(0) @binding(12) var<storage, read_write> volume_accum: array<vec4f>;
@group(0) @binding(13) var<storage, read_write> volume_moments: array<vec4f>;
@group(0) @binding(14) var<storage, read_write> surface_accum: array<vec4f>;
struct VolumeComponents {
    direct: vec4f,
    direct_moments: vec4f,
    indirect_moments: vec4f,
}
@group(0) @binding(15) var<storage, read_write> volume_components: array<VolumeComponents>;

// ---------------------------------------------------------------- RNG

fn pcg(v: u32) -> u32 {
    let state = v * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn rng_init(px: vec2u, sample: u32, seed: u32) -> u32 {
    var h = pcg(seed);
    h = pcg(h ^ px.x);
    h = pcg(h ^ px.y);
    h = pcg(h ^ sample);
    return h;
}

fn rand(state: ptr<function, u32>) -> f32 {
    *state = pcg(*state);
    return f32(*state >> 8u) * (1.0 / 16777216.0);
}

// Randomly shifted Halton point cloud. Uniform marginals preserve the light
// integral, while the distance/area/selection coordinates fill their domains
// progressively instead of leaving large holes. Each pixel and emitter have
// independent rotations; neither tile size nor dispatch chunking changes them.
fn volume_coordinate(sample_index: u32, dimension: u32, stream: u32) -> f32 {
    let bases = array<u32,6>(2u,3u,5u,7u,11u,13u);
    let base = bases[dimension];
    var index = sample_index+1u;
    var inverse = 1.0/f32(base);
    var value = 0.0;
    loop {
        if (index==0u) { break; }
        value += f32(index%base)*inverse;
        index /= base;
        inverse /= f32(base);
    }
    let shift = f32(pcg(stream ^ (dimension*0x9e3779b9u))>>8u)/16777216.0;
    return fract(value+shift);
}

// ---------------------------------------------------------------- Distance field

struct Hit {
    hit: bool,
    t: f32,
    steps: u32,
    sample: DeSample,
    exhausted: bool,
    light_index: u32,
    ocean_kind: u32,
    ground: bool,
}

// Sphere tracing with over-relaxation and fallback (Keinert et al. 2014).
// `pixel_cone` is the angular pixel footprint; the hit threshold scales with t.
fn march_segment(ro: vec3f, rd: vec3f, pixel_cone: f32, t_end: f32) -> Hit {
    let safety = u.fractal.p0.w;
    let hit_cone = pixel_cone * u.fractal.p0.x;
    var t = 0.0;
    var omega = 1.2;
    var prev_radius = 0.0;
    var step_len = 0.0;
    var cand_t = 0.0;
    var cand_sample = DeSample(T_MAX, vec4f(1.0), 0u, 0u, 0u);
    var cand_err = 1e30;
    var out: Hit;
    out.hit = false;
    out.t = t_end;
    out.steps = 0u;
    out.exhausted = false;
    out.light_index = NO_LIGHT;
    out.ocean_kind = 0u;
    out.ground = false;
    out.sample = DeSample(T_MAX, vec4f(1.0), 0u, 0u, 0u);
    for (var i = 0u; i < u.max_steps; i++) {
        if (t >= t_end) { return out; }
        out.steps = i + 1u;
        var s = de_sample(ro + rd * t);
        var radius = abs(s.distance) * safety;
        // A bounding box is only a stepping aid. Never accept it as a surface,
        // including the relaxed candidate at the end of the step budget.
        if (s.flags != 0u && radius / max(t, 1e-6) <= hit_cone * 5.0) {
            s = de_sample_exact(ro + rd * t);
            radius = abs(s.distance) * safety;
        }
        let sor_fail = omega > 1.0 && (radius + prev_radius) < step_len * 1.0001;
        if (sor_fail) {
            step_len = step_len - omega * step_len;
            omega = 1.0;
        } else {
            step_len = radius * omega;
        }
        prev_radius = radius;
        let err = radius / max(t, 1e-6);
        if (!sor_fail && s.flags == 0u && err < cand_err) {
            cand_t = t;
            cand_sample = s;
            cand_err = err;
        }
        if (!sor_fail && s.flags == 0u && err < hit_cone) {
            out.hit = true;
            out.t = t;
            out.sample = s;
            return out;
        }
        // An over-relaxed step must not jump over an occluder just before
        // the segment endpoint. Only a conservative radius can prove it clear.
        if (!sor_fail && t + step_len >= t_end) {
            if (radius >= t_end - t) { return out; }
            omega = 1.0;
            step_len = radius;
        }
        t += step_len;
    }
    if (cand_err < hit_cone * 4.0) {
        out.hit = true;
        out.t = cand_t;
        out.sample = cand_sample;
    } else {
        out.exhausted = true;
    }
    return out;
}

// ---------------------------------------------------------------- Lights

fn env_radiance(dir: vec3f) -> vec3f {
    var L = vec3f(0.0);
    for (var i = 0u; i < u.light_count; i++) {
        if (u.lights[i].kind.x == KIND_ENV) {
            let l = u.lights[i];
            if (dir.y >= 0.0) {
                L += mix(l.b.xyz, l.a.xyz, sqrt(dir.y));
            } else {
                L += mix(l.b.xyz, l.c.xyz, sqrt(-dir.y));
            }
        }
    }
    return L;
}

fn environment_value(light:Light,dir:vec3f) -> vec3f {
    if (dir.y>=0.0) { return mix(light.b.xyz,light.a.xyz,sqrt(dir.y)); }
    return mix(light.b.xyz,light.c.xyz,sqrt(-dir.y));
}

fn environment_probability(light:Light,area_total:f32) -> f32 {
    let energy=max(max(luminance(light.a.xyz),luminance(light.b.xyz)),luminance(light.c.xyz));
    if (energy<=0.0) { return 0.0; }
    // Unbiased roulette saves dim sky shadow rays next to a powerful neon.
    return clamp(4.0*PI*energy/(4.0*PI*energy+area_total),0.05,1.0);
}

fn environment_emission(dir:vec3f,pdf:f32,use_mis:bool,n:vec3f,from_volume:bool,area_total:f32) -> vec3f {
    var radiance=vec3f(0.0);
    for (var i=0u;i<u.light_count;i++) {
        let light=u.lights[i]; if (light.kind.x!=KIND_ENV) {continue;}
        var weight=1.0;
        if (use_mis) {
            let proposal=environment_probability(light,area_total)*select(max(dot(n,dir),0.0)/PI,1.0/(4.0*PI),from_volume);
            weight=mis_power(pdf,proposal);
        }
        radiance+=environment_value(light,dir)*weight;
    }
    return radiance;
}

fn luminance(c: vec3f) -> f32 {
    return dot(c, vec3f(0.2126, 0.7152, 0.0722));
}

// Branchless orthonormal basis (Duff et al. 2017). Columns: tangent, bitangent, n.
fn tangent_frame(n: vec3f) -> mat3x3f {
    let s = select(-1.0, 1.0, n.z >= 0.0);
    let a = -1.0 / (s + n.z);
    let b = n.x * n.y * a;
    let t = vec3f(1.0 + s * n.x * n.x * a, s * b, -s * n.x);
    let bt = vec3f(b, s + n.y * n.y * a, -n.y);
    return mat3x3f(t, bt, n);
}

fn cosine_sample(n: vec3f, r1: f32, r2: f32) -> vec3f {
    let phi = 2.0 * PI * r1;
    let r = sqrt(r2);
    return tangent_frame(n) * vec3f(r * cos(phi), r * sin(phi), sqrt(max(0.0, 1.0 - r2)));
}

fn cone_sample(dir: vec3f, cos_max: f32, r1: f32, r2: f32) -> vec3f {
    let cos_t = 1.0 - r1 * (1.0 - cos_max);
    let sin_t = sqrt(max(0.0, 1.0 - cos_t * cos_t));
    let phi = 2.0 * PI * r2;
    return tangent_frame(dir) * vec3f(sin_t * cos(phi), sin_t * sin(phi), cos_t);
}

// ---------------------------------------------------------------- BRDF (Lambert + GGX)

fn ggx_d(nh: f32, a2: f32) -> f32 {
    let d = nh * nh * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d);
}

fn smith_g1(nv: f32, a2: f32) -> f32 {
    return 2.0 * nv / (nv + sqrt(a2 + (1.0 - a2) * nv * nv));
}

fn fresnel(f0: vec3f, vh: f32) -> vec3f {
    return f0 + (1.0 - f0) * pow(1.0 - vh, 5.0);
}

// Heitz (JCGT 2018): GGX visible-normal sampling avoids hidden microfacets
// and large path weights at grazing angles. PDF below uses the same VNDF.
fn sample_ggx_h(n: vec3f, v: vec3f, alpha: f32, r1: f32, r2: f32) -> vec3f {
    let frame = tangent_frame(n);
    let local_v = transpose(frame) * v;
    let vh = safe_normalize(vec3f(alpha * local_v.xy, max(local_v.z, 1e-5)));
    let lensq = dot(vh.xy, vh.xy);
    var t1 = vec3f(1.0, 0.0, 0.0);
    if (lensq > 0.0) { t1 = vec3f(-vh.y, vh.x, 0.0) / sqrt(lensq); }
    let t2 = cross(vh, t1);
    let r = sqrt(r1);
    let phi = 2.0 * PI * r2;
    let x = r * cos(phi);
    let blend = 0.5 * (1.0 + vh.z);
    let y = mix(sqrt(max(0.0, 1.0 - x * x)), r * sin(phi), blend);
    let nh = x * t1 + y * t2 + sqrt(max(0.0, 1.0 - x * x - y * y)) * vh;
    return frame * safe_normalize(vec3f(alpha * nh.xy, max(nh.z, 0.0)));
}

struct Surface {
    n: vec3f,
    diffuse: vec3f,
    f0: vec3f,
    alpha: f32,
    emission: vec3f,
    water: f32,
    eta: f32,
    transmission: f32,
    transmission_pdf: f32,
    diffuse_sigma: f32,
}

fn surface_value(slot: u32, n: vec3f, sample: DeSample) -> f32 {
    if (slot < 4u) { return sample.traps[slot]; }
    if (slot == COLOR_SLOT_NORMAL) { return n.y * 0.5 + 0.5; }
    return f32(sample.iterations) / max(f32(u.fractal.kind.y), 1.0);
}

fn make_surface(p: vec3f, n: vec3f, v: vec3f, sample: DeSample, footprint: f32) -> Surface {
    var albedo = u.material.albedo.xyz;
    let slot = u.coloring.kind_count.x;
    let assigned = sample.material > 0u && sample.material <= u.finish_count.x;
    if (!assigned && slot == COLOR_SLOT_ORBIT) {
        var value = dot(u.coloring.weights, sample.traps)
            + u.coloring.extra.x * f32(sample.iterations) / max(f32(u.fractal.kind.y), 1.0)
            + u.coloring.extra.y * (n.y*0.5+0.5);
        let gain = u.coloring.extra.z;
        if (gain > 0.0) { value = sign(value)*log(1.0+gain*abs(value))/log(1.0+gain); }
        albedo *= ramp_sample(value);
    } else if (!assigned && slot < 4u) {
        albedo *= ramp_sample(sample.traps[slot]);
    } else if (!assigned && slot == COLOR_SLOT_NORMAL) {
        albedo *= ramp_sample(n.y * 0.5 + 0.5);
    } else if (!assigned && slot == COLOR_SLOT_ITER) {
        albedo *= ramp_sample(f32(sample.iterations) / max(f32(u.fractal.kind.y), 1.0));
    }
    var rough = u.material.albedo.w;
    var metallic = u.material.params.x;
    var spec = u.material.params.y;
    var emission = vec3f(0.0);
    if (assigned) {
        let f = u.finishes[sample.material - 1u];
        albedo = f.albedo.xyz;
        rough = f.albedo.w;
        metallic = f.params.x;
        spec = f.params.y;
        emission = f.emission.xyz;
    }
    if (!assigned && u.surface.kind.y > 0u) {
        let t = clamp(surface_value(u.surface.kind.x, n, sample), 0.0, 1.0);
        rough = u.surface.finish.x + (u.surface.finish.y - u.surface.finish.x) * t;
        metallic = u.surface.finish.z + (u.surface.finish.w - u.surface.finish.z) * t;
    }
    var shading_n = n;
    if (assigned) {
        let f = u.finishes[sample.material-1u];
        if (f.detail_kind.x != 0u) {
            var detail = finish_eval(f,p,footprint);
            if (f.detail_kind.x == 9u) {
                // Settled powder gathers on tops and recedes on steep bevels.
                detail *= smoothstep(0.10,0.80,n.y);
            }
            if (f.detail_kind.x == 12u) {
                detail.w *= smoothstep(0.45,0.95,n.y);
            }
            let value = detail.w;
            let amount = f.detail_mapping.y * value;
            albedo = mix(albedo,f.detail_tint.xyz,amount);
            if (f.detail_kind.x == 9u) {
                rough += max(f.detail_mapping.w,0.0)*amount;
                metallic *= 1.0-amount;
                spec = mix(spec,0.025,amount);
            } else if (f.detail_kind.x == 12u) {
                rough = mix(rough,clamp(rough+f.detail_mapping.w,0.025,1.0),amount);
                spec = mix(spec,0.0204,amount);
                metallic *= 1.0-amount;
            } else {
                rough += f.detail_mapping.w * (2.0*value-1.0);
            }
            if (f.detail_kind.x == 2u) { metallic *= 1.0-0.75*amount; }
            if (abs(f.detail_mapping.z) > 0.0) { shading_n = finish_normal(f,n,detail.xyz); }
            if (f.detail_kind.x == 7u && ocean_enabled()) {
                let tide = u.seascape.settings.y + 0.22 + 0.32*finish_noise(p*0.8,f.detail_kind.y+311u).w;
                let wet = 1.0-smoothstep(tide,tide+0.55,p.y);
                albedo *= mix(1.0,0.36,wet);
                rough = mix(rough,0.18,wet);
                // Small colonies collect on dry upward-facing rock, not submerged faces.
                let colony = smoothstep(0.55,0.75,value)*smoothstep(0.35,0.85,n.y)*(1.0-wet)*0.2;
                albedo = mix(albedo,vec3f(0.09,0.12,0.055),colony);
            }
        }
    }
    let texture_amount = texture_mask(sample);
    if (!assigned && u.texture.effects.w > 0.0 && texture_amount > 0.0) {
        let tex_p = texture_parallax(p, n, v, footprint);
        let value = texture_filtered_value(tex_p, footprint);
        albedo *= mix(vec3f(1.0), texture_color(value), u.texture.effects.x * texture_amount);
        rough = mix(rough, value, u.texture.effects.y * texture_amount);
        if (u.texture.effects.z > 0.0) {
            shading_n = safe_normalize(mix(n, texture_normal(tex_p, n, footprint), texture_amount));
        }
    }
    rough = clamp(rough, 0.02, 1.0);
    var s: Surface;
    s.n = shading_n;
    s.diffuse = albedo * (1.0 - metallic);
    s.f0 = mix(vec3f(spec), albedo, metallic);
    s.alpha = rough * rough;
    s.emission = emission;
    s.water = 0.0;
    s.eta = 1.0;
    s.transmission = 0.0;
    s.transmission_pdf = 0.0;
    s.diffuse_sigma = 0.0;
    if (assigned && (u.finishes[sample.material-1u].detail_kind.x==10u || u.finishes[sample.material-1u].detail_kind.x==11u || u.finishes[sample.material-1u].detail_kind.x==12u)) {
        s.diffuse_sigma=rough*0.45;
    }
    return s;
}

// BRDF value without the cosine term. v points to the eye, l to the light.
fn brdf_eval(s: Surface, v: vec3f, l: vec3f) -> vec3f {
    let nl = dot(s.n, l);
    let nv = dot(s.n, v);
    if (nl <= 0.0 || nv <= 0.0) {
        return vec3f(0.0);
    }
    let h = normalize(v + l);
    let nh = max(dot(s.n, h), 0.0);
    let vh = max(dot(v, h), 0.0);
    let a2 = s.alpha * s.alpha;
    var f = fresnel(s.f0, vh);
    if (s.water > 0.0) { f = vec3f(ocean_fresnel(vh, s.eta)); }
    let spec = ggx_d(nh, a2) * smith_g1(nv, a2) * smith_g1(nl, a2) * f / max(4.0 * nv * nl, 1e-4);
    var diffuse_factor=1.0;
    if (s.diffuse_sigma>0.0) {
        // Oren-Nayar rough diffuse reflectance. The lobe remains cosine sampled;
        // its PDF is unchanged and the BRDF supplies the correct path weight.
        let sigma2=s.diffuse_sigma*s.diffuse_sigma;
        let a=1.0-sigma2/(2.0*(sigma2+0.33));
        let b=0.45*sigma2/(sigma2+0.09);
        let sin_l=sqrt(max(0.0,1.0-nl*nl));
        let sin_v=sqrt(max(0.0,1.0-nv*nv));
        let cos_phi=max(0.0,dot(l-s.n*nl,v-s.n*nv)/max(sin_l*sin_v,1e-8));
        diffuse_factor=a+b*cos_phi*sin_l*sin_v/max(max(nl,nv),1e-5);
    }
    return s.diffuse * (1.0 - f) * diffuse_factor / PI + spec;
}

fn mis_power(a: f32, b: f32) -> f32 {
    // Rescale before squaring: close/large emitters and grazing rays can
    // otherwise underflow or overflow even when both PDFs are finite.
    let scale = max(a, b);
    if (scale <= 0.0) { return 0.0; }
    let pa = a / scale;
    let pb = b / scale;
    return pa * pa / (pa * pa + pb * pb);
}

fn sun_pdf(cos_max: f32) -> f32 {
    return 1.0 / (2.0 * PI * max(1.0 - cos_max, 1e-7));
}

fn spec_probability(s: Surface, nv: f32) -> f32 {
    let f_avg = fresnel(s.f0, nv);
    return clamp(luminance(f_avg) / max(luminance(f_avg) + luminance(s.diffuse), 1e-4), 0.05, 0.95);
}

// Density of the lobe mixture used in `radiance` for direction l.
fn bsdf_pdf(s: Surface, v: vec3f, l: vec3f, p_spec: f32) -> f32 {
    let nl = dot(s.n, l);
    if (nl <= 0.0) { return 0.0; }
    let hh = normalize(v + l);
    let nh = max(dot(s.n, hh), 0.0);
    let a2 = s.alpha * s.alpha;
    let nv = max(dot(s.n, v), 1e-4);
    let pdf_spec = ggx_d(nh, a2) * smith_g1(nv, a2) / (4.0 * nv);
    return p_spec * pdf_spec + (1.0 - p_spec) * nl / PI;
}

// Radiance of every sun disk containing `dir` (zero otherwise).
fn sun_radiance(dir: vec3f, pdf_bsdf: f32, use_mis: bool) -> vec3f {
    var L = vec3f(0.0);
    for (var i = 0u; i < u.light_count; i++) {
        let light = u.lights[i];
        if (light.kind.x == KIND_SUN && dot(dir, light.a.xyz) >= light.a.w) {
            let pl = sun_pdf(light.a.w);
            var w = 1.0;
            if (use_mis) { w = mis_power(pdf_bsdf, pl); }
            L += light.color.xyz * pl * w;
        }
    }
    return L;
}

// Power/distance-weighted area-light selection keeps one shadow query per
// bounce. Division by selection probability preserves the lighting integral.
// Suns retain their disk sampling and MIS. `p` is offset off the surface.
fn direct_light(p: vec3f, s: Surface, geometric_n: vec3f, v: vec3f, p_spec: f32, area_total: f32, pixel_cone: f32, in_fog: bool, last: bool, cell: u32, strength: f32, rng: ptr<function, u32>) -> vec3f {
    var L = vec3f(0.0);
    var selected_area = NO_LIGHT;
    var selected_weight = 0.0;
    if (area_total > 0.0) {
        let threshold = rand(rng) * area_total;
        var sum = 0.0;
        for (var i = 0u; i < u.light_count; i++) {
            if (!is_emitter(u.lights[i])) { continue; }
            let weight = area_weight(p, s.n, u.lights[i]);
            sum += weight;
            if (weight > 0.0) {
                selected_area = i;
                selected_weight = weight;
                if (threshold < sum) { break; }
            }
        }
    }
    for (var i = 0u; i < u.light_count; i++) {
        let light = u.lights[i];
        if (light.kind.x == KIND_ENV) {
            if (max(max(max(light.a.x,light.a.y),light.a.z),max(max(max(light.b.x,light.b.y),light.b.z),max(max(light.c.x,light.c.y),light.c.z)))<=0.0) { continue; }
            let probability=environment_probability(light,area_total);
            if (rand(rng)>=probability) {continue;}
            let dir=cosine_sample(s.n,rand(rng),rand(rng));
            let nl=max(dot(s.n,dir),0.0);
            if (dot(geometric_n,dir)<=0.0 || nl<=0.0) { continue; }
            let visibility=scene_visibility_weight(p,dir,pixel_cone,ray_extent());
            if (visibility<=0.0) { continue; }
            var tr=visibility;
            if (in_fog) { tr*=fog_transmittance(p,dir,fog_extent(),rng); }
            let pl=probability*nl/PI;
            var pb=0.0;
            if (!last) { pb=continuation_pdf(s,geometric_n,v,dir,p_spec,cell,strength); }
            L+=brdf_eval(s,v,dir)*nl*environment_value(light,dir)*tr*select(mis_power(pl,pb),1.0,last)/pl*ocean_shadow_transmittance(p,dir,ray_extent());
        } else if (light.kind.x == KIND_HDRI) {
            if (light.color.x <= 0.0) { continue; }
            let dir = hdri_sample(light, rand(rng), rand(rng), rand(rng));
            let nl = dot(s.n, dir);
            if (nl <= 0.0 || dot(geometric_n, dir) <= 0.0) { continue; }
            let visibility = scene_visibility_weight(p, dir, pixel_cone, ray_extent());
            if (visibility <= 0.0) { continue; }
            var tr = visibility;
            if (in_fog) { tr *= fog_transmittance(p, dir, fog_extent(), rng); }
            let pl = hdri_pdf(light, dir);
            var pb = 0.0;
            if (!last) { pb = continuation_pdf(s, geometric_n, v, dir, p_spec, cell, strength); }
            L += brdf_eval(s, v, dir) * nl * hdri_lookup(light, dir) * tr * ocean_shadow_transmittance(p, dir, ray_extent())
                * select(mis_power(pl, pb), 1.0, last) / pl;
        } else if (light.kind.x == KIND_SUN) {
            // color = irradiance; disk radiance is color * sun_pdf, weighted against BSDF sampling.
            let dir = cone_sample(light.a.xyz, light.a.w, rand(rng), rand(rng));
            let nl = dot(s.n, dir);
            if (nl <= 0.0 || dot(geometric_n, dir) <= 0.0) { continue; }
            let visibility = scene_visibility_weight(p, dir, pixel_cone, ray_extent());
            if (visibility <= 0.0) { continue; }
            var tr = visibility;
            if (in_fog) { tr *= fog_transmittance(p, dir, fog_extent(), rng); }
            let pl = sun_pdf(light.a.w);
            var pb = 0.0;
            if (!last) { pb = continuation_pdf(s, geometric_n, v, dir, p_spec, cell, strength); }
            L += brdf_eval(s, v, dir) * nl * light.color.xyz * select(mis_power(pl, pb), 1.0, last) * tr * ocean_shadow_transmittance(p, dir, ray_extent());
        } else if (is_emitter(light)) {
            if (i != selected_area) { continue; }
            let sampled = sample_emitter(light, rng);
            let q = sampled.position;
            let ln = sampled.normal;
            let to_q = q - p;
            let dist2 = dot(to_q, to_q);
            if (dist2 < 1e-8) { continue; }
            let dist = sqrt(dist2);
            let dir = to_q / dist;
            let nl = dot(s.n, dir);
            let cos_l = dot(ln, -dir);
            if (nl <= 0.0 || cos_l <= 0.0 || dot(geometric_n, dir) <= 0.0) { continue; }
            let visibility = scene_visibility_weight(p, dir, pixel_cone, shadow_endpoint(dist));
            if (visibility <= 0.0) { continue; }
            var tr = visibility;
            if (in_fog) { tr *= fog_transmittance(p, dir, dist, rng); }
            let pl = (selected_weight / area_total) * area_direction_pdf(light, dist2, cos_l);
            var pb = 0.0;
            if (!last) { pb = continuation_pdf(s, geometric_n, v, dir, p_spec, cell, strength); }
            let w = select(mis_power(pl, pb), 1.0, last);
            L += brdf_eval(s, v, dir) * nl * emitter_radiance(light,-dir) * tr * w / pl * ocean_shadow_transmittance(p, dir, dist);
        }
    }
    return L;
}

// ---------------------------------------------------------------- Fog

fn fog_sigma(p: vec3f) -> f32 {
    let a = u.atmosphere.a;
    return a.x * exp(min(0.0,-(p.y-a.y)*a.z));
}

// Exact Beer-Lambert integral for the existing capped exponential height fog.
// Splitting at fog_height handles both directions and eliminates shadow noise.
fn fog_segment_tau(ro: vec3f,rd: vec3f,begin: f32,end: f32) -> f32 {
    let distance=max(end-begin,0.0);
    if (distance==0.0) { return 0.0; }
    let sigma=fog_sigma(ro+rd*begin);
    let mid_height=(ro+rd*((begin+end)*0.5)).y;
    if (mid_height<=u.atmosphere.a.y || u.atmosphere.a.z==0.0) { return u.atmosphere.a.x*distance; }
    let k=rd.y*u.atmosphere.a.z; let x=k*distance;
    if (abs(x)<0.001) { return sigma*distance*(1.0-x*0.5+x*x/6.0); }
    return max(0.0,(sigma-fog_sigma(ro+rd*end))/k);
}

fn fog_split(ro: vec3f,rd: vec3f,distance: f32) -> f32 {
    if (abs(rd.y)<1e-8) { return distance; }
    return clamp((u.atmosphere.a.y-ro.y)/rd.y,0.0,distance);
}

fn fog_optical_depth(ro: vec3f,rd: vec3f,distance: f32) -> f32 {
    let split=fog_split(ro,rd,distance);
    return fog_segment_tau(ro,rd,0.0,split)+fog_segment_tau(ro,rd,split,distance);
}

fn fog_transmittance(ro: vec3f, rd: vec3f, dist: f32, rng: ptr<function, u32>) -> f32 {
    return exp(-fog_optical_depth(ro,rd,dist));
}

fn fog_invert_segment(ro:vec3f,rd:vec3f,begin:f32,end:f32,tau:f32) -> f32 {
    let sigma=fog_sigma(ro+rd*begin); let k=rd.y*u.atmosphere.a.z;
    if ((ro+rd*((begin+end)*0.5)).y<=u.atmosphere.a.y || abs(k*(end-begin))<0.0001) {
        return min(end,begin+tau/max(sigma,1e-30));
    }
    if (sigma>1e-20) { return clamp(begin-log(max(1e-7,1.0-tau*k/sigma))/k,begin,end); }
    let remaining=max(0.0,fog_segment_tau(ro,rd,begin,end)-tau);
    let end_sigma=fog_sigma(ro+rd*end);
    return clamp(end+log(max(1e-7,1.0+remaining*k/max(end_sigma,1e-30)))/(-k),begin,end);
}

// Inverse optical depth: exact collision distribution, without null events.
fn fog_free_flight(ro: vec3f, rd: vec3f, t_end: f32, rng: ptr<function, u32>) -> f32 {
    if (u.atmosphere.a.x<=0.0) { return t_end; }
    let tau=-log(max(1.0-rand(rng),1e-7));
    let split=fog_split(ro,rd,t_end);
    let first=fog_segment_tau(ro,rd,0.0,split);
    let total=first+fog_segment_tau(ro,rd,split,t_end);
    if (tau>=total) { return t_end; }
    if (tau<first) { return fog_invert_segment(ro,rd,0.0,split,tau); }
    return fog_invert_segment(ro,rd,split,t_end,tau-first);
}

// Direct volume lighting is a line integral, independent of whether the
// continuation path happens to scatter. Combine equiangular sampling about
// the sampled emitter point with a truncated optical-depth distribution.
// This avoids rare, extremely bright chromatic collisions near small lights.
fn fog_segment_weight(ro: vec3f, rd: vec3f, end: f32, light: Light) -> f32 {
    let center=select(light.a.xyz,(light.a.xyz+light.b.xyz)*0.5,light.kind.x==KIND_TUBE);
    var interval=vec2f(0.0,end);
    if (is_projector(light)) {
        interval=cone_segment(ro,rd,end,projector_bound_apex(light),area_normal(light),light.b.w);
        if (interval.y<=interval.x) { return 0.0; }
    }
    let along=clamp(dot(center-ro,rd),interval.x,interval.y);
    let delta=center-(ro+rd*along);
    let area=emitter_area(light);
    // Max-channel power gives saturated blue the same opportunity as red.
    return max(max(light.color.x,light.color.y),light.color.z)*area/(dot(delta,delta)+area);
}

fn fog_segment_direct(ro: vec3f, rd: vec3f, end: f32, pixel_cone: f32, rng: ptr<function,u32>, point_index: u32, point_stream: u32, primary: bool) -> vec3f {
    if (end<=0.0 || max(max(u.atmosphere.b.x,u.atmosphere.b.y),u.atmosphere.b.z)<=0.0) { return vec3f(0.0); }
    let tau_end=fog_optical_depth(ro,rd,end);
    if (tau_end<=0.0) { return vec3f(0.0); }
    var total=0.0;
    for (var i=0u;i<u.light_count;i++) {
        if (is_emitter(u.lights[i])) { total+=fog_segment_weight(ro,rd,end,u.lights[i]); }
    }
    var result=vec3f(0.0);
    for (var i=0u;i<u.light_count;i++) {
        let light=u.lights[i];
        if (!is_emitter(light)) { continue; }
        let probability=volume_light_probability(fog_segment_weight(ro,rd,end,light),total);
        let stream=pcg(point_stream ^ (i*0x85ebca6bu));
        var selection=rand(rng);
        if (primary) { selection=volume_coordinate(point_index,5u,stream); }
        if (probability<=0.0 || selection>=probability) { continue; }
        var emitter=sample_emitter(light,rng);
        if (primary) {
            emitter=sample_emitter_point(light,vec3f(volume_coordinate(point_index,0u,stream),
                volume_coordinate(point_index,1u,stream),volume_coordinate(point_index,2u,stream)));
        }
        var interval=vec2f(0.0,end);
        if (is_projector(light)) {
            interval=cone_segment(ro,rd,end,emitter.position,area_normal(light),light.b.w);
            if (interval.y<=interval.x) { continue; }
        }
        let tau_begin=fog_optical_depth(ro,rd,interval.x);
        let tau_finish=fog_optical_depth(ro,rd,interval.y);
        let tau_span=max(0.0,tau_finish-tau_begin);
        if (tau_span<=0.0) { continue; }
        var conditional_mass=1.0-exp(-tau_span);
        if (tau_span<0.001) { conditional_mass=tau_span*(1.0-tau_span*0.5+tau_span*tau_span/6.0); }
        let interval_mass=exp(-tau_begin)*conditional_mass;
        if (interval_mass<1e-30) { continue; }
        let delta=emitter.position-ro;
        let projection=dot(delta,rd);
        let perpendicular=length(delta-rd*projection);
        let theta_a=atan2(interval.x-projection,max(perpendicular,1e-6));
        let theta_b=atan2(interval.y-projection,max(perpendicular,1e-6));
        let span=theta_b-theta_a;
        let regular=perpendicular>1e-4 && span>1e-5;
        var xi=rand(rng);
        var technique=rand(rng);
        if (primary) {
            xi=volume_coordinate(point_index,3u,stream);
            technique=volume_coordinate(point_index,4u,stream);
        }
        var t=mix(interval.x,interval.y,xi);
        if (technique<0.5) {
            if (regular) { t=clamp(projection+perpendicular*tan(mix(theta_a,theta_b,xi)),interval.x,interval.y); }
        } else {
            let tau=tau_begin-log(max(1.0-xi*conditional_mass,1e-7));
            let split=fog_split(ro,rd,interval.y);
            let first=fog_segment_tau(ro,rd,0.0,split);
            if (tau<first) { t=fog_invert_segment(ro,rd,0.0,split,tau); }
            else { t=fog_invert_segment(ro,rd,split,interval.y,tau-first); }
        }
        let p=ro+rd*t;
        let transmittance=exp(-fog_optical_depth(ro,rd,t));
        let sigma=fog_sigma(p);
        var angular_pdf=1.0/(interval.y-interval.x);
        if (regular) { angular_pdf=perpendicular/(span*(perpendicular*perpendicular+(t-projection)*(t-projection))); }
        let distance_pdf=0.5*(angular_pdf+sigma*transmittance/interval_mass);
        let to_light=emitter.position-p;
        let distance2=dot(to_light,to_light);
        if (distance2<1e-8) { continue; }
        let distance=sqrt(distance2);
        let wi=to_light/distance;
        let cosine=dot(emitter.normal,-wi);
        if (cosine<=0.0) { continue; }
        let visibility=scene_visibility_weight(p,wi,pixel_cone,shadow_endpoint(distance));
        if (visibility<=0.0) { continue; }
        let tr=transmittance*exp(-fog_optical_depth(p,wi,distance));
        let volume=nebula_segment(ro,rd,t,pixel_cone);
        result+=emitter_radiance(light,-wi)*(sigma*tr*hg_phase(dot(rd,wi),u.atmosphere.a.w)*visibility*volume.w
            / (probability*distance_pdf*area_direction_pdf(light,distance2,cosine)))
            *ocean_shadow_transmittance(p,wi,distance);
    }
    return result*u.atmosphere.b.xyz;
}

fn hg_phase(cos_theta: f32, g: f32) -> f32 {
    let d = 1.0 + g * g - 2.0 * g * cos_theta;
    return (1.0 - g * g) / (4.0 * PI * d * sqrt(d));
}

fn hg_sample(dir: vec3f, g: f32, r1: f32, r2: f32) -> vec3f {
    var cos_t = 1.0 - 2.0 * r1;
    if (abs(g) > 1e-3) {
        let sq = (1.0 - g * g) / (1.0 - g + 2.0 * g * r1);
        cos_t = (1.0 + g * g - sq * sq) / (2.0 * g);
    }
    let sin_t = sqrt(max(0.0, 1.0 - cos_t * cos_t));
    let phi = 2.0 * PI * r2;
    return tangent_frame(dir) * vec3f(sin_t * cos(phi), sin_t * sin(phi), cos_t);
}

// Direct light at a scattering event in the fog (no surface, phase instead of BRDF).
fn fog_direct_light(p: vec3f, rd: vec3f, area_total: f32, pixel_cone: f32, last: bool, rng: ptr<function, u32>) -> vec3f {
    let g = u.atmosphere.a.w;
    var L = vec3f(0.0);
    for (var i = 0u; i < u.light_count; i++) {
        let light = u.lights[i];
        if (light.kind.x == KIND_ENV) {
            if (max(max(max(light.a.x,light.a.y),light.a.z),max(max(max(light.b.x,light.b.y),light.b.z),max(max(light.c.x,light.c.y),light.c.z)))<=0.0) { continue; }
            let probability=environment_probability(light,area_total);
            if (rand(rng)>=probability) {continue;}
            let z=1.0-2.0*rand(rng); let phi=2.0*PI*rand(rng);
            let radius=sqrt(max(0.0,1.0-z*z));
            let dir=vec3f(radius*cos(phi),z,radius*sin(phi));
            let visibility=scene_visibility_weight(p,dir,pixel_cone,ray_extent());
            if (visibility<=0.0) { continue; }
            let tr=visibility*fog_transmittance(p,dir,fog_extent(),rng);
            let ph=hg_phase(dot(rd,dir),g); let pl=probability/(4.0*PI);
            L+=environment_value(light,dir)*ph*tr*select(mis_power(pl,ph),1.0,last)/pl*ocean_shadow_transmittance(p,dir,ray_extent());
        } else if (light.kind.x == KIND_HDRI) {
            if (light.color.x <= 0.0) { continue; }
            let dir = hdri_sample(light, rand(rng), rand(rng), rand(rng));
            let visibility = scene_visibility_weight(p, dir, pixel_cone, ray_extent());
            if (visibility <= 0.0) { continue; }
            let tr = visibility * fog_transmittance(p, dir, fog_extent(), rng);
            let ph = hg_phase(dot(rd, dir), g);
            let pl = hdri_pdf(light, dir);
            L += hdri_lookup(light, dir) * ph * tr * ocean_shadow_transmittance(p, dir, ray_extent())
                * select(mis_power(pl, ph), 1.0, last) / pl;
        } else if (light.kind.x == KIND_SUN) {
            let dir = cone_sample(light.a.xyz, light.a.w, rand(rng), rand(rng));
            let visibility = scene_visibility_weight(p, dir, pixel_cone, ray_extent());
            if (visibility <= 0.0) { continue; }
            let tr = visibility * fog_transmittance(p, dir, fog_extent(), rng);
            let ph = hg_phase(dot(rd, dir), g);
            let pl = sun_pdf(light.a.w);
            L += light.color.xyz * ph * tr * select(mis_power(pl, ph), 1.0, last) * ocean_shadow_transmittance(p, dir, ray_extent());
        }
    }
    return L;
}

// ---------------------------------------------------------------- Camera & integrator

struct Ray {
    o: vec3f,
    d: vec3f,
}

fn camera_ray(px: vec2u, rng: ptr<function, u32>) -> Ray {
    let cam = u.camera;
    let jitter = vec2f(rand(rng), rand(rng));
    let ndc = ((vec2f(px) + jitter) / vec2f(u.image_size)) * 2.0 - 1.0;
    let tan_half = cam.params.x;
    let aspect = cam.params.y;
    var dir = normalize(cam.forward.xyz
        + cam.right.xyz * (ndc.x * tan_half)
        + cam.up.xyz * (-ndc.y * tan_half / aspect));
    var origin = cam.origin.xyz;
    let lens_r = cam.params.z;
    if (lens_r > 0.0) {
        let focus = origin + dir * (cam.params.w / max(dot(dir, cam.forward.xyz), 1e-4));
        let r = lens_r * sqrt(rand(rng));
        let phi = 2.0 * PI * rand(rng);
        origin = origin + cam.right.xyz * (r * cos(phi)) + cam.up.xyz * (r * sin(phi));
        dir = normalize(focus - origin);
    }
    return Ray(origin, dir);
}

fn radiance(px: vec2u, rng: ptr<function, u32>, first_depth: ptr<function, f32>, first_aux: ptr<function, vec4f>, training_slot: u32, volume_out: ptr<function, vec3f>, direct_out: ptr<function, vec3f>, point_index: u32) -> vec3f {
    // One uniformly selected path vertex supplies a training observation.
    // Avoid a per-ray 32-vertex stack on GPUs with a small register budget.
    var training_rng = 0u;
    if (GUIDE_TRAINING) { training_rng = pcg(*rng ^ 0x6a09e667u); }
    var training_seen = 0u;
    var training = GuideRecord(false, vec3f(1.0), vec3f(0.0));
    let pixel_cone = u.camera.params.x * 2.0 / f32(u.image_size.x);
    var ray = camera_ray(px, rng);
    // Transport a texture footprint through the path; rough reflections see
    // a broader footprint and should not resolve subpixel relief or parallax.
    var texture_width = 0.0;
    var texture_spread = pixel_cone * 0.5;
    var throughput = vec3f(1.0);
    var L = vec3f(0.0);
    // Exact additive path partition. Primary in-scattering and paths whose
    // first real vertex is in the fog belong to the volume. Paths first hitting
    // a surface retain their indirect volume lighting in the surface layer.
    var primary_volume = vec3f(0.0);
    var volume_path = false;
    let point_stream=pcg(pcg(px.x^u.seed)^pcg(px.y^0x27d4eb2du));
    var prev_pdf = 0.0;        // pdf of the direction that produced this segment
    var prev_from_bsdf = false;
    var prev_normal = vec3f(0.0);
    var prev_area_total = 0.0;
    var prev_from_volume = false;
    var prev_volume_nee = false;
    var underwater = ocean_inside(ray.o);
    var solid_medium = false;
    var medium_slot = 0u;
    if (OPTICS_ENABLED) {
        let initial = de_sample_exact(ray.o);
        let optics = material_optics(initial.material);
        solid_medium = initial.distance < 0.0 && optics.absorption.w > 0.0
            && optics.surface.y > 0.0 && optics.surface.w == 0.0;
        medium_slot = initial.material;
    }
    var null_surfaces = 0u;
    for (var bounce = 0u; bounce <= u.max_bounces;) {
        var h = trace_scene(ray.o, ray.d, pixel_cone, ray_extent());
        h = optical_refine_hit(ray.o,ray.d,h);
        if (OPTICS_ENABLED && solid_medium) {
            let medium = material_optics(medium_slot);
            var distance = h.t;
            if (medium.subsurface.w > 0.0) {
                distance = min(h.t,-log(max(1e-7,1.0-rand(rng)))/medium.subsurface.w);
            }
            throughput *= exp(-medium.absorption.xyz*distance);
            if (distance < h.t) {
                if (bounce == u.max_bounces) { break; }
                throughput *= medium.subsurface.xyz;
                ray = Ray(ray.o + ray.d*distance,hg_sample(ray.d,0.0,rand(rng),rand(rng)));
                prev_from_bsdf = false;
                prev_from_volume = true;
                prev_volume_nee = false;
                prev_pdf = 0.0;
                texture_spread = max(texture_spread,0.25);
                bounce++;
                continue;
            }
        }
        var gate_depth = 0.0;
        let gate_t = portal_intersection(ray.o, ray.d, h.t);
        if (gate_t < h.t) {
            // The local medium ends at the aperture. Sample that segment
            // before adding membrane emission or transporting the ray; this
            // keeps camera and next-event paths consistent through fog.
            if (u.atmosphere.a.x > 0.0 && !underwater && !solid_medium) {
                let direct_volume = throughput*fog_segment_direct(ray.o,ray.d,gate_t,pixel_cone,rng,point_index,point_stream,bounce==0u && null_surfaces==0u);
                L += direct_volume;
                if (bounce == 0u) { primary_volume += direct_volume; }
                let t_s = fog_free_flight(ray.o, ray.d, gate_t, rng);
                if (t_s < gate_t) {
                    if (bounce == 0u) { volume_path = true; }
                    let p_s = ray.o + ray.d * t_s;
                    let volume = nebula_segment(ray.o, ray.d, t_s, pixel_cone);
                    L += throughput * volume.xyz;
                    throughput *= volume.w;
                    texture_width += t_s * texture_spread;
                    texture_spread = max(texture_spread, 0.25);
                    if (bounce == 0u) { *first_depth = gate_t; }
                    let volume_area_total = area_total_weight(p_s, vec3f(0.0));
                    L += throughput * u.atmosphere.b.xyz * fog_direct_light(p_s, ray.d,
                        volume_area_total, pixel_cone, bounce == u.max_bounces, rng);
                    throughput *= u.atmosphere.b.xyz;
                    if (bounce == u.max_bounces) { break; }
                    let direction = hg_sample(ray.d, u.atmosphere.a.w, rand(rng), rand(rng));
                    prev_pdf = hg_phase(dot(ray.d, direction), u.atmosphere.a.w);
                    prev_from_bsdf = true;
                    prev_from_volume = true;
                    prev_volume_nee = true;
                    prev_normal = vec3f(0.0);
                    prev_area_total = volume_area_total;
                    ray = Ray(p_s, direction);
                    bounce++;
                    continue;
                }
            }
            gate_depth = gate_t;
            if (!underwater && !solid_medium) {
                let volume = nebula_segment(ray.o, ray.d, gate_t, pixel_cone);
                L += throughput * volume.xyz;
                throughput *= volume.w;
            }
            let gate_point = ray.o + ray.d * gate_t;
            let membrane = portal_membrane(gate_point, texture_width + gate_t * texture_spread);
            var weight = 1.0;
            if (prev_from_bsdf && !prev_from_volume) {
                weight = mis_power(prev_pdf, portal_pdf(ray.o, gate_point));
            }
            L += throughput * membrane.xyz * weight;
            throughput *= membrane.w;
            let entry = gate_point - u.atmosphere.portal.xyz;
            let scale = u.atmosphere.portal_world.x;
            ray.o = WORLD_OFFSET + vec3f(entry.x * scale,
                u.atmosphere.portal_world.y + entry.y * scale, -0.01);
            texture_width = (texture_width + gate_t * texture_spread) * scale;
            // No main-world NEE technique samples paths through this aperture.
            prev_from_bsdf = false;
            prev_pdf = 0.0;
            h = otherworld_trace(ray.o, ray.d, pixel_cone, 2500.0);
        }
        let remote = in_otherworld(ray.o);
        let in_fog = u.atmosphere.a.x > 0.0 && !underwater && !remote && !solid_medium;
        if (remote && h.hit) {
            // Aerial perspective in the remote vista: homogeneous extinction
            // with a sky-coloured source approximation, not a second volume sampler.
            let transmittance = exp(-h.t * 0.0012);
            L += throughput * otherworld_sky(ray.d) * (1.0 - transmittance);
            throughput *= transmittance;
        }
        var geometric_n = vec3f(0.0);
        if (h.hit) {
            if (remote) {
                geometric_n = otherworld_normal(ray.o + ray.d * h.t);
            } else if (h.light_index != NO_LIGHT) {
                geometric_n = area_normal(u.lights[h.light_index]);
            } else if (h.ground) {
                geometric_n = ground_normal(ray.o + ray.d * h.t, ray.d);
            } else if (h.ocean_kind != 0u) {
                geometric_n = vec3f(0.0, 1.0, 0.0);
                if (h.ocean_kind == OCEAN_INTERFACE) {
                    geometric_n = ocean_normal((ray.o + ray.d * h.t).xz, 0.0, false)
                        * select(1.0, -1.0, underwater);
                }
            } else {
                geometric_n = normal_at(ray.o + ray.d * h.t,
                    max(h.t * pixel_cone * 0.5, 1e-4) * u.fractal.p0.x);
                if (OPTICS_ENABLED && material_optics(h.sample.material).absorption.w > 0.0) {
                    geometric_n = normal_at(ray.o+ray.d*h.t,1e-4);
                }
            }
            if (bounce == 0u) {
                // Guide export filtering with the underlying geometry. A rare
                // first fog sample must not classify a water pixel as sky.
                *first_depth = h.t;
                *first_aux = vec4f(geometric_n, f32(h.steps));
            }
        }
        if (underwater) {
            let tr = ocean_transmittance(h.t);
            L += throughput * ocean_inscatter(tr);
            throughput *= tr;
        }
        if (in_fog) {
            var t_end = h.t;
            if (!h.hit) { t_end = fog_extent(); }
            let direct_volume = throughput*fog_segment_direct(ray.o,ray.d,t_end,pixel_cone,rng,point_index,point_stream,bounce==0u && null_surfaces==0u);
            L += direct_volume;
            if (bounce == 0u) { primary_volume += direct_volume; }
            let t_s = fog_free_flight(ray.o, ray.d, t_end, rng);
            if (t_s < t_end) {
                if (bounce == 0u) { volume_path = true; }
                let p_s = ray.o + ray.d * t_s;
                let volume = nebula_segment(ray.o, ray.d, t_s, pixel_cone);
                L += throughput * volume.xyz;
                throughput *= volume.w;
                texture_width += t_s * texture_spread;
                texture_spread = max(texture_spread, 0.25);
                if (bounce == 0u && !h.hit) { *first_depth = t_s; }
                let volume_area_total = area_total_weight(p_s, vec3f(0.0));
                L += throughput * u.atmosphere.b.xyz * fog_direct_light(p_s, ray.d, volume_area_total, pixel_cone, bounce == u.max_bounces, rng);
                throughput *= u.atmosphere.b.xyz;
                if (bounce == u.max_bounces) { break; }
                let new_dir = hg_sample(ray.d, u.atmosphere.a.w, rand(rng), rand(rng));
                prev_pdf = hg_phase(dot(ray.d, new_dir), u.atmosphere.a.w);
                prev_from_bsdf = true;
                prev_from_volume = true;
                prev_volume_nee = true;
                prev_normal = vec3f(0.0);
                prev_area_total = volume_area_total;
                ray = Ray(p_s, new_dir);
                bounce++;
                continue;
            }
        }
        if (!underwater && !remote && !solid_medium) {
            let volume = nebula_segment(ray.o, ray.d, h.t, pixel_cone);
            L += throughput * volume.xyz;
            throughput *= volume.w;
        }
        if (!h.hit) {
            if (remote) {
                L += throughput * otherworld_sky(ray.d);
                if (bounce == 0u && gate_depth > 0.0) { *first_depth = gate_depth + 2500.0; }
                break;
            }
            // A ray remaining in an infinite water medium cannot see the sky
            // through a missed, nearly horizontal interface.
            if (underwater) { break; }
            if (!h.exhausted) {
                if (u.seascape.settings.x > 0.0 && !ocean_enabled()) {
                    L += throughput * coast_radiance(ray.o, ray.d, pixel_cone, prev_pdf, prev_from_bsdf);
                    break;
                }
                if (ocean_enabled()) { L += throughput * coast_sky(ray.d); }
                else { L += throughput * environment_emission(ray.d,prev_pdf,prev_from_bsdf,prev_normal,prev_from_volume,prev_area_total); }
                L += throughput * hdri_emission(ray.d, prev_pdf, prev_from_bsdf);
                L += throughput * sun_radiance(ray.d, prev_pdf, prev_from_bsdf);
                L += throughput * nebula_stars(ray.d, pixel_cone);
            }
            break;
        }
        let p = ray.o + ray.d * h.t;
        if (h.light_index != NO_LIGHT) {
            if (bounce == 0u) {
                *first_depth = h.t;
                *first_aux = vec4f(emitter_normal(u.lights[h.light_index],p), f32(h.steps));
            }
            // A fog segment already estimated all emitter paths with its line
            // integral. Phase-hit emission would count that same energy twice.
            if (!prev_volume_nee) {
                L += throughput * area_emission(ray.o, ray.d, h.t, prev_pdf,
                    prev_from_bsdf, prev_normal, prev_area_total, prev_from_volume);
            }
            break;
        }
        var n = geometric_n;
        let optics = material_optics(h.sample.material);
        let optical = OPTICS_ENABLED && optics.absorption.w > 0.0 && h.ocean_kind == 0u && !remote;
        let entering = dot(ray.d,geometric_n) < 0.0;
        if (optical && !entering) { n = -n; }
        if (optical && optics.surface.x < 1.0 && rand(rng) >= optics.surface.x) {
            null_surfaces++;
            if (null_surfaces > 32u) { break; }
            let offset = max(h.t*pixel_cone*0.15*u.fractal.p0.x,1e-4);
            ray.o = p + ray.d*offset;
            if (bounce == 0u) { *first_depth = -1.0; *first_aux = vec4f(0.0,0.0,0.0,-1.0); }
            continue;
        }
        if (bounce == 0u) {
            *first_depth = h.t;
            if (remote && gate_depth > 0.0) { *first_depth = gate_depth + h.t / u.atmosphere.portal_world.x; }
            *first_aux = vec4f(n, f32(h.steps));
            if (h.ocean_kind == 0u && !h.ground && !remote) {
                throughput *= 1.0 - u.atmosphere.b.w * smoothstep(0.0, 1.0, f32(h.steps) / 96.0);
            }
        }
        let v = -ray.d;
        texture_width += h.t * texture_spread;
        var s: Surface;
        if (remote) {
            s = otherworld_surface(p, n, max(texture_width, 1e-4));
        } else if (h.ocean_kind != 0u) {
            s = ocean_surface(p, n, v, max(texture_width, 1e-4), h.ocean_kind, underwater);
        } else {
            s = make_surface(p, n, v, h.sample, max(texture_width, 1e-4));
        }
        L += throughput * s.emission;
        var offset = max(h.t * pixel_cone * 2.0, 1e-3) * u.fractal.p0.x;
        if (h.ocean_kind != 0u || h.ground) { offset = 1e-3; }
        if (remote) { offset = 0.012; }
        if (optical && optics.surface.y > 0.0 && rand(rng) < optics.surface.y) {
            if (bounce == u.max_bounces) { break; }
            let sampled = optical_bounce(optics,n,ray.d,s.alpha,entering,rng);
            if (!sampled.valid) { break; }
            throughput *= sampled.weight;
            // Refraction uses a much smaller bias than opaque relief, so
            // thin glass does not lose its entire thickness at low resolution.
            let bias = max(1e-4,min(offset,0.001));
            ray = Ray(p + sampled.direction*bias,sampled.direction);
            if (sampled.transmitted && optics.surface.w == 0.0) {
                solid_medium = entering;
                medium_slot = h.sample.material;
            }
            prev_from_bsdf = false;
            prev_from_volume = false;
            prev_volume_nee = false;
            prev_pdf = 0.0;
            texture_spread = max(texture_spread,s.alpha*0.1);
            bounce++;
            continue;
        }
        let p_off = p + n * offset;
        let nv = max(dot(s.n, v), 1e-4);
        let p_spec = spec_probability(s, nv);
        let area_total = area_total_weight(p_off, s.n);
        var cell = 0u;
        if (s.water == 0.0 && GUIDE_SAMPLING && (u.guiding_flags & 2u) != 0u && bounce < u.max_bounces) { cell = guide_cell(p, n); }
        var strength = 0.0;
        if (s.water == 0.0 && bounce < u.max_bounces) { strength = guide_strength(cell, s, p_spec); }

        if (remote) {
            L += throughput * otherworld_direct(p_off, s, n, v, pixel_cone, rng);
        } else {
            L += throughput * direct_light(p_off, s, n, v, p_spec, area_total, pixel_cone, in_fog, bounce == u.max_bounces, cell, strength, rng);
            L += throughput * portal_direct(p_off, s, n, v, p_spec, pixel_cone,
                bounce == u.max_bounces, cell, strength, in_fog, rng);
        }

        if (bounce == u.max_bounces) {
            break;
        }
        if (s.transmission_pdf > 0.0 && rand(rng) < s.transmission_pdf) {
            let direction = refract(ray.d, n, s.eta);
            if (dot(direction, n) >= 0.0 || dot(direction, direction) < 0.5) { break; }
            throughput *= (s.transmission / s.transmission_pdf) * s.eta * s.eta;
            prev_from_bsdf = false; // Delta transmission has no NEE competitor.
            prev_from_volume = false;
            prev_volume_nee = false;
            prev_pdf = 0.0;
            ray = Ray(p - n * offset, direction);
            underwater = ocean_inside(ray.o);
            texture_spread *= s.eta;
            bounce++;
            continue;
        }
        // Sample the next direction: a mixture of GGX and cosine lobes.
        let r0 = rand(rng);
        let r1 = rand(rng);
        let r2 = rand(rng);
        var l: vec3f;
        if (strength > 0.0 && r0 < strength) {
            l = guide_sample(cell, n, r0 / strength, r1, r2);
        } else {
            var bsdf_choice = r0;
            if (strength > 0.0) { bsdf_choice = (r0 - strength) / (1.0 - strength); }
            if (bsdf_choice < p_spec) {
                let hh = sample_ggx_h(s.n, v, s.alpha, r1, r2);
                l = reflect(-v, hh);
            } else {
                l = cosine_sample(s.n, r1, r2);
            }
        }
        let nl = dot(s.n, l);
        if (nl <= 0.0 || dot(n, l) <= 0.0) {
            break;
        }
        let pdf = continuation_pdf(s, n, v, l, p_spec, cell, strength);
        if (pdf <= 1e-6) {
            break;
        }
        if (s.water == 0.0 && GUIDE_TRAINING && training_slot != NO_GUIDE_RECORD) {
            // A separate random stream keeps learning from perturbing image samples.
            training_seen += 1u;
            training_rng = pcg(training_rng);
            if (training_rng % training_seen == 0u) {
                guide_capture(training_slot, p, n, l, pdf, bsdf_pdf(s, v, l, p_spec));
                training = GuideRecord(true, throughput, L);
            }
        }
        throughput *= brdf_eval(s, v, l) * nl / pdf;
        prev_pdf = pdf;
        prev_from_bsdf = true;
        prev_normal = s.n;
        prev_area_total = area_total;
        prev_from_volume = false;
        prev_volume_nee = false;

        if (bounce >= 2u) {
            let q = clamp(max(throughput.x, max(throughput.y, throughput.z)), 0.05, 0.95);
            if (rand(rng) > q) {
                break;
            }
            throughput /= q;
        }
        ray = Ray(p_off, l);
        if (ocean_enabled()) { underwater = ocean_inside(ray.o); }
        texture_spread = max(texture_spread, s.alpha * 0.25);
        bounce++;
    }
    if (GUIDE_TRAINING) { guide_learn(training_slot, training, L); }
    *volume_out = select(primary_volume, L, volume_path);
    *direct_out = primary_volume;
    return L;
}

override PATH_WORKGROUP_X: u32 = 8u;
override PATH_WORKGROUP_Y: u32 = 8u;
@compute @workgroup_size(PATH_WORKGROUP_X, PATH_WORKGROUP_Y, 1)
fn main(@builtin(global_invocation_id) gid: vec3u) {
    if (gid.x >= u.tile_size.x || gid.y >= u.tile_size.y) {
        return;
    }
    let px = u.tile_origin + gid.xy;
    if (px.x >= u.image_size.x || px.y >= u.image_size.y) {
        return;
    }
    let idx = px.y * u.image_size.x + px.x;
    var first_depth = -1.0;
    var first_aux = vec4f(0.0, 0.0, 0.0, -1.0);
    for (var s = 0u; s < u.sample_count; s++) {
        var rng = rng_init(px, u.sample_start + s, u.seed);
        var d = -1.0;
        var a = vec4f(0.0, 0.0, 0.0, -1.0);
        let training_slot = guide_record_slot(idx, u.sample_start + s);
        var r: vec3f;
        var volume = vec3f(0.0);
        var direct_volume = vec3f(0.0);
        if ((u.flags & 1u) != 0u) {
            // Flat image: sample the colour field directly, without 3D transport.
            let jitter = vec2f(rand(&rng), rand(&rng));
            let uv = (vec2f(px)+jitter)/vec2f(u.image_size);
            let p = vec2f(uv.x-0.5,(0.5-uv.y)/u.camera.params.y)*(6.0/u.camera.params.x);
            let c = u.camera.origin.z; let sn = u.camera.origin.w;
            let q = u.camera.origin.xy + vec2f(c*p.x-sn*p.y,sn*p.x+c*p.y);
            r = texture_color(texture_value(vec3f(q,0.0)));
            d = 1.0; a = vec4f(0.0,0.0,1.0,0.0);
        } else {
            r = radiance(px, &rng, &d, &a, training_slot, &volume, &direct_volume, u.sample_start+s);
        }
        // One add per sample keeps the summation order independent of chunking.
        let ok = all(r == r) && all(abs(r) < vec3f(1e30));
        accum[idx] += vec4f(select(vec3f(0.0), r, ok), 1.0);
        let v = select(vec3f(0.0), volume, ok);
        let surface = select(vec3f(0.0), r-volume, ok);
        let old = volume_accum[idx];
        let old_mean = old.xyz/max(old.w,1.0);
        let delta = v-old_mean;
        let new_mean = old_mean+delta/(old.w+1.0);
        if ((u.flags & 4u) != 0u) {
        let component = volume_components[idx];
        let direct = select(vec3f(0.0), direct_volume, ok);
        let direct_delta = direct-component.direct.xyz/max(old.w,1.0);
        let direct_mean = (component.direct.xyz+direct)/(old.w+1.0);
        let indirect = v-direct;
        let indirect_delta = indirect-(old.xyz-component.direct.xyz)/max(old.w,1.0);
        let indirect_mean = (old.xyz+v-component.direct.xyz-direct)/(old.w+1.0);
        volume_components[idx].direct = component.direct+vec4f(direct,1.0);
        volume_components[idx].direct_moments = component.direct_moments+vec4f(max(vec3f(0.0),direct_delta*(direct-direct_mean)),1.0);
        volume_components[idx].indirect_moments = component.indirect_moments+vec4f(max(vec3f(0.0),indirect_delta*(indirect-indirect_mean)),1.0);
        }
        volume_moments[idx] += vec4f(max(vec3f(0.0),delta*(v-new_mean)),1.0);
        volume_accum[idx] = old+vec4f(v,1.0);
        surface_accum[idx] += vec4f(surface,1.0);
        if (s == 0u) {
            first_depth = d;
            first_aux = a;
        }
    }
    if (u.sample_start == 0u) {
        depth_buf[idx] = bitcast<u32>(first_depth);
        aux[idx] = first_aux;
    }
}
