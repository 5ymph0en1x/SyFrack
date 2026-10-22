// Rectangles participate in the same closest-hit and visibility queries as
// the implicit form. Their backs are opaque, but only their fronts emit.
const NO_LIGHT: u32 = 0xffffffffu;

fn is_emitter(light: Light) -> bool {
    return light.kind.x==KIND_AREA || light.kind.x==KIND_TUBE;
}

fn is_projector(light: Light) -> bool { return light.kind.x==KIND_AREA && light.kind.y!=0u; }

// C2 angular blending of radiance, shared by camera, reflection and NEE paths.
fn beam_profile(light: Light, direction: vec3f) -> f32 {
    if (!is_projector(light)) { return 1.0; }
    let cosine=dot(area_normal(light),direction);
    let t=clamp((cosine-light.b.w)/max(light.a.w-light.b.w,1e-7),0.0,1.0);
    return t*t*t*(10.0+t*(-15.0+6.0*t));
}

fn emitter_radiance(light: Light, direction: vec3f) -> vec3f {
    return light.color.xyz*beam_profile(light,direction);
}

// Exact intersection with the positive cone. Convexity gives one interval,
// even when the quadratic describes both the forward and backward cones.
fn cone_segment(ro: vec3f, rd: vec3f, end: f32, apex: vec3f, axis: vec3f, cosine: f32) -> vec2f {
    var interval=vec2f(0.0,end);
    let delta=ro-apex; let axial=dot(delta,axis); let slope=dot(rd,axis);
    if (abs(slope)<1e-8) {
        if (axial<0.0) { return vec2f(1.0,0.0); }
    } else {
        let crossing=-axial/slope;
        if (slope>0.0) { interval.x=max(interval.x,crossing); }
        else { interval.y=min(interval.y,crossing); }
    }
    if (interval.y<=interval.x) { return vec2f(1.0,0.0); }
    let c2=cosine*cosine;
    let s2=(1.0-cosine)*(1.0+cosine);
    let radial=delta-axis*axial;
    let radial_slope=rd-axis*slope;
    // Avoid subtracting nearly equal, large axial squared distances, notably
    // for narrow cones with a far virtual apex and nearly axial camera rays.
    let a=s2*slope*slope-c2*dot(radial_slope,radial_slope);
    let b=2.0*(s2*axial*slope-c2*dot(radial,radial_slope));
    let c=s2*axial*axial-c2*dot(radial,radial);
    if (abs(a)<1e-12) {
        if (abs(b)<1e-12) {
            if (c<0.0) { return vec2f(1.0,0.0); }
        } else if (b>0.0) { interval.x=max(interval.x,-c/b); }
        else { interval.y=min(interval.y,-c/b); }
        return interval;
    }
    let discriminant=b*b-4.0*a*c;
    if (discriminant<=0.0) {
        let middle=(interval.x+interval.y)*0.5;
        if ((a*middle+b)*middle+c<0.0) { return vec2f(1.0,0.0); }
        return interval;
    }
    let root=sqrt(discriminant);
    let q=-0.5*(b+select(-root,root,b>=0.0));
    let r0=q/a; let r1=c/q;
    let low=min(r0,r1); let high=max(r0,r1);
    if (a<0.0) { return vec2f(max(interval.x,low),min(interval.y,high)); }
    let before=vec2f(interval.x,min(interval.y,low));
    if (before.y>before.x) { return before; }
    return vec2f(max(interval.x,high),interval.y);
}

// Bounding cone of the entire emitting rectangle, not merely its centre.
// Moving its virtual apex backward by radius/tan(theta) is conservative.
fn projector_bound_apex(light: Light) -> vec3f {
    let radius=0.5*length(light.b.xyz+light.c.xyz);
    return light.a.xyz-area_normal(light)*(radius/light.c.w);
}

fn emitter_area(light: Light) -> f32 {
    if (light.kind.x==KIND_TUBE) {
        return 2.0*PI*light.a.w*(light.b.w+2.0*light.a.w);
    }
    return length(cross(light.b.xyz,light.c.xyz));
}

fn emitter_normal(light: Light, point: vec3f) -> vec3f {
    if (light.kind.x==KIND_TUBE) {
        let axis=light.b.xyz-light.a.xyz;
        let t=clamp(dot(point-light.a.xyz,axis)/dot(axis,axis),0.0,1.0);
        return safe_normalize(point-light.a.xyz-t*axis);
    }
    return area_normal(light);
}

