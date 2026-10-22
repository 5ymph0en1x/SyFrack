// Orbit-density source for the common creation graph. 65536 integer mass units
// per retained point conserve bilinear mass. Two u32 words implement exact
// positive 64-bit sums, including collapsed attractors on legacy GPUs.
struct Info { counts:vec4u, batch:vec4u }
@group(0) @binding(0) var<uniform> info:Info;
@group(0) @binding(1) var<storage,read> parameters:array<vec4f>;
@group(0) @binding(2) var<storage,read_write> lanes:array<vec4u>;
@group(0) @binding(3) var<storage,read_write> histogram:array<atomic<u32>>;
@group(0) @binding(4) var<storage,read_write> image:array<vec4f>;
fn random(state:ptr<function,u32>)->f32 {
    *state=*state+0x9e3779b9u;var x=*state;x=(x^(x>>16u))*0x7feb352du;
    x=(x^(x>>15u))*0x846ca68bu;return f32((x^(x>>16u))>>8u)/16777216.0;
}
fn apply_transform(index:u32,p:vec2f,state:ptr<function,u32>)->vec2f {
    let offset=u32(parameters[4].y)+index*5u;
    let a=parameters[offset+1u];let b=parameters[offset+2u];let c=parameters[offset+3u];
    let q=vec2f(a.x*p.x+a.y*p.y+a.z,a.w*p.x+b.x*p.y+b.y);
    let r2=max(dot(q,q),1e-24);let r=sqrt(r2);let theta=atan2(q.y,q.x);
    var result=vec2f(0.0);let terms=u32(parameters[offset].w);let start=u32(parameters[offset+4u].x);
    for(var j=0u;j<terms;j++){
        let t=parameters[start+j];var v=q;
        switch u32(t.x) {
            case 1u:{v=sin(q);}
            case 2u:{v=q/r2;}
            case 3u:{v=vec2f(q.x*sin(r2)-q.y*cos(r2),q.x*cos(r2)+q.y*sin(r2));}
            case 4u:{v=vec2f((q.x-q.y)*(q.x+q.y),2.0*q.x*q.y)/r;}
            case 5u:{v=theta/3.14159265*vec2f(sin(3.14159265*r),cos(3.14159265*r));}
            case 6u:{v=4.0*q/(r2+4.0);}
            case 7u:{let branch=floor(random(state)*t.z);let angle=(theta+6.2831853*branch)/t.z;
                v=pow(r2,t.w/(2.0*t.z))*vec2f(cos(angle),sin(angle));}
            case 8u:{let re=1.0+t.z*q.x+t.w*(q.x*q.x-q.y*q.y);let im=t.z*q.y+2.0*t.w*q.x*q.y;
                v=vec2f(q.x*re+q.y*im,q.y*re-q.x*im)/max(re*re+im*im,1e-24);}
            default:{}
        }
        result+=t.y*v;
    }
    return vec2f(b.z*result.x+b.w*result.y+c.x,c.y*result.x+c.z*result.y+c.w);
}
fn palette(t:f32)->vec3f {
    let p=clamp(t,0.0,1.0)*4095.0;let i=min(u32(floor(p)),4094u);let at=u32(parameters[4].x);
    return mix(parameters[at+i].xyz,parameters[at+i+1u].xyz,fract(p));
}
fn add_moment(at:u32,value:u32){
    let previous=atomicAdd(&histogram[at],value);
    if(previous>0xffffffffu-value){atomicAdd(&histogram[at+1u],1u);}
}
fn moment(at:u32)->f32{return f32(atomicLoad(&histogram[at+1u]))*4294967296.0+f32(atomicLoad(&histogram[at]));}
fn moments(p:vec2i)->vec4f {
    let n=i32(info.counts.x);let q=clamp(p,vec2i(0),vec2i(n-1));let at=(u32(q.y)*info.counts.x+u32(q.x))*8u;
    return vec4f(moment(at+2u),moment(at+4u),moment(at+6u),moment(at))/65536.0;
}
@compute @workgroup_size(64)
fn initialize(@builtin(global_invocation_id) id:vec3u){
    let i=id.x;if(i>=info.counts.z){return;}var rng=info.counts.w^(i*0xa0761d65u);
    let x=random(&rng)*2.0-1.0;let y=random(&rng)*2.0-1.0;let c=random(&rng);
    lanes[i]=vec4u(bitcast<u32>(x),bitcast<u32>(y),bitcast<u32>(c),rng);
}
@compute @workgroup_size(64)
fn accumulate(@builtin(global_invocation_id) id:vec3u){
    let lane=id.x;if(lane>=info.counts.z){return;}
    let count=info.counts.y/info.counts.z+select(0u,1u,lane<info.counts.y%info.counts.z);
    var p=bitcast<vec2f>(lanes[lane].xy);var color=bitcast<f32>(lanes[lane].z);var rng=lanes[lane].w;
    let n=u32(parameters[0].x);let transforms=u32(parameters[4].y);let resolution=info.counts.x;
    for(var step=0u;step<info.batch.y;step++){
        let iteration=info.batch.x+step;if(iteration>=count+64u){break;}
        let choice=random(&rng);var index=n-1u;
        for(var i=0u;i<n;i++){if(choice<parameters[transforms+i*5u].x){index=i;break;}}
        let t=parameters[transforms+index*5u];p=apply_transform(index,p,&rng);color=mix(color,t.y,t.z);
        if(!all(abs(p)<vec2f(1e8))){p=vec2f(random(&rng),random(&rng))*2.0-1.0;continue;}
        if(iteration<64u){continue;}
        var q=p;if(parameters[0].y>=0.0){q=apply_transform(u32(parameters[0].y),p,&rng);}
        q-=parameters[1].xy;let sn=parameters[2].x;let cs=parameters[2].y;
        let uv=vec2f(cs*q.x+sn*q.y,-sn*q.x+cs*q.y)/(parameters[1].zw*2.0)+vec2f(0.5);
        let pixel=vec2f(uv.x,1.0-uv.y)*f32(resolution)-vec2f(0.5);
        if(!all(pixel>=vec2f(0.0))||!all(pixel<vec2f(f32(resolution-1u)))){continue;}
        let lo=vec2u(floor(pixel));let fraction=fract(pixel);let rgb=palette(color);
        let w0=u32(floor((1.0-fraction.x)*(1.0-fraction.y)*65536.0));
        let w1=u32(floor(fraction.x*(1.0-fraction.y)*65536.0));
        let w2=u32(floor((1.0-fraction.x)*fraction.y*65536.0));
        let weights=array<u32,4>(w0,w1,w2,65536u-w0-w1-w2);
        for(var j=0u;j<4u;j++){
            let cell=((lo.y+j/2u)*resolution+lo.x+j%2u)*8u;let w=weights[j];
            add_moment(cell,w);
            for(var c=0u;c<3u;c++){add_moment(cell+2u*(c+1u),u32(round(clamp(rgb[c],0.0,1.0)*f32(w))));}
        }
    }
    lanes[lane]=vec4u(bitcast<vec2u>(p),bitcast<u32>(color),rng);
}
@compute @workgroup_size(8,8)
fn resolve(@builtin(global_invocation_id) id:vec3u){
    let pixel=vec2u(id.x,id.y+info.batch.z);let n=info.counts.x;
    if(pixel.x>=n||pixel.y>=n||id.y>=info.batch.w){return;}
    let center=vec2i(pixel);var tensor=vec3f(0.0);var neighborhood=0.0;
    for(var y=-1;y<=1;y++){for(var x=-1;x<=1;x++){
        let q=center+vec2i(x,y);let gx=log(1.0+moments(q+vec2i(1,0)).w)-log(1.0+moments(q-vec2i(1,0)).w);
        let gy=log(1.0+moments(q+vec2i(0,1)).w)-log(1.0+moments(q-vec2i(0,1)).w);
        tensor+=vec3f(gx*gx,gx*gy,gy*gy);neighborhood+=moments(q).w;
    }}
    if(neighborhood==0.0){image[pixel.y*n+pixel.x]=vec4f(parameters[3].yzw,0.0);return;}
    let angle=0.5*atan2(2.0*tensor.y,tensor.x-tensor.z);let normal=vec2f(cos(angle),sin(angle));
    let coherence=clamp(length(vec2f(tensor.x-tensor.z,2.0*tensor.y))/(tensor.x+tensor.z+1e-12),0.0,1.0);
    let noise=clamp(6.0/max(sqrt(moments(center).w),1.0),0.0,1.0);
    let long=0.42+max(parameters[3].x-0.42,0.0)*noise;let short=mix(long,0.42,coherence);
    let support=i32(ceil(long*2.5));var sum=vec4f(0.0);var total=0.0;
    for(var y=-support;y<=support;y++){for(var x=-support;x<=support;x++){
        let q=center+vec2i(x,y);if(any(q<vec2i(0))||any(q>=vec2i(i32(n)))){continue;}
        let dn=f32(x)*normal.x+f32(y)*normal.y;let dt=-f32(x)*normal.y+f32(y)*normal.x;
        let r2=dn*dn/(short*short)+dt*dt/(long*long);if(r2>6.25){continue;}
        let weight=exp(-0.5*r2);sum+=weight*moments(q);total+=weight;
    }}
    let density=(sum.w/max(total,1e-20))/(f32(info.counts.y)/f32(n*n));
    let gain=parameters[2].w;var energy=density;
    if(gain>1e-6){energy=log(1.0+gain*density)/log(1.0+gain);}
    energy*=parameters[2].z;
    let rgb=parameters[3].yzw+sum.xyz/max(sum.w,1e-20)*energy;
    image[pixel.y*n+pixel.x]=vec4f(rgb,energy);
}
