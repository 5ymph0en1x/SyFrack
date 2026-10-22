fn material_optics(index: u32) -> Optics {
    return u.optics[select(0u, index, index <= u.finish_count.x)];
}

// Pixel-sized surface errors are useful for opaque detail, but would create
// duplicate interfaces or erase thin glass. Refine only optical intersections.
fn optical_refine_hit(ro: vec3f, rd: vec3f, hit: Hit) -> Hit {
    var h = hit;
    if (h.curved || !OPTICS_ENABLED || !h.hit || h.ground || h.ocean_kind != 0u || h.light_index != NO_LIGHT
        || material_optics(h.sample.material).absorption.w == 0.0) { return h; }
    let epsilon = max(1e-5,min(1e-3,h.t*1e-5));
    for (var i=0u;i<8u;i++) {
        let p = ro+rd*h.t;
        let d = scene_sample_exact(p).distance;
        if (abs(d) < 1e-5) { break; }
        let derivative = (scene_sample_exact(p+rd*epsilon).distance
            -scene_sample_exact(p-rd*epsilon).distance)/(2.0*epsilon);
        if (abs(derivative)<1e-5) { break; }
        h.t = max(0.0,h.t-clamp(d/derivative,-0.5,0.5));
    }
    h.sample = scene_sample_exact(ro+rd*h.t);
    return h;
}

struct OpticalBounce {
    direction: vec3f,
    weight: vec3f,
    transmitted: bool,
    valid: bool,
}

// Sample the dielectric as its own mixture component. Its VNDF reflection
// and transmission are integrated by continuation; NEE is deliberately not
// mixed into this component, avoiding mismatched transmission PDFs/caustics.
fn optical_bounce(o: Optics, n: vec3f, rd: vec3f, alpha: f32, entering: bool,
    thickness:f32,wavelength:f32,rng: ptr<function,u32>) -> OpticalBounce {
    let index=dispersed_ior(o.surface.z,wavelength);
    let coated=NATURE_FILM_ENABLED&&thickness>=0.0;
    if (index <= 1.0001&&!coated) {
        let weight = select(vec3f(1.0),exp(-o.absorption.xyz*o.absorption.w),o.surface.w>0.0);
        return OpticalBounce(rd,weight,true,true);
    }
    let thin = o.surface.w > 0.0;
    let eta = select(index, 1.0 / index, entering||thin);
    let h = sample_ggx_h(n,-rd,alpha,rand(rng),rand(rng));
    var f = ocean_fresnel(max(0.0,dot(-rd,h)),eta);
    if (thin) { f = 2.0*f/(1.0+f); }
    var color=vec3f(f);var probability=f;
    if(coated){let n0=select(index,1.0,entering||thin);let n2=select(1.0,index,entering&&!thin);var coat=vec3f(0.0);let cosine=max(0.0,dot(-rd,h));if(DISPERSION_ENABLED&&u.phenomena.outputs.z!=0u){coat=vec3f(film_between(cosine,wavelength,thickness,n0,vec2f(n2,0.0)));}else{coat=film_rgb_between(cosine,thickness,n0,vec2f(n2,0.0));}color=mix(color,coat,u.phenomena.film.x);probability=dot(color,vec3f(1.0/3.0));}
    let transmitted=refract(rd,h,eta);if(!thin&&dot(transmitted,transmitted)<0.5){color=vec3f(1.0);probability=1.0;}
    let a2 = alpha*alpha;
    if (rand(rng) < probability) {
        let direction = reflect(rd,h);
        let cosine = dot(n,direction);
        return OpticalBounce(direction,color/max(probability,1e-20)*smith_g1(max(cosine,1e-6),a2),false,cosine>0.0);
    }
    var direction = transmitted;
    if (thin) { direction = rd; }
    let cosine = -dot(n,direction);
    var weight = vec3f(smith_g1(max(cosine,1e-6),a2)*eta*eta);
    if (thin) { weight = exp(-o.absorption.xyz*o.absorption.w); }
    weight*=(vec3f(1.0)-color)/max(1.0-probability,1e-20);
    return OpticalBounce(direction,weight,true,cosine>0.0 && dot(direction,direction)>0.5);
}