// The cylindrical body and two spherical caps share one exact closest hit.
fn tube_intersection(ro: vec3f, rd: vec3f, light: Light) -> f32 {
    let axis=(light.b.xyz-light.a.xyz)/light.b.w; let oa=ro-light.a.xyz;
    let axial_ray=dot(axis,rd); let axial_origin=dot(axis,oa);
    let side=cross(axis,rd); let a=dot(side,side);
    var nearest=1e30;
    if (a>1e-10) {
        // Closest approach avoids subtracting nearly equal squared distances
        // when a centimetre-wide neon is viewed from hundreds of metres away.
        let perpendicular=dot(oa,side)/sqrt(a);
        let h=light.a.w*light.a.w-perpendicular*perpendicular;
        if (h<0.0) { return nearest; }
        let middle=-(dot(rd,oa)-axial_origin*axial_ray)/a;
        let root=sqrt(max(0.0,h)/a);
        for (var i=0u;i<2u;i++) {
            let t=middle+select(-root,root,i==1u);
            let y=axial_origin+t*axial_ray;
            if (h>=0.0 && t>1e-6 && y>=0.0 && y<=light.b.w) { nearest=min(nearest,t); }
        }
    }
    if (a<=1e-10) {
        let radial=oa-axis*axial_origin;
        if (dot(radial,radial)>light.a.w*light.a.w) { return nearest; }
    }
    for (var cap=0u;cap<2u;cap++) {
        let center=select(light.a.xyz,light.b.xyz,cap==1u);
        let oc=ro-center; let projection=dot(oc,rd);
        let closest=oc-rd*projection;
        let h=light.a.w*light.a.w-dot(closest,closest);
        if (h<0.0) { continue; }
        for (var i=0u;i<2u;i++) {
            let t=-projection+select(-sqrt(h),sqrt(h),i==1u);
            let y=dot(ro+rd*t-light.a.xyz,axis);
            let on_cap=select(y<=0.0,y>=light.b.w,cap==1u);
            if (t>1e-6 && on_cap) { nearest=min(nearest,t); }
        }
    }
    return nearest;
}

fn emitter_intersection(ro: vec3f, rd: vec3f, light: Light) -> f32 {
    if (light.kind.x==KIND_TUBE) { return tube_intersection(ro,rd,light); }
    return area_intersection(ro,rd,light);
}

struct EmitterPoint { position: vec3f, normal: vec3f }

fn sample_emitter(light: Light, rng: ptr<function,u32>) -> EmitterPoint {
    if (light.kind.x!=KIND_TUBE) {
        return sample_emitter_point(light,vec3f(rand(rng),rand(rng),0.0));
    }
    return sample_emitter_point(light,vec3f(rand(rng),rand(rng),rand(rng)));
}

// Explicit coordinates also support stratified point clouds of emitter samples.
fn sample_emitter_point(light: Light, uvw: vec3f) -> EmitterPoint {
    if (light.kind.x!=KIND_TUBE) {
        let q=light.a.xyz+(uvw.x-0.5)*light.b.xyz+(uvw.y-0.5)*light.c.xyz;
        return EmitterPoint(q,area_normal(light));
    }
    let axis=(light.b.xyz-light.a.xyz)/light.b.w;
    let frame=tangent_frame(axis);
    let choice=uvw.x; let polar=uvw.y; let phi=2.0*PI*uvw.z;
    let radial=frame[0]*cos(phi)+frame[1]*sin(phi);
    let cylinder_fraction=light.b.w/(light.b.w+2.0*light.a.w);
    if (choice<cylinder_fraction) {
        return EmitterPoint(mix(light.a.xyz,light.b.xyz,polar)+light.a.w*radial,radial);
    }
    let top=(choice-cylinder_fraction)/(1.0-cylinder_fraction)>=0.5;
    let n=radial*sqrt(max(0.0,1.0-polar*polar))+axis*polar*select(-1.0,1.0,top);
    return EmitterPoint(select(light.a.xyz,light.b.xyz,top)+light.a.w*n,n);
}

fn area_normal(light: Light) -> vec3f {
    return safe_normalize(cross(light.b.xyz, light.c.xyz));
}

// Returns a distance or a miss marker. Edge vectors are orthogonal and span
// the full width/height (the same convention used by area-light sampling).
fn area_intersection(ro: vec3f, rd: vec3f, light: Light) -> f32 {
    let n = area_normal(light);
    let denominator = dot(n, rd);
    if (abs(denominator) < 1e-7) { return 1e30; }
    let t = dot(light.a.xyz - ro, n) / denominator;
    if (t <= 1e-6) { return 1e30; }
    let q = ro + rd * t - light.a.xyz;
    let x = dot(q, light.b.xyz) / dot(light.b.xyz, light.b.xyz);
    let y = dot(q, light.c.xyz) / dot(light.c.xyz, light.c.xyz);
    if (abs(x) > 0.5 || abs(y) > 0.5) { return 1e30; }
    return t;
}

