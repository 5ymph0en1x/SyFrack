// Export-only material guides. Reuse the first sample's camera ray and saved
// geometry, evaluating the same filtered material and bump as the path tracer.
fn store_export_guides(i: u32, albedo: vec3f, normal: vec3f, ray: Ray, distance: f32) {
    var transmission=1.0;
    var in_fog=u.atmosphere.a.x>0.0 && !ocean_inside(ray.o) && !in_otherworld(ray.o);
    if (in_fog && OPTICS_ENABLED) {
        let initial=de_sample_exact(ray.o);
        let optics=material_optics(initial.material);
        if (initial.distance<0.0 && optics.absorption.w>0.0 && optics.surface.y>0.0 && optics.surface.w==0.0) { in_fog=false; }
    }
    if (in_fog) {
        let end=min(distance,portal_intersection(ray.o,ray.d,distance));
        transmission=exp(-fog_optical_depth(ray.o,ray.d,end));
    }
    // Expected primary features: the volume has scattering albedo but no
    // surface normal. Hidden wall detail must fade with its transmission.
    // Do not renormalize: the vector length encodes surface coverage.
    let feature_transmission=select(transmission,1.0,(u.flags & 2u)!=0u);
    accum[i]=vec4f(mix(u.atmosphere.b.xyz,albedo,feature_transmission),1.0);
    // Alpha carries the deterministic optical boundary for volume reconstruction.
    // OIDN reads only the three normal channels from its PFM guide.
    aux[i]=vec4f(normal*feature_transmission,transmission);
}

@compute @workgroup_size(8,8,1)
fn export_guides(@builtin(global_invocation_id) gid: vec3u) {
    if (any(gid.xy >= u.tile_size)) { return; }
    let px = u.tile_origin + gid.xy;
    if (any(px >= u.image_size)) { return; }
    let i = output_index(px);
    let depth = bitcast<f32>(depth_buf[i]);
    let normal = aux[i].xyz;
    accum[i] = vec4f(0.0,0.0,0.0,1.0);
    aux[i] = vec4f(0.0,0.0,0.0,1.0);
    var rng = rng_init(px,0u,u.seed);
    let ray = camera_ray(px,&rng);
    if (depth<=0.0 || dot(normal,normal)<0.5) {
        // A stored sky depth may be the first random fog event; it is not a
        // boundary of the medium and must never drive reconstruction guides.
        store_export_guides(i,vec3f(0.0),vec3f(0.0),ray,fog_extent());
        return;
    }
    let p = ray.o+ray.d*depth;
    for (var j=0u;j<u.light_count;j++) {
        if (is_emitter(u.lights[j]) && abs(emitter_intersection(ray.o,ray.d,u.lights[j])-depth)<0.001) {
            store_export_guides(i,vec3f(1.0),emitter_normal(u.lights[j],p),ray,depth);
            return;
        }
    }
    let cone = 2.0*u.camera.params.x/f32(u.image_size.x);
    var sample = de_sample_exact(p);
    if (u.atmosphere.ground.x>0.0 && abs(ground_intersection(ray.o,ray.d,cone,ray_extent())-depth)<0.001) {
        sample.material = u32(u.atmosphere.ground.z);
    }
    let s = make_surface(p,normal,-ray.d,sample,max(depth*cone*0.5,0.0001));
    store_export_guides(i,clamp(s.diffuse+s.f0,vec3f(0.0),vec3f(1.0)),s.n,ray,depth);
}
