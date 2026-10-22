// Fixed RK4 kernels: topology-independent reaction/diffusion and nonlinear wave.
struct SimulationInfo {size:vec4u,scale:vec4f,reaction:vec4f,wave:vec4f}
@group(0) @binding(0) var<uniform> info:SimulationInfo;
@group(0) @binding(1) var<storage,read_write> initial:array<vec4f>;
@group(0) @binding(2) var<storage,read_write> stage_a:array<vec4f>;
@group(0) @binding(3) var<storage,read_write> stage_b:array<vec4f>;
@group(0) @binding(4) var<storage,read_write> weighted:array<vec4f>;
@group(0) @binding(5) var<storage,read> coefficients:array<vec4f>;
@group(0) @binding(6) var<storage,read_write> statistics:array<atomic<u32>,4>;
fn state(i:u32,stage:u32)->vec4f{if(stage==0u){return initial[i];}if(stage==1u||stage==3u){return stage_a[i];}return stage_b[i];}
fn adjacent(i:u32,axis:u32,direction:i32)->u32{
    let r=info.size.xyz;let c=vec3u(i%r.x,(i/r.x)%r.y,i/(r.x*r.y));
    var n=i32(c[axis])+direction;let limit=i32(r[axis]);
    if(info.size.w==1u){n=(n%limit+limit)%limit;}else{n=clamp(n,0,limit-1);}
    var out=c;out[axis]=u32(n);return(out.z*r.y+out.y)*r.x+out.x;
}
fn derivative(i:u32,stage:u32)->vec4f{
    let v=state(i,stage);let c=coefficients[i];if(c.z<=0.0||c.w>=0.5){return vec4f(0.0);}
    var lap=vec4f(0.0);for(var axis=0u;axis<3u;axis++){if(info.size[axis]>1u){lap+=(state(adjacent(i,axis,-1),stage)+state(adjacent(i,axis,1),stage)-2.0*v)*info.scale[axis];}}
    if(info.size.w==0u){
        let reaction=v.x*v.y*v.y;let feed=clamp(info.reaction.z+c.y,0.0,1.0);let kill=clamp(info.reaction.w+c.x,0.0,1.0);
        return vec4f(info.reaction.x*lap.x-reaction+feed*(1.0-v.x),info.reaction.y*lap.y+reaction-(feed+kill)*v.y,0.0,0.0);
    }
    let potential=c.x+info.wave.y*dot(v.xy,v.xy);let h=-info.wave.x*lap.xy+potential*v.xy;
    return vec4f(h.y-info.wave.z*h.x+c.y,-h.x-info.wave.z*h.y,0.0,0.0);
}
@compute @workgroup_size(64)fn rk1(@builtin(global_invocation_id)gid:vec3u){let i=gid.x;if(i>=arrayLength(&initial)){return;}let k=derivative(i,0u);weighted[i]=k;stage_a[i]=initial[i]+0.5*info.scale.w*k;}
@compute @workgroup_size(64)fn rk2(@builtin(global_invocation_id)gid:vec3u){let i=gid.x;if(i>=arrayLength(&initial)){return;}let k=derivative(i,1u);weighted[i]+=2.0*k;stage_b[i]=initial[i]+0.5*info.scale.w*k;}
@compute @workgroup_size(64)fn rk3(@builtin(global_invocation_id)gid:vec3u){let i=gid.x;if(i>=arrayLength(&initial)){return;}let k=derivative(i,2u);weighted[i]+=2.0*k;stage_a[i]=initial[i]+info.scale.w*k;}
@compute @workgroup_size(64)fn rk4(@builtin(global_invocation_id)gid:vec3u){let i=gid.x;if(i>=arrayLength(&initial)){return;}let k=derivative(i,3u);let v=initial[i]+(weighted[i]+k)*(info.scale.w/6.0);initial[i]=v;if(!all(abs(v)<=vec4f(3.4028235e38))){atomicOr(&statistics[1],1u);}else{atomicMax(&statistics[0],bitcast<u32>(dot(v.xy,v.xy)));}}