fn trace_geometry_lights(ro: vec3f, rd: vec3f, pixel_cone: f32, t_end: f32) -> Hit {
    var nearest = t_end;
    var light_index = NO_LIGHT;
    var ground = false;
    if (u.atmosphere.ground.x > 0.0) {
        let t = ground_intersection(ro, rd, pixel_cone, nearest);
        if (t > 1e-5 && t < nearest) { nearest = t; ground = true; }
    }
    for (var i = 0u; i < u.light_count; i++) {
        if (!is_emitter(u.lights[i])) { continue; }
        let t = emitter_intersection(ro, rd, u.lights[i]);
        if (t < nearest) {
            nearest = t;
            light_index = i;
            ground = false;
        }
    }
    var h = march_segment(ro, rd, pixel_cone, nearest);
    if (!h.hit && !h.exhausted && (light_index != NO_LIGHT || ground)) {
        h.hit = true;
        h.t = nearest;
        h.light_index = light_index;
        h.ground = ground;
        if (ground) { h.sample.material = u32(u.atmosphere.ground.z); }
    }
    return h;
}

fn trace_scene_straight(ro: vec3f, rd: vec3f, pixel_cone: f32, t_end: f32) -> Hit {
    if (in_otherworld(ro)) { return otherworld_trace(ro, rd, pixel_cone, t_end); }
    let water = ocean_intersection(ro, rd, t_end);
    var h = trace_geometry_lights(ro, rd, pixel_cone, water.t);
    if (!h.hit && !h.exhausted && water.kind != 0u) {
        h.hit = true;
        h.t = water.t;
        h.ocean_kind = water.kind;
    }
    return h;
}

fn trace_scene(ro:vec3f,rd:vec3f,pixel_cone:f32,t_end:f32)->Hit {
    if(!kerr_enabled()){return trace_scene_straight(ro,rd,pixel_cone,t_end);}
    var state=kerr_launch(ro,rd);var out:Hit;out.curved=true;out.light_index=NO_LIGHT;
    out.medium=vec4f(0.0,0.0,0.0,1.0);out.endpoint=ro;out.tangent=rd;
    out.sample=DeSample(T_MAX,vec4f(1.0),0u,0u,0u);
    for(var i=0u;i<u32(u.phenomena.lens_config.y);i++){
        out.steps=i+1u;let p=state.x.xyz;let radius=kerr_radius(p);
        if(radius<=kerr_horizon()*1.001){out.exhausted=true;return out;}
        let derivative=kerr_rhs(state);out.tangent=normalize(derivative.x.xyz);out.endpoint=p;
        if(out.t>=t_end||(radius>=u.phenomena.lens_config.z&&dot(p-u.phenomena.lens_center.xyz,out.tangent)>0.0)){return out;}
        var sample=scene_sample(p);let epsilon=max(1e-5,out.t*pixel_cone*u.fractal.p0.x);
        if(sample.flags!=0u&&abs(sample.distance)*u.fractal.p0.w<=epsilon*5.0){sample=scene_sample_exact(p);}
        if(sample.flags==0u&&abs(sample.distance)*u.fractal.p0.w<=epsilon){out.hit=true;out.sample=sample;return out;}
        let dt=kerr_step_size(state,min(abs(sample.distance)*u.fractal.p0.w,t_end-out.t));
        let next=kerr_advance(state,dt);let chord=next.x.xyz-p;let length_chord=length(chord);
        if(!(length_chord>0.0&&length_chord<1e10)){out.exhausted=true;return out;}
        let direction=chord/length_chord;var end=length_chord;
        if(u.atmosphere.ground.x>0.0){let t=ground_intersection(p,direction,pixel_cone,end);if(t>1e-5&&t<end){end=t;out.ground=true;}}
        for(var j=0u;j<u.light_count;j++){if(!is_emitter(u.lights[j])){continue;}let t=emitter_intersection(p,direction,u.lights[j]);if(t<end){end=t;out.ground=false;out.light_index=j;}}
        let endpoint=p+direction*end;let medium=nature_chord(p,endpoint);
        out.medium=vec4f(out.medium.xyz+out.medium.w*medium.xyz,out.medium.w*medium.w);out.t+=end;
        if(out.ground||out.light_index!=NO_LIGHT){out.hit=true;out.endpoint=endpoint;out.tangent=direction;if(out.ground){out.sample.material=u32(u.atmosphere.ground.z);}return out;}
        state=next;
    }
    out.exhausted=true;return out;
}

// Keep the sampled emitter out of its own shadow segment, allowing for f32
// roundoff without using the image-resolution-dependent geometry tolerance.
fn shadow_endpoint(distance: f32) -> f32 {
    return max(0.0, distance - max(1e-5, distance * 1e-6));
}

fn scene_visible(ro: vec3f, rd: vec3f, pixel_cone: f32, t_end: f32) -> bool {
    return scene_visibility_weight(ro,rd,pixel_cone,t_end)>0.0;
}

