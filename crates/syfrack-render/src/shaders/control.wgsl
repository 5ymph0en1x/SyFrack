// Fixed graph machine. The program, connections and constants are data.
// Scratch is SoA: adjacent invocations access adjacent words of each register.
struct Instruction { op:u32, dst:u32, a:u32, b:u32, p:u32, q:u32, jump:u32, extra:u32 }
struct Value { pd:vec4f, traps:vec4f, tags:vec4u }
struct Machine {
    origin:vec4f, ray:vec4f, trace:vec4f, candidate:vec4f, normal:vec4f, tags:vec4u,
    curve_x:vec4f,curve_k:vec4f,medium:vec4f,hitpoint:vec4f,
}
struct ControlInfo { program:vec4u, config:vec4u }
@group(0) @binding(1) var<storage,read> code:array<Instruction>;
@group(0) @binding(2) var<storage,read_write> memory:array<vec4u>;
@group(0) @binding(3) var<storage,read_write> accum:array<vec4f>;
@group(0) @binding(5) var<storage,read_write> depth:array<f32>;
@group(0) @binding(6) var<storage,read_write> aux:array<vec4f>;
@group(0) @binding(7) var<storage,read_write> status:array<atomic<u32>,4>;
@group(0) @binding(8) var<uniform> control:ControlInfo;

