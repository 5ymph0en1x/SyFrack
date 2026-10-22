// Bounded SoA path storage. A queue carries IDs, never copies complete paths.
// The scene/depth group is shared; film and transport use a separate layout.
struct WaveQueue {
    capacity: u32,
    count: u32,
    sample: u32,
    phase: u32,
    next: atomic<u32>,
    active_count: u32,
    error: atomic<u32>,
    round: u32,
    reserved: array<vec4u, 2>,
    ids: array<u32>,
}
@group(1) @binding(0) var<storage, read_write> wf_data: array<vec4u>;
@group(1) @binding(1) var<storage, read_write> wf_queue: WaveQueue;
@group(1) @binding(2) var<storage, read_write> wf_film: array<vec4u>;
@group(2) @binding(0) var<storage, read_write> wf_args: array<u32>;

struct WavePath {
    ray: Ray,
    throughput: vec3f,
    L: vec3f,
    primary_volume: vec3f,
    prev_normal: vec3f,
    prev_pdf: f32,
    prev_area_total: f32,
    texture_width: f32,
    texture_spread: f32,
    first_depth: f32,
    wavelength:f32,
    first_aux: vec4f,
    rng: u32,
    bounce: u32,
    medium_slot: u32,
    null_surfaces: u32,
    volume_path: bool,
    prev_from_bsdf: bool,
    prev_from_volume: bool,
    prev_volume_nee: bool,
    underwater: bool,
    solid_medium: bool,
    alive: bool,
}
// 12 vec4 rows per path plus 7 rows per hit. All rows are SoA planes.
fn wf_row(field: u32, id: u32) -> u32 { return field*wf_queue.capacity+id; }
fn wf_pixel(id: u32) -> vec2u {
    return u.tile_origin+vec2u(id%u.tile_size.x,id/u.tile_size.x);
}
fn wf_load(id: u32) -> WavePath {
    var p: WavePath;
    p.ray=Ray(bitcast<vec4f>(wf_data[wf_row(0u,id)]).xyz,
              bitcast<vec4f>(wf_data[wf_row(1u,id)]).xyz);
    p.throughput=bitcast<vec4f>(wf_data[wf_row(2u,id)]).xyz;
    p.L=bitcast<vec4f>(wf_data[wf_row(3u,id)]).xyz;
    p.primary_volume=bitcast<vec4f>(wf_data[wf_row(4u,id)]).xyz;
    p.prev_normal=bitcast<vec4f>(wf_data[wf_row(5u,id)]).xyz;
    let f=bitcast<vec4f>(wf_data[wf_row(6u,id)]);
    p.prev_pdf=f.x; p.prev_area_total=f.y; p.texture_width=f.z; p.texture_spread=f.w;
    p.first_aux=bitcast<vec4f>(wf_data[wf_row(7u,id)]);
    p.first_depth=bitcast<f32>(wf_data[wf_row(8u,id)].x);p.wavelength=bitcast<f32>(wf_data[wf_row(8u,id)].y);
    let h=wf_data[wf_row(9u,id)];
    p.rng=h.x; p.bounce=h.y; p.medium_slot=h.z; p.null_surfaces=h.w;
    let flags=wf_data[wf_row(10u,id)].x;
    p.volume_path=(flags&1u)!=0u; p.prev_from_bsdf=(flags&2u)!=0u;
    p.prev_from_volume=(flags&4u)!=0u; p.prev_volume_nee=(flags&8u)!=0u;
    p.underwater=(flags&16u)!=0u; p.solid_medium=(flags&32u)!=0u;
    p.alive=wf_data[wf_row(11u,id)].x!=0u;
    return p;
}
fn wf_save(id: u32, p: WavePath) {
    wf_data[wf_row(0u,id)]=bitcast<vec4u>(vec4f(p.ray.o,0.0));
    wf_data[wf_row(1u,id)]=bitcast<vec4u>(vec4f(p.ray.d,0.0));
    wf_data[wf_row(2u,id)]=bitcast<vec4u>(vec4f(p.throughput,0.0));
    wf_data[wf_row(3u,id)]=bitcast<vec4u>(vec4f(p.L,0.0));
    wf_data[wf_row(4u,id)]=bitcast<vec4u>(vec4f(p.primary_volume,0.0));
    wf_data[wf_row(5u,id)]=bitcast<vec4u>(vec4f(p.prev_normal,0.0));
    wf_data[wf_row(6u,id)]=bitcast<vec4u>(vec4f(p.prev_pdf,p.prev_area_total,p.texture_width,p.texture_spread));
    wf_data[wf_row(7u,id)]=bitcast<vec4u>(p.first_aux);
    wf_data[wf_row(8u,id)]=vec4u(bitcast<u32>(p.first_depth),bitcast<u32>(p.wavelength),0u,0u);
    wf_data[wf_row(9u,id)]=vec4u(p.rng,p.bounce,p.medium_slot,p.null_surfaces);
    let flags=u32(p.volume_path)|(u32(p.prev_from_bsdf)<<1u)|(u32(p.prev_from_volume)<<2u)
        |(u32(p.prev_volume_nee)<<3u)|(u32(p.underwater)<<4u)|(u32(p.solid_medium)<<5u);
    wf_data[wf_row(10u,id)]=vec4u(flags,0u,0u,0u);
    wf_data[wf_row(11u,id)]=vec4u(u32(p.alive),0u,0u,0u);
}
fn wf_save_hit(id: u32, h: Hit) {
    let flags=u32(h.hit)|(u32(h.exhausted)<<1u)|(u32(h.ground)<<2u)|(u32(h.curved)<<3u);
    wf_data[wf_row(12u,id)]=vec4u(bitcast<u32>(h.t),h.steps,flags,h.light_index);
    wf_data[wf_row(13u,id)]=bitcast<vec4u>(h.sample.traps);
    wf_data[wf_row(14u,id)]=vec4u(bitcast<u32>(h.sample.distance),h.sample.iterations,h.sample.material,h.sample.flags);
    wf_data[wf_row(15u,id)]=vec4u(h.ocean_kind,0u,0u,0u);
    if(KERR_ENABLED){
        wf_data[wf_row(16u,id)]=bitcast<vec4u>(vec4f(h.endpoint,0.0));
        wf_data[wf_row(17u,id)]=bitcast<vec4u>(vec4f(h.tangent,0.0));
        wf_data[wf_row(18u,id)]=bitcast<vec4u>(h.medium);
    }
}
fn wf_load_hit(id: u32) -> Hit {
    let a=wf_data[wf_row(12u,id)]; let b=wf_data[wf_row(14u,id)];
    var h: Hit;
    h.t=bitcast<f32>(a.x); h.steps=a.y; h.hit=(a.z&1u)!=0u;
    h.exhausted=(a.z&2u)!=0u; h.ground=(a.z&4u)!=0u; h.light_index=a.w;
    h.sample=DeSample(bitcast<f32>(b.x),bitcast<vec4f>(wf_data[wf_row(13u,id)]),b.y,b.z,b.w);
    h.ocean_kind=wf_data[wf_row(15u,id)].x;h.curved=(a.z&8u)!=0u;
    if(KERR_ENABLED){
        h.endpoint=bitcast<vec4f>(wf_data[wf_row(16u,id)]).xyz;
        h.tangent=bitcast<vec4f>(wf_data[wf_row(17u,id)]).xyz;
        h.medium=bitcast<vec4f>(wf_data[wf_row(18u,id)]);
    }
    return h;
}