fn scene_visibility_weight(ro:vec3f,rd:vec3f,pixel_cone:f32,t_end:f32)->f32{
    return geometry_visibility_weight(ro,rd,pixel_cone,t_end)*nature_volume(ro,rd,t_end).w;
}
fn geometry_visibility_weight(ro: vec3f, rd: vec3f, pixel_cone: f32, t_end: f32) -> f32 {
    if (OPTICS_ENABLED) {
        var origin = ro; var remaining = t_end; var visibility = 1.0;
        for (var i=0u;i<32u;i++) {
            var h = trace_geometry_lights(origin,rd,pixel_cone,remaining);
            h = optical_refine_hit(origin,rd,h);
            if (h.exhausted) { return 0.0; }
            if (!h.hit) { return visibility; }
            if (h.light_index!=NO_LIGHT) { return 0.0; }
            let o = material_optics(h.sample.material);
            if (o.absorption.w==0.0) { return 0.0; }
            // Coverage shadows have an analytic expectation. Solid dielectric
            // paths require refraction and are handled by BSDF continuation.
            visibility *= 1.0-o.surface.x;
            if (visibility<1e-6) { return 0.0; }
            let distance = h.t+1e-4;
            origin += rd*distance; remaining -= distance;
            if (remaining<=0.0) { return visibility; }
        }
        return 0.0;
    }
    let h = trace_geometry_lights(ro, rd, pixel_cone, t_end);
    return select(0.0,1.0,!h.hit && !h.exhausted);
}

fn area_weight(p: vec3f, n: vec3f, light: Light) -> f32 {
    let area = emitter_area(light);
    let center=select(light.a.xyz,(light.a.xyz+light.b.xyz)*0.5,light.kind.x==KIND_TUBE);
    let to_light = center - p;
    let dist2 = max(dot(to_light, to_light), 1e-6);
    let dir = to_light / sqrt(dist2);
    var facing = 0.25;
    if (light.kind.x!=KIND_TUBE) { facing=max(dot(area_normal(light),-dir),0.0); }
    if (is_projector(light)) {
        let axis=area_normal(light); let delta=p-projector_bound_apex(light);
        let axial=dot(delta,axis);
        if (axial<0.0 || axial*axial<light.b.w*light.b.w*dot(delta,delta)) { return 0.0; }
        facing*=max(0.025,beam_profile(light,-dir));
    }
    return luminance(light.color.xyz) * area * facing
        * max(dot(n, dir), 0.15) / dist2;
}

fn area_total_weight(p: vec3f, n: vec3f) -> f32 {
    var total = 0.0;
    for (var i = 0u; i < u.light_count; i++) {
        if (is_emitter(u.lights[i])) {
            total += area_weight(p, n, u.lights[i]);
        }
    }
    return total;
}

// Independent per-emitter roulette for a volumetric segment, with at most
// three shadow samples in expectation. Probability compensation preserves
// energy when several colours contribute to the same segment.
fn volume_light_probability(weight: f32, total: f32) -> f32 {
    if (total<=0.0) { return 0.0; }
    return min(1.0,3.0*weight/total);
}

// Uniform area density converted to solid-angle density at the path vertex.
fn area_direction_pdf(light: Light, distance_squared: f32, cos_light: f32) -> f32 {
    let area = emitter_area(light);
    return distance_squared / max(area * cos_light, 1e-20);
}

fn area_emission(ro: vec3f, rd: vec3f, distance: f32, previous_pdf: f32,
    use_mis: bool, previous_normal: vec3f, previous_total: f32, from_volume: bool) -> vec3f {
    var L = vec3f(0.0);
    for (var i = 0u; i < u.light_count; i++) {
        let light = u.lights[i];
        if (!is_emitter(light)) { continue; }
        let cos_l = dot(emitter_normal(light,ro+rd*distance), -rd);
        if (cos_l <= 0.0) { continue; }
        let t = emitter_intersection(ro, rd, light);
        // Sum coincident emitters too: splitting a softbox into identical
        // lights must preserve its radiance and its light-selection PMFs.
        if (abs(t - distance) > max(1e-6, distance * 1e-6)) { continue; }
        var w = 1.0;
        if (use_mis) {
            var pl = area_direction_pdf(light, distance * distance, cos_l);
            var selection = 0.0;
            if (previous_total > 0.0) {
                let proposal_normal = select(previous_normal, vec3f(0.0), from_volume);
                selection = area_weight(ro, proposal_normal, light) / previous_total;
                if (from_volume) { selection = volume_light_probability(area_weight(ro,vec3f(0.0),light),previous_total); }
            }
            pl *= selection;
            w = mis_power(previous_pdf, pl);
        }
        L += emitter_radiance(light,-rd) * w;
    }
    return L;
}