fn word(lane:u32,row:u32)->u32 { return row*control.config.y+lane; }
fn value(lane:u32,reg:u32)->Value {
    return Value(bitcast<vec4f>(memory[word(lane,reg*3u)]),
        bitcast<vec4f>(memory[word(lane,reg*3u+1u)]),memory[word(lane,reg*3u+2u)]);
}
fn put(lane:u32,reg:u32,v:Value) {
    memory[word(lane,reg*3u)]=bitcast<vec4u>(v.pd);
    memory[word(lane,reg*3u+1u)]=bitcast<vec4u>(v.traps);
    memory[word(lane,reg*3u+2u)]=v.tags;
}
fn load_machine(lane:u32)->Machine {
    let base=control.program.x*3u;
    return Machine(bitcast<vec4f>(memory[word(lane,base)]),
        bitcast<vec4f>(memory[word(lane,base+1u)]),bitcast<vec4f>(memory[word(lane,base+2u)]),
        bitcast<vec4f>(memory[word(lane,base+3u)]),bitcast<vec4f>(memory[word(lane,base+4u)]),memory[word(lane,base+5u)],bitcast<vec4f>(memory[word(lane,base+6u)]),bitcast<vec4f>(memory[word(lane,base+7u)]),bitcast<vec4f>(memory[word(lane,base+8u)]),bitcast<vec4f>(memory[word(lane,base+9u)]));
}
fn save_machine(lane:u32,s:Machine) {
    let base=control.program.x*3u;
    memory[word(lane,base)]=bitcast<vec4u>(s.origin);
    memory[word(lane,base+1u)]=bitcast<vec4u>(s.ray);
    memory[word(lane,base+2u)]=bitcast<vec4u>(s.trace);
    memory[word(lane,base+3u)]=bitcast<vec4u>(s.candidate);
    memory[word(lane,base+4u)]=bitcast<vec4u>(s.normal);
    memory[word(lane,base+5u)]=s.tags;
    memory[word(lane,base+6u)]=bitcast<vec4u>(s.curve_x);memory[word(lane,base+7u)]=bitcast<vec4u>(s.curve_k);
    memory[word(lane,base+8u)]=bitcast<vec4u>(s.medium);memory[word(lane,base+9u)]=bitcast<vec4u>(s.hitpoint);
}
fn gp3(at:u32)->vec3f { return vec3f(gp(at),gp(at+1u),gp(at+2u)); }
fn control_empty()->Value { return Value(vec4f(0.0,0.0,0.0,1e30),vec4f(1e30),vec4u(0u)); }
fn sample_origin(s:ptr<function,Machine>,p:vec3f) {
    (*s).origin=vec4f(p,0.0); (*s).tags.x=0u; (*s).tags.y=0u;
}
fn stencil(i:u32)->vec3f {
    let offsets=array<vec3f,4>(vec3f(1.0,-1.0,-1.0),vec3f(-1.0,-1.0,1.0),vec3f(-1.0,1.0,-1.0),vec3f(1.0));
    return offsets[i];
}
fn complete_hit(lane:u32,s:ptr<function,Machine>,t:f32) {
    (*s).candidate.z=t;
    (*s).hitpoint=vec4f(select(u.camera.origin.xyz+(*s).ray.xyz*t,(*s).curve_x.xyz,kerr_enabled()),0.0);
    let cone=u.camera.params.x*2.0/f32(u.image_size.x);
    (*s).candidate.w=max(t*cone*0.5,1e-4)*u.fractal.p0.x;
    (*s).tags.z=2u;
    sample_origin(s,(*s).hitpoint.xyz+stencil(0u)*(*s).candidate.w);
}
fn complete_miss(lane:u32,s:ptr<function,Machine>,captured:bool){
    var medium=(*s).medium;if(!kerr_enabled()){medium=nature_volume(u.camera.origin.xyz,(*s).ray.xyz,ray_extent());}
    accum[lane]=vec4f(medium.xyz+select(vec3f(0.025,0.035,0.05),vec3f(0.0),captured)*medium.w,1.0);(*s).tags.z=6u;
}
fn finish_sample(lane:u32,s:ptr<function,Machine>,v:Value) {
    let combined=nature_surface((*s).origin.xyz,DeSample(v.pd.w,v.traps,v.tags.x,v.tags.y,v.tags.z));
    let d=combined.distance;
    if (control.config.x==1u) {
        let scales=vec4f(gp(0u),gp(1u),gp(2u),gp(3u));
        aux[lane]=select(min(v.traps/scales,vec4f(1.0)),vec4f(1.0),v.traps>=vec4f(1e30));
        accum[lane]=vec4f(d,f32(min(v.tags.x,u.fractal.kind.y)),f32(v.tags.y),f32(v.tags.z));
        (*s).tags.z=6u; return;
    }
    // An undefined distance is a graph evaluation error, never empty space.
    if (!(abs(d)<=3.4028235e38)) {
        (*s).tags.z=8u;atomicStore(&status[3],v.tags.w);return;
    }
    if ((*s).tags.z>=2u) {
        let i=(*s).tags.z-2u;
        (*s).normal=vec4f((*s).normal.xyz+stencil(i)*d,0.0);
        if (i<3u) {
            (*s).tags.z+=1u;
            sample_origin(s,(*s).hitpoint.xyz+stencil(i+1u)*(*s).candidate.w);
        } else {
            let n=safe_normalize((*s).normal.xyz);
            let light=0.20+0.80*max(dot(n,normalize(vec3f(0.4,0.7,0.6))),0.0);
            var color=vec3f(0.68,0.70,0.73)*light;
            let p=(*s).hitpoint.xyz;
            let field=nature_field(p);
            if((field.inside&&(u.phenomena.outputs.x&1u)!=0u)||(u.phenomena.outputs.x&64u)!=0u){color=field.color*light;}
            if(field.inside&&(u.phenomena.outputs.x&2u)!=0u){color+=palette_sample(u.phenomena.palette,field.a.x)*field.a.y*u.phenomena.gains.z;}
            var medium=(*s).medium;if(!kerr_enabled()){medium=nature_volume(u.camera.origin.xyz,(*s).ray.xyz,(*s).candidate.z);}
            accum[lane]=vec4f(medium.xyz+color*medium.w,1.0);
            depth[lane]=(*s).candidate.z;
            aux[lane]=vec4f(n,f32((*s).tags.w));
            (*s).tags.z=6u;
        }
        return;
    }
    if(kerr_enabled()){
        let state=KerrState((*s).curve_x,(*s).curve_k);let radius=kerr_radius(state.x.xyz);
        if(radius<=kerr_horizon()*1.001){complete_miss(lane,s,true);return;}
        let tangent=normalize(kerr_rhs(state).x.xyz);(*s).ray=vec4f(tangent,0.0);
        if((*s).trace.x>=ray_extent()||(radius>=u.phenomena.lens_config.z&&dot(state.x.xyz-u.phenomena.lens_center.xyz,tangent)>0.0)){complete_miss(lane,s,false);return;}
        if((*s).tags.w>=u32(u.phenomena.lens_config.y)){complete_miss(lane,s,true);return;}
        if(abs(d)*u.fractal.p0.w<=max(1e-5,(*s).trace.x*u.camera.params.x*2.0/f32(u.image_size.x)*u.fractal.p0.x)){complete_hit(lane,s,(*s).trace.x);return;}
        let next=kerr_advance(state,kerr_step_size(state,abs(d)*u.fractal.p0.w));
        let medium=nature_chord(state.x.xyz,next.x.xyz);(*s).medium=vec4f((*s).medium.xyz+(*s).medium.w*medium.xyz,(*s).medium.w*medium.w);
        (*s).trace.x+=length(next.x.xyz-state.x.xyz);(*s).curve_x=next.x;(*s).curve_k=next.k;(*s).tags.w+=1u;
        sample_origin(s,next.x.xyz);return;
    }
    // The same over-relaxed sphere tracing and candidate rules as march_segment.
    // Every sample is the exact field: no bounding envelope can become a hit.
    let radius=abs(d)*u.fractal.p0.w;
    let t=(*s).trace.x;
    let failed=(*s).trace.y>1.0 && radius+(*s).trace.z<(*s).trace.w*1.0001;
    if (failed) { (*s).trace.w-=(*s).trace.y*(*s).trace.w; (*s).trace.y=1.0; }
    else { (*s).trace.w=radius*(*s).trace.y; }
    (*s).trace.z=radius;
    let err=radius/max(t,1e-6);
    let cone=u.camera.params.x*2.0/f32(u.image_size.x)*u.fractal.p0.x;
    if (!failed && err<(*s).candidate.y) { (*s).candidate.x=t; (*s).candidate.y=err; }
    if (!failed && err<cone) { complete_hit(lane,s,t); return; }
    let end=ray_extent();
    if (!failed && t+(*s).trace.w>=end) {
        if (radius>=end-t) { complete_miss(lane,s,false); return; }
        (*s).trace.y=1.0; (*s).trace.w=radius;
    }
    (*s).trace.x+=(*s).trace.w;
    if ((*s).trace.x>=end) { complete_miss(lane,s,false); return; }
    // Instruction quanta bound a dispatch, not the lifetime of a ray. Continue
    // until a real hit or a proven exit; never abandon a ray at a step budget.
    (*s).tags.w=min((*s).tags.w,0xfffffffeu)+1u;
    sample_origin(s,u.camera.origin.xyz+(*s).ray.xyz*(*s).trace.x);
}