// Reuse the reference bounce body verbatim, retaining random draw and MIS order.
// Only the outer continuation becomes a resumable state transition.
fn wf_advance(px: vec2u, state: WavePath, wave_hit: Hit) -> WavePath {
    var rng_value=state.rng; let rng=&rng_value;
    var depth_value=state.first_depth; let first_depth=&depth_value;
    var aux_value=state.first_aux; let first_aux=&aux_value;
    var ray=state.ray; var throughput=state.throughput; var L=state.L;
    var primary_volume=state.primary_volume; var volume_path=state.volume_path;
    var prev_normal=state.prev_normal; var prev_pdf=state.prev_pdf;
    var prev_area_total=state.prev_area_total; var prev_from_bsdf=state.prev_from_bsdf;
    var prev_from_volume=state.prev_from_volume; var prev_volume_nee=state.prev_volume_nee;
    var texture_width=state.texture_width; var texture_spread=state.texture_spread;
    var underwater=state.underwater; var solid_medium=state.solid_medium;
    let wavelength=state.wavelength;var medium_slot=state.medium_slot; var null_surfaces=state.null_surfaces;
    var bounce=state.bounce;
    let pixel_cone=u.camera.params.x*2.0/f32(u.image_size.x);
    let point_stream=pcg(pcg(px.x^u.seed)^pcg(px.y^0x27d4eb2du));
    let point_index=u.sample_start+wf_queue.sample;
    let training_slot=NO_GUIDE_RECORD;
    var training_seen=0u; var training_rng=0u;
    var training=GuideRecord(false,vec3f(1.0),vec3f(0.0));
    var wf_alive=false;
    loop {
$BOUNCE_BODY
        wf_alive=true;
        break;
    }
    var result=state;
    result.rng=rng_value; result.first_depth=depth_value; result.first_aux=aux_value;
    result.ray=ray; result.throughput=throughput; result.L=L;
    result.primary_volume=primary_volume; result.volume_path=volume_path;
    result.prev_normal=prev_normal; result.prev_pdf=prev_pdf;
    result.prev_area_total=prev_area_total; result.prev_from_bsdf=prev_from_bsdf;
    result.prev_from_volume=prev_from_volume; result.prev_volume_nee=prev_volume_nee;
    result.texture_width=texture_width; result.texture_spread=texture_spread;
    result.underwater=underwater; result.solid_medium=solid_medium;
    result.medium_slot=medium_slot; result.null_surfaces=null_surfaces;
    result.bounce=bounce; result.alive=wf_alive;
    return result;
}

