// Complex dynamics sampled as a native texture. Independent of solid distance fields.
fn planar_mul(a: vec2f,b: vec2f) -> vec2f {
    return vec2f(a.x*b.x-a.y*b.y,a.x*b.y+a.y*b.x);
}
fn planar_power(z: vec2f,power: u32) -> vec2f {
    var result = vec2f(1.0,0.0);
    for (var i=0u; i<8u; i++) {
        if (i >= power) { break; }
        result = planar_mul(result,z);
    }
    return result;
}
fn planar_div(a: vec2f,b: vec2f) -> vec2f {
    return planar_mul(a,vec2f(b.x,-b.y))/max(dot(b,b),1e-24);
}
fn planar_sample(f: PlanarFractal,input: vec2f) -> DeSample {
    let family = f.kind.x;
    let power = f.kind.y;
    let budget = f.kind.z;
    let sn = sin(f.mapping.y);
    let cs = cos(f.mapping.y);
    var point = vec2f(cs*input.x-sn*input.y,sn*input.x+cs*input.y)/f.mapping.x;
    if (f.mapping.z != 0.0) { point *= f.mapping.z/max(dot(point,point),1e-8); }
    point += f.constants.zw;
    let parameter_plane = family == 2u || family == 3u;
    let root = family == 4u || family == 5u;
    let c = select(f.constants.xy,point,parameter_plane);
    var z = select(point,vec2f(0.0),parameter_plane);
    var previous = vec2f(0.0);
    let rotation = vec2f(cos(f.mapping.w),sin(f.mapping.w));
    let lambda = rotation*f.settings.x;
    var ring = 1e20;
    var stripe = 0.0;
    var previous_error = 1.0;
    var count = 0u;
    var measured = 0u;
    var orbit_time = f32(budget);
    var escaped = false;
    for (var i=0u; i<512u; i++) {
        if (i >= budget) { break; }
        let radius = length(z);
        if (radius > f.settings.z) { escaped = true; break; }
        ring = min(ring,abs(radius-f.settings.w));
        var angle = 0.0;
        if (radius != 0.0) { angle = atan2(z.y,z.x); }
        stripe += 0.5+0.5*sin(f.detail.x*angle);
        measured += 1u;
        if (root) {
            let residual = planar_power(z,power)-vec2f(1.0,0.0);
            if (family == 4u && dot(residual,residual) < 1e-12) {
                let error = max(length(residual),1e-20);
                let fraction = clamp(log(1e-6/previous_error)/log(error/previous_error),0.0,1.0);
                orbit_time = select(f32(count)-1.0+fraction,0.0,count==0u); break;
            }
            previous_error = max(length(residual),1e-20);
            let old = z;
            let derivative = planar_power(z,power-1u)*f32(power);
            z -= planar_mul(lambda,planar_div(residual,derivative));
            if (family == 5u) {
                z += c;
                if (dot(z-old,z-old) < 1e-12) { count += 1u; orbit_time = f32(count); break; }
            }
        } else {
            var base = z;
            if (family == 3u) { base = abs(z); }
            var next = planar_mul(rotation,planar_power(base,power))+c;
            if (family == 6u) { next += previous*f.settings.y; }
            previous = z;
            z = next;
        }
        count += 1u;
    }
    let radius = min(length(z),1e20);
    escaped = escaped || radius > f.settings.z;
    if (escaped && !root) {
        orbit_time = f32(count)+1.0-log(log(max(radius,1.0001)))/log(f32(power));
    } else if (escaped) { orbit_time = f32(count); }
    var final_angle = 0.5;
    if (radius != 0.0) { final_angle = atan2(z.y,z.x)/(2.0*PI)+0.5; }
    var bands = stripe/f32(max(measured,1u));
    if (escaped && !root && measured > 0u) {
        var angle = 0.0;
        if (radius != 0.0) { angle = atan2(z.y,z.x); }
        let last = 0.5+0.5*sin(f.detail.x*angle);
        let full = (stripe+last)/f32(measured+1u);
        let t = clamp(1.0+(log(log(f.settings.z))-log(log(max(radius,1.0001))))/log(f32(power)),0.0,1.0);
        bands = mix(bands,full,t);
    }
    let traps = vec4f(clamp(orbit_time/f32(budget),0.0,1.0),ring/(1.0+ring),bands,final_angle);
    return DeSample(radius/(1.0+radius),traps,count,0u,0u);
}