fn instruction(lane:u32,s:ptr<function,Machine>) {
    if ((*s).tags.x>=control.program.z) { (*s).tags.z=8u; return; }
    let i=code[(*s).tags.x];
    (*s).tags.x+=1u;
    // Every register referenced by an instruction has been range-checked by the
    // host lowerer. Control-flow indices and call depth are checked again here.
    var v=value(lane,i.dst);
    let a=value(lane,i.a); let b=value(lane,i.b);
    var p=v.pd.xyz; var dr=v.pd.w;
    switch i.op {
        case OP_Input: { v=Value(vec4f((*s).origin.xyz,1.0),vec4f(1e30),vec4u(0u)); }
        case OP_Copy: { v=a; if (i.extra!=0u) { v.pd.w=1e30; v.tags.y=0u; } }
        case OP_Translate: { p+=gp3(i.p); v.pd=vec4f(p,dr); }
        case OP_Rotate: { p=rotate_euler_xyz(p,radians(gp3(i.p))); v.pd=vec4f(p,dr); }
        case OP_Scale: { let f=gp(i.p); v.pd=vec4f(p*f,dr*abs(f)); }
        case OP_ScaleAxes: { let f=gp3(i.p); v.pd=vec4f(p*f,dr*max(max(f.x,f.y),f.z)); }
        case OP_Mirror: { let n=safe_normalize(gp3(i.p)); let h=dot(p,n)-gp(i.p+3u); if (h<0.0) {p-=2.0*h*n;} v.pd=vec4f(p,dr); }
        case OP_BoxFold: { let l=gp(i.p); v.pd=vec4f(clamp(p,vec3f(-l),vec3f(l))*2.0-p,dr); }
        case OP_SphereFold: {
            let r2=dot(p,p);let mr=gp(i.p);let fr=gp(i.p+1u);let mr2=mr*mr;let fr2=fr*fr;
            var k=1.0;if(r2<mr2){k=fr2/mr2;}else if(r2<fr2){k=fr2/r2;}v.pd=vec4f(p*k,dr*k);
        }
        case OP_BoxFoldAxes: { let l=gp3(i.p);let folded=clamp(p,-l,l)*2.0-p;v.pd=vec4f(select(p,folded,(vec3u(i.q)&vec3u(1u,2u,4u))!=vec3u(0u)),dr); }
        case OP_SphereInversion: {let r=gp(i.p);let m=gp(i.p+1u);let k=r*r/max(dot(p,p),m*m);v.pd=vec4f(p*k,dr*k);}
        case OP_AddConstant: {let m=gp3(i.p);v.pd=vec4f(p+(*s).origin.xyz*m+gp3(i.p+3u),dr+max(max(abs(m.x),abs(m.y)),abs(m.z)));}
        case OP_Abs: {v.pd=vec4f(select(p,abs(p),(vec3u(i.q)&vec3u(1u,2u,4u))!=vec3u(0u)),dr);}
        case OP_MengerFold: {
            var z=abs(p);if(z.x<z.y){z=vec3f(z.y,z.x,z.z);}if(z.x<z.z){z=vec3f(z.z,z.y,z.x);}if(z.y<z.z){z=vec3f(z.x,z.z,z.y);}
            let scale=gp(i.p);let off=gp3(i.p+1u);z=z*scale-off*(scale-1.0);if(z.z< -0.5*off.z*(scale-1.0)){z.z+=off.z*(scale-1.0);}v.pd=vec4f(z,dr*scale);
        }
        case OP_Kaleido: {let sector=6.2831855/f32(max(i.q,1u));var angle=atan2(p.z,p.x)+radians(gp(i.p));angle=abs(modf_euclid(angle+sector*0.5,sector)-sector*0.5);let r=length(p.xz);v.pd=vec4f(r*cos(angle),p.y,r*sin(angle),dr);}
        case OP_Power: {
            let power=gp(i.p);let radius=length(p);
            if(power==6.0 || power==8.0){
                if(radius>=1e-12){let powered=bulb_power68(p,radius,power);v.pd=vec4f(powered.xyz,dr*powered.w);}else{v.pd=vec4f(0.0);}
            } else if(power==3.0 && radius>=1e-12) {
                let powered=cubic_polar(p,radius);v.pd=vec4f(powered.xyz,dr*powered.w);
            } else {
                let r=max(radius,1e-12);var theta=acos(clamp(p.z/r,-1.0,1.0));if(radius>=1e-12){theta=atan2(length(p.xy),p.z);}
                let phi=atan2(p.y,p.x);dr*=pow(r,power-1.0)*power;
                let zr=pow(r,power);let st=sin(theta*power);let ct=cos(theta*power);let sp=sin(phi*power);let cp=cos(phi*power);
                v.pd=vec4f(vec3f(st*cp,st*sp,ct)*zr,dr);
            }
        }
        case OP_AddOrigin: {v.pd=vec4f(p+(*s).origin.xyz,dr+1.0);}
        case OP_AddSeed: {let m=gp3(i.p);v.pd=vec4f(p+a.pd.xyz*m,dr+a.pd.w*max(max(abs(m.x),abs(m.y)),abs(m.z)));}
        case OP_Twist: {let r=rot2(p.xz,gp(i.p)*p.y);v.pd=vec4f(r.x,p.y,r.y,dr);}
        case OP_Bend: {let r=rot2(p.xy,gp(i.p)*p.x);v.pd=vec4f(r,p.z,dr);}
        case OP_Tile: {let per=gp3(i.p);let half=per*0.5;v.pd=vec4f(modf_euclid3(p+half,per)-half,dr);}
        case OP_RepeatFinite: {let per=gp3(i.p);let last=gp3(i.p+3u)-vec3f(1.0);let half=last*0.5;let cell=clamp(floor(p/per+half+vec3f(0.5)),vec3f(0.0),last)-half;v.pd=vec4f(p-cell*per,dr);}
        case OP_Trap: {
            var m=length(p);switch ((i.q>>4u)&15u) {
                case 1u:{m=length(p.yz);}case 2u:{m=length(p.xz);}case 3u:{m=length(p.xy);}case 4u:{m=abs(p.y);}
                case 5u:{m=orbit_angle(p.xy);}case 6u:{m=orbit_angle(p.xz);}case 7u:{m=orbit_angle(p.yz);}case 8u:{m=log(1.0+length(p));}default:{}
            }
            let slot=i.q&3u;switch(i.q>>8u) {
                case 0u:{v.traps[slot]=min(v.traps[slot],m);}
                case 1u:{v.traps[slot]=select(max(v.traps[slot],m),m,v.traps[slot]>=1e30);}
                default:{v.traps[slot]=m;}
            }
        }
        case OP_Sphere:{v=a;v.pd.w=(length(a.pd.xyz)-gp(i.p))/a.pd.w;}
        case OP_Torus:{v=a;v.pd.w=(length(vec2f(length(a.pd.xz)-gp(i.p),a.pd.y))-gp(i.p+1u))/a.pd.w;}
        case OP_Cylinder:{v=a;v.pd.w=cylinder_distance(a.pd.xyz,gp(i.p),gp(i.p+1u))/a.pd.w;}
        case OP_Box:{v=a;v.pd.w=box_distance(a.pd.xyz,gp3(i.p))/a.pd.w;}
        case OP_RoundBox:{v=a;v.pd.w=round_box_distance(a.pd.xyz,gp3(i.p),gp(i.p+3u))/a.pd.w;}
        case OP_Rock:{v=a;v.pd.w=rock_distance(a.pd.xyz,gp3(i.p),gp(i.p+3u),u32(gp(i.p+4u)))/a.pd.w;}
        case OP_Plane:{v=a;v.pd.w=(dot(a.pd.xyz,safe_normalize(gp3(i.p)))-gp(i.p+3u))/a.pd.w;}
        case OP_NormFence:{v=a;v.pd.w=(length(a.pd.xyz)-gp(i.p))/abs(a.pd.w);}
        case OP_BulbFence:{v=a;v.pd.w=bulb_distance(a.pd.xyz,a.pd.w);}
        case OP_Union:{v=b;if(a.pd.w<=b.pd.w){v=a;}v.pd.w=min(a.pd.w,b.pd.w);}
        case OP_Intersection:{v=b;if(a.pd.w>=b.pd.w){v=a;}v.pd.w=max(a.pd.w,b.pd.w);}
        case OP_Subtraction:{v=a;v.pd.w=max(a.pd.w,-b.pd.w);}
        case OP_SmoothUnion,OP_SmoothIntersection:{
            let k=gp(i.p);let sign=select(-1.0,1.0,i.op==OP_SmoothIntersection);
            let h=clamp(0.5-sign*0.5*(b.pd.w-a.pd.w)/k,0.0,1.0);
            v=b;if(h>=0.5){v=a;}v.pd.w=b.pd.w+(a.pd.w-b.pd.w)*h+sign*k*h*(1.0-h);v.traps=blend_traps(a.traps,b.traps,h);
            if(a.pd.w>=1e30){v=b;if(i.op==OP_SmoothIntersection){v.pd.w=1e30;}}
            else if(b.pd.w>=1e30){v=a;if(i.op==OP_SmoothIntersection){v.pd.w=1e30;}}
        }
        case OP_Material:{v=a;v.tags.y=u32(gp(i.p));}
        case OP_Bounded:{v=b;v.pd.w=max(box_distance(a.pd.xyz,gp3(i.p))/a.pd.w,b.pd.w);}
        case OP_LoopBegin:{v=a;v.tags.y=0u;}
        case OP_LoopTest:{if(a.tags.y>=u32(gp(i.p)) || length(p)>gp(i.p+1u)){(*s).tags.x=i.jump;}return;}
        case OP_LoopEnd:{v.tags.x+=1u;var counter=a;counter.tags.y+=1u;put(lane,i.a,counter);(*s).tags.x=i.jump;}
        case OP_InstanceBegin:{v=control_empty();put(lane,i.a,Value(vec4f(0.0),vec4f(0.0),vec4u(0u,0xffffffffu,0u,0u)));}
        case OP_InstanceNext:{
            var it=a;let cursor=it.tags.x;
            if(cursor>=2u*u32(gp(i.extra+3u))-1u){(*s).tags.x=i.jump;return;}
            let at=i.q+cursor*8u;let bound=box_distance((*s).origin.xyz-gp3(at),gp3(at+3u));
            it.tags.x=cursor+1u;
            if(bound>v.pd.w){it.tags.x=u32(gp(at+6u));put(lane,i.a,it);(*s).tags.x-=1u;return;}
            let leaf=gp(at+7u);if(leaf<0.0){put(lane,i.a,it);(*s).tags.x-=1u;return;}
            let j=u32(leaf);let row=(i.p+j*12u)/4u;let x=gp_row(row);let y=gp_row(row+1u);let z=gp_row(row+2u);
            let world=vec4f((*s).origin.xyz,1.0);let local=vec3f(dot(x,world),dot(y,world),dot(z,world));
            let scales=inverseSqrt(vec3f(dot(x.xyz,x.xyz),dot(y.xyz,y.xyz),dot(z.xyz,z.xyz)));
            let fence=box_distance(local*scales,gp3(i.extra)*scales);
            if(fence>v.pd.w){put(lane,i.a,it);(*s).tags.x-=1u;return;}
            it.pd=vec4f(min(min(scales.x,scales.y),scales.z),fence,0.0,0.0);it.tags.z=j;
            put(lane,i.a,it);put(lane,i.b,Value(vec4f(local,1.0),vec4f(1e30),vec4u(0u)));return;
        }
        case OP_InstanceAccumulate:{
            var candidate=a;candidate.pd.w=max(candidate.pd.w*b.pd.x,b.pd.y);
            if(candidate.pd.w<v.pd.w || (candidate.pd.w==v.pd.w && b.tags.z<b.tags.y)){
                v=candidate;var it=b;it.tags.y=it.tags.z;put(lane,i.b,it);
            }
            (*s).tags.x=i.jump;
        }
        case OP_Call:{
            let sp=(*s).tags.y;if(sp>=control.program.y){(*s).tags.z=8u;return;}
            let base=control.program.x*3u+10u+sp*2u;
            memory[word(lane,base)]=bitcast<vec4u>((*s).origin);
            memory[word(lane,base+1u)]=vec4u((*s).tags.x,i.dst,0u,0u);
            (*s).origin=vec4f(a.pd.xyz,0.0);(*s).tags.y=sp+1u;(*s).tags.x=i.jump;return;
        }
        case OP_Return:{
            if((*s).tags.y==0u){finish_sample(lane,s,a);}else{
                (*s).tags.y-=1u;let base=control.program.x*3u+10u+(*s).tags.y*2u;
                (*s).origin=bitcast<vec4f>(memory[word(lane,base)]);let ret=memory[word(lane,base+1u)];
                (*s).tags.x=ret.x;put(lane,ret.y,a);
            }return;
        }
        default:{(*s).tags.z=8u;return;}
    }
    if (i.op>=OP_Sphere && i.op<=OP_BulbFence) {v.tags.w=i.extra;}
    put(lane,i.dst,v);
}