@compute @workgroup_size(1)
fn wf_start() {
    wf_queue.sample+=1u; wf_queue.phase=1u; wf_queue.active_count=0u;
    atomicStore(&wf_queue.next,wf_queue.count); wf_queue.round=0u;
}
@compute @workgroup_size(64)
fn wf_generate(@builtin(global_invocation_id) gid: vec3u) {
    let id=gid.x; if (id>=wf_queue.count) { return; }
    let px=wf_pixel(id); var p: WavePath;
    var initial_rng=rng_init(px,u.sample_start+wf_queue.sample,u.seed);
    p.ray=camera_ray(px,&initial_rng);p.throughput=vec3f(1.0);p.wavelength=550.0;
    if(DISPERSION_ENABLED&&u.phenomena.outputs.z!=0u){let channel=spectral_channel(px,u.sample_start+wf_queue.sample);p.wavelength=array<f32,3>(610.0,550.0,460.0)[channel];p.throughput=vec3f(0.0);p.throughput[channel]=3.0;}
    p.rng=initial_rng;
    let pixel_cone=u.camera.params.x*2.0/f32(u.image_size.x);
    p.texture_spread=pixel_cone*0.5;
    p.first_depth=-1.0; p.first_aux=vec4f(0.0,0.0,0.0,-1.0);
    p.underwater=ocean_inside(p.ray.o); p.alive=true;
    if (OPTICS_ENABLED) {
        let initial=scene_sample_exact(p.ray.o); let optics=material_optics(initial.material);
        p.solid_medium=initial.distance<0.0 && optics.absorption.w>0.0
            && optics.surface.y>0.0 && optics.surface.w==0.0;
        p.medium_slot=initial.material;
    }
    wf_save(id,p); wf_queue.ids[id]=id;
}
@compute @workgroup_size(1)
fn wf_prepare() {
    wf_queue.phase^=1u; wf_queue.active_count=atomicLoad(&wf_queue.next);
    if (wf_queue.active_count>wf_queue.count) { atomicOr(&wf_queue.error,1u); wf_queue.active_count=0u; }
    atomicStore(&wf_queue.next,0u); wf_queue.round+=1u;
    wf_args[0]=(wf_queue.active_count+63u)/64u; wf_args[1]=1u; wf_args[2]=1u;
}
@compute @workgroup_size(64)
fn wf_intersect(@builtin(global_invocation_id) gid: vec3u) {
    if (gid.x>=wf_queue.active_count) { return; }
    let id=wf_queue.ids[wf_queue.phase*wf_queue.capacity+gid.x];
    let ro=bitcast<vec4f>(wf_data[wf_row(0u,id)]).xyz;
    let rd=bitcast<vec4f>(wf_data[wf_row(1u,id)]).xyz;
    var h=trace_scene(ro,rd,u.camera.params.x*2.0/f32(u.image_size.x),ray_extent());
    h=optical_refine_hit(ro,rd,h); wf_save_hit(id,h);
}
@compute @workgroup_size(64)
fn wf_shade(@builtin(global_invocation_id) gid: vec3u) {
    if (gid.x>=wf_queue.active_count) { return; }
    let id=wf_queue.ids[wf_queue.phase*wf_queue.capacity+gid.x];
    wf_save(id,wf_advance(wf_pixel(id),wf_load(id),wf_load_hit(id)));
}
var<workgroup> wf_scan: array<u32,64>;
var<workgroup> wf_base: u32;
@compute @workgroup_size(64)
fn wf_compact(@builtin(global_invocation_id) gid: vec3u, @builtin(local_invocation_index) lane: u32) {
    var id=0u; var live=0u;
    if (gid.x<wf_queue.active_count) {
        id=wf_queue.ids[wf_queue.phase*wf_queue.capacity+gid.x];
        live=wf_data[wf_row(11u,id)].x;
    }
    wf_scan[lane]=live; workgroupBarrier();
    for (var offset=1u;offset<64u;offset*=2u) {
        var previous=0u; if (lane>=offset) { previous=wf_scan[lane-offset]; }
        workgroupBarrier(); wf_scan[lane]+=previous; workgroupBarrier();
    }
    if (lane==0u) { wf_base=atomicAdd(&wf_queue.next,wf_scan[63u]); }
    workgroupBarrier();
    if (live!=0u) {
        let out=wf_base+wf_scan[lane]-1u;
        if (out>=wf_queue.capacity) { atomicOr(&wf_queue.error,2u); return; }
        wf_queue.ids[(wf_queue.phase^1u)*wf_queue.capacity+out]=id;
    }
}
fn wf_read_film(plane: u32, id: u32) -> vec4f {
    if (plane>=5u) { return bitcast<vec4f>(wf_film[5u*wf_queue.capacity+3u*id+plane-5u]); }
    return bitcast<vec4f>(wf_film[plane*wf_queue.capacity+id]);
}
fn wf_write_film(plane: u32, id: u32, value: vec4f) {
    if (plane>=5u) { wf_film[5u*wf_queue.capacity+3u*id+plane-5u]=bitcast<vec4u>(value); return; }
    wf_film[plane*wf_queue.capacity+id]=bitcast<vec4u>(value);
}
@compute @workgroup_size(64)
fn wf_commit(@builtin(global_invocation_id) gid: vec3u) {
    if (gid.x>=wf_queue.count) { return; }
    let p=wf_load(gid.x);
    if (p.alive) { atomicOr(&wf_queue.error,4u); return; }
    let idx=output_index(wf_pixel(gid.x));
    let r=p.L; let volume=select(p.primary_volume,r,p.volume_path);
    let ok=all(r==r)&&all(abs(r)<vec3f(1e30));
    wf_write_film(0u,idx,wf_read_film(0u,idx)+vec4f(select(vec3f(0.0),r,ok),1.0));
    let v=select(vec3f(0.0),volume,ok);
    let old=wf_read_film(2u,idx); let old_mean=old.xyz/max(old.w,1.0);
    let delta=v-old_mean; let new_mean=old_mean+delta/(old.w+1.0);
    if ((u.flags&4u)!=0u) {
        let component=wf_read_film(5u,idx); let direct=select(vec3f(0.0),p.primary_volume,ok);
        let direct_delta=direct-component.xyz/max(old.w,1.0);
        let direct_mean=(component.xyz+direct)/(old.w+1.0);
        let indirect=v-direct; let indirect_delta=indirect-(old.xyz-component.xyz)/max(old.w,1.0);
        let indirect_mean=(old.xyz+v-component.xyz-direct)/(old.w+1.0);
        wf_write_film(5u,idx,component+vec4f(direct,1.0));
        wf_write_film(6u,idx,wf_read_film(6u,idx)+vec4f(max(vec3f(0.0),direct_delta*(direct-direct_mean)),1.0));
        wf_write_film(7u,idx,wf_read_film(7u,idx)+vec4f(max(vec3f(0.0),indirect_delta*(indirect-indirect_mean)),1.0));
    }
    wf_write_film(3u,idx,wf_read_film(3u,idx)+vec4f(max(vec3f(0.0),delta*(v-new_mean)),1.0));
    wf_write_film(2u,idx,old+vec4f(v,1.0));
    wf_write_film(4u,idx,wf_read_film(4u,idx)+vec4f(select(vec3f(0.0),r-volume,ok),1.0));
    if (u.sample_start+wf_queue.sample==0u) {
        depth_buf[idx]=bitcast<u32>(p.first_depth); wf_write_film(1u,idx,p.first_aux);
    }
}