@compute @workgroup_size(64)
fn control_init(@builtin(global_invocation_id) id:vec3u) {
    let lane=id.x;if(lane>=control.config.y){return;}
    var s:Machine;
    if(control.config.x==1u){s.origin=accum[lane];}else{
        let px=vec2u(lane%u.image_size.x,lane/u.image_size.x);
        let ndc=((vec2f(px)+vec2f(0.5))/vec2f(u.image_size))*2.0-1.0;
        s.ray=vec4f(normalize(u.camera.forward.xyz+u.camera.right.xyz*(ndc.x*u.camera.params.x)+u.camera.up.xyz*(-ndc.y*u.camera.params.x/u.camera.params.y)),0.0);
        s.origin=u.camera.origin;
    }
    s.medium=vec4f(0.0,0.0,0.0,1.0);
    if(kerr_enabled()&&control.config.x==0u){let curve=kerr_launch(s.origin.xyz,s.ray.xyz);s.curve_x=curve.x;s.curve_k=curve.k;}
    s.trace=vec4f(0.0,1.2,0.0,0.0);s.candidate=vec4f(0.0,1e30,0.0,0.0);s.tags=vec4u(0u,0u,1u,1u);
    accum[lane]=vec4f(0.025,0.03,0.04,1.0);depth[lane]=-1.0;aux[lane]=vec4f(0.0,0.0,0.0,-1.0);
    if((u.flags&1u)!=0u&&control.config.x==0u){
        let px=vec2u(lane%u.image_size.x,lane/u.image_size.x);
        let uv=(vec2f(px)+vec2f(0.5))/vec2f(u.image_size);
        let point=vec2f(uv.x-0.5,(0.5-uv.y)/u.camera.params.y)*(6.0/u.camera.params.x);
        let c=u.camera.origin.z;let sn=u.camera.origin.w;
        let q=u.camera.origin.xy+vec2f(c*point.x-sn*point.y,sn*point.x+c*point.y);
        let field=nature_field(vec3f(q,0.0));var color=field.color;
        if((u.phenomena.outputs.x&2u)!=0u){color+=field.color*field.a.y*u.phenomena.gains.z;}
        accum[lane]=vec4f(color,1.0);depth[lane]=1.0;aux[lane]=vec4f(0.0,0.0,1.0,0.0);s.tags.z=6u;
    }
    save_machine(lane,s);
}
@compute @workgroup_size(64)
fn control_step(@builtin(global_invocation_id) id:vec3u) {
    let lane=id.x;if(lane>=control.config.y){return;}
    var s=load_machine(lane);
    for(var n=0u;n<control.program.w;n++){
        if(s.tags.z>=6u){break;}
        instruction(lane,&s);
    }
    save_machine(lane,s);
    if(s.tags.z<6u){atomicAdd(&status[0],1u);}
    if(s.tags.z==8u){atomicAdd(&status[2],1u);}
}
