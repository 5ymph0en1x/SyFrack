const NATURE_FIELDS_ENABLED: bool = true;
const NATURE_VOLUME_ENABLED: bool = true;
const NATURE_FILM_ENABLED: bool = true;
const DISPERSION_ENABLED: bool = true;
// Shared finite fields. Two RGBA rows per vertex, sampled identically in
// the fixed control machine and the specialized path transport.
struct NatureValue { a:vec4f, b:vec4f, color:vec3f, inside:bool }
fn nature_vertex(c:vec3u)->NatureValue {
    let r=u.phenomena.grid.yzw;
    let index=(c.z*r.y+c.y)*r.x+c.x;
    let at=u.phenomena.grid.x+index*2u;
    return NatureValue(gp_row(at),gp_row(at+1u),vec3f(0.0),true);
}
fn nature_field_base(p:vec3f)->NatureValue {
    if(!NATURE_FIELDS_ENABLED||u.phenomena.grid.y==0u){return NatureValue(vec4f(0.0),vec4f(0.0),vec3f(0.0),false);}
    let uv=(p-u.phenomena.center.xyz+u.phenomena.half.xyz)/(2.0*u.phenomena.half.xyz);
    let inside=all(uv>=vec3f(0.0))&&all(uv<=vec3f(1.0));
    let size=u.phenomena.grid.yzw-vec3u(1u);
    let q=clamp(uv,vec3f(0.0),vec3f(1.0))*vec3f(size);
    let low=vec3u(floor(q));let high=min(low+vec3u(1u),size);let f=fract(q);
    var a=vec4f(0.0);var b=vec4f(0.0);
    for(var z=0u;z<select(2u,1u,size.z==0u);z++){for(var y=0u;y<2u;y++){for(var x=0u;x<2u;x++){
        let c=vec3u(select(low.x,high.x,x==1u),select(low.y,high.y,y==1u),select(low.z,high.z,z==1u));
        let weight=select(1.0-f.x,f.x,x==1u)*select(1.0-f.y,f.y,y==1u)*select(1.0-f.z,f.z,z==1u);
        let v=nature_vertex(c);a+=v.a*weight;b+=v.b*weight;
    }}}
    return NatureValue(a,b,palette_sample(u.phenomena.palette,a.x),inside);
}
fn nature_field(p:vec3f)->NatureValue {
    let base=nature_field_base(p);
    if(NATURE_FIELDS_ENABLED&&u.phenomena.program.y>0u){return creation_field(p,base);}
    return base;
}
fn nature_surface(p:vec3f,base:DeSample)->DeSample {
    if(!NATURE_FIELDS_ENABLED||(u.phenomena.outputs.x&24u)==0u||u.phenomena.grid.y==0u){return base;}
    // Geometry is compiled to the conservative finite field. Appearance VM
    // evaluation here would repeat all orbit/palette work at every march step.
    let v=nature_field_base(p);var result=base;
    if((u.phenomena.outputs.x&8u)!=0u&&u.phenomena.outputs.y!=0u){
        let boundary=box_distance(p-u.phenomena.center.xyz,u.phenomena.half.xyz);
        let distance=max(boundary,(v.a.w-u.phenomena.gains.x)/u.phenomena.center.w);
        let take=u.phenomena.outputs.y==3u||(u.phenomena.outputs.y==1u&&distance<result.distance)||(u.phenomena.outputs.y==2u&&distance>result.distance);
        if(take){result=DeSample(distance,vec4f(v.a.x,0.0,0.0,0.0),0u,0u,0u);}
    }
    if((u.phenomena.outputs.x&16u)!=0u){
        let amount=u.phenomena.gains.y;let value=select(0.0,v.b.x,v.inside);
        result.distance=(result.distance-amount*value)/(1.0+abs(amount)*u.phenomena.half.w);
    }
    return result;
}
fn film_mul(a:vec2f,b:vec2f)->vec2f{return vec2f(a.x*b.x-a.y*b.y,a.x*b.y+a.y*b.x);}
fn film_div(a:vec2f,b:vec2f)->vec2f{return film_mul(a,vec2f(b.x,-b.y))/max(dot(b,b),1e-30);}
fn film_sqrt(z:vec2f)->vec2f{let r=length(z);return vec2f(sqrt(max(0.0,(r+z.x)*0.5)),select(-1.0,1.0,z.y>=0.0)*sqrt(max(0.0,(r-z.x)*0.5)));}
// Airy sum for one lossless dielectric layer over a complex-index substrate.
fn film_reflectance(cosine:f32,wavelength:f32,thickness:f32)->f32{return film_between(cosine,wavelength,thickness,1.0,vec2f(u.phenomena.film.w,u.phenomena.film_extra.x));}
fn film_between(cosine:f32,wavelength:f32,thickness:f32,n0:f32,n2:vec2f)->f32{
 let c0=clamp(abs(cosine),1e-6,1.0);let s0=1.0-c0*c0;let n1=u.phenomena.film.z+u.phenomena.film_extra.z/pow(wavelength*0.001,2.0);
 if(n2.y==0.0&&n0*n0*s0>=n2.x*n2.x){return 1.0;}
 let c1=film_sqrt(vec2f(1.0-n0*n0*s0/(n1*n1),0.0));let c2=film_sqrt(vec2f(1.0,0.0)-film_div(vec2f(n0*n0*s0,0.0),film_mul(n2,n2)));
 let q1=c1*n1;let q2=film_mul(n2,c2);let delta=c1*(4.0*PI*n1*max(thickness,0.0)/wavelength);let phase=exp(-delta.y)*vec2f(cos(delta.x),sin(delta.x));var sum=0.0;
 for(var pol=0u;pol<2u;pol++){var a=film_div(vec2f(n0*c0,0.0)-q1,vec2f(n0*c0,0.0)+q1);var b=film_div(q1-q2,q1+q2);if(pol==1u){a=film_div(vec2f(n1*c0,0.0)-c1*n0,vec2f(n1*c0,0.0)+c1*n0);b=film_div(film_mul(n2,c1)-c2*n1,film_mul(n2,c1)+c2*n1);}let amplitude=film_div(a+film_mul(b,phase),vec2f(1.0,0.0)+film_mul(film_mul(a,b),phase));sum+=dot(amplitude,amplitude);}
 return clamp(0.5*sum,0.0,1.0);
}
fn spectral_gaussian(w:f32,center:f32,left:f32,right:f32)->f32{let t=(w-center)*select(right,left,w<center);return exp(-0.5*t*t);}
fn spectral_xyz(w:f32)->vec3f{
 return vec3f(0.362*spectral_gaussian(w,442.0,0.0624,0.0374)+1.056*spectral_gaussian(w,599.8,0.0264,0.0323)-0.065*spectral_gaussian(w,501.1,0.0490,0.0382),0.821*spectral_gaussian(w,568.8,0.0213,0.0247)+0.286*spectral_gaussian(w,530.9,0.0613,0.0322),1.217*spectral_gaussian(w,437.0,0.0845,0.0278)+0.681*spectral_gaussian(w,459.0,0.0385,0.0725));
}
fn spectral_rgb(v:vec3f)->vec3f{return vec3f(3.2406*v.x-1.5372*v.y-0.4986*v.z,-0.9689*v.x+1.8758*v.y+0.0415*v.z,0.0557*v.x-0.2040*v.y+1.0570*v.z);}
fn film_rgb(cosine:f32,thickness:f32)->vec3f{return film_rgb_between(cosine,thickness,1.0,vec2f(u.phenomena.film.w,u.phenomena.film_extra.x));}
fn film_rgb_between(cosine:f32,thickness:f32,n0:f32,n2:vec2f)->vec3f{
 var xyz=vec3f(0.0);var white=vec3f(0.0);for(var i=0u;i<41u;i++){let w=385.0+f32(i)*10.0;let response=spectral_xyz(w);white+=response;xyz+=response*film_between(cosine,w,thickness,n0,n2);}return clamp(spectral_rgb(xyz)/spectral_rgb(white),vec3f(0.0),vec3f(1.0));
}
fn dispersed_ior(n550:f32,wavelength:f32)->f32{if(u.phenomena.outputs.z==0u){return n550;}return max(1.0,n550+u.phenomena.film_extra.w*(1.0/pow(wavelength*0.001,2.0)-1.0/(0.55*0.55)));}
// A pixel-scrambled cycle gives every band equal weight in each three-sample
// block. Absolute sample indices preserve it across tiles, dispatches and resume.
fn spectral_channel(pixel:vec2u,sample:u32)->u32{
    // Self-contained: probes and the control machine do not include path RNG.
    var hash=pixel.x^(pixel.y*0x9e3779b9u)^u.seed^0x68bc21ebu;
    hash=(hash^(hash>>16u))*0x7feb352du;
    hash=(hash^(hash>>15u))*0x846ca68bu;
    let offset=(hash^(hash>>16u))%3u;
    return (sample%3u+offset)%3u;
}


// Deterministic midpoint quadrature of a bounded, self-emitting absorbing field.
// Radiance is additive and w carries Beer-Lambert transmission.
fn nature_volume(ro:vec3f,rd:vec3f,end:f32)->vec4f{
    if(!NATURE_VOLUME_ENABLED||!NATURE_FIELDS_ENABLED||(u.phenomena.outputs.x&4u)==0u||u.phenomena.grid.y==0u){return vec4f(0.0,0.0,0.0,1.0);}
    let inverse=vec3f(select(1e20,1.0/rd.x,abs(rd.x)>1e-12),select(1e20,1.0/rd.y,abs(rd.y)>1e-12),select(1e20,1.0/rd.z,abs(rd.z)>1e-12));
    let a=(u.phenomena.center.xyz-u.phenomena.half.xyz-ro)*inverse;
    let b=(u.phenomena.center.xyz+u.phenomena.half.xyz-ro)*inverse;
    let near=min(a,b);let far=max(a,b);
    let begin=max(0.0,max(max(near.x,near.y),near.z));let finish=min(end,min(min(far.x,far.y),far.z));
    if(finish<=begin){return vec4f(0.0,0.0,0.0,1.0);}
    let cells=u.phenomena.grid.yzw;let count=max(u.phenomena.outputs.w,clamp(max(max(cells.x,cells.y),cells.z)*2u,8u,256u));
    let dt=(finish-begin)/f32(count);var light=vec3f(0.0);var transmission=1.0;
    for(var i=0u;i<count;i++){
        let field=nature_field(ro+rd*(begin+(f32(i)+0.5)*dt));
        let density=max(0.0,field.a.z);let sigma=density*u.phenomena.gains.w;
        let attenuation=exp(-sigma*dt);var integral=dt;
        if(sigma>1e-6){integral=(1.0-attenuation)/sigma;}
        let excitation=select(1.0,field.a.y,(u.phenomena.outputs.x&2u)!=0u);
        light+=transmission*field.color*density*excitation*u.phenomena.gains.z*integral;
        transmission*=attenuation;if(transmission<1e-5){break;}
    }
    return vec4f(light,transmission);
}

// A short geodesic chord uses one midpoint. Curvature subdivision controls its
// spatial integration error, so the volume is never integrated on a false ray.
fn nature_chord(a:vec3f,b:vec3f)->vec4f {
    if(!NATURE_VOLUME_ENABLED||!NATURE_FIELDS_ENABLED||(u.phenomena.outputs.x&4u)==0u){return vec4f(0.0,0.0,0.0,1.0);}
    let field=nature_field((a+b)*0.5);if(!field.inside){return vec4f(0.0,0.0,0.0,1.0);}
    let dt=length(b-a);let density=max(0.0,field.a.z);let sigma=density*u.phenomena.gains.w;
    let transmission=exp(-sigma*dt);var integral=dt;if(sigma>1e-6){integral=(1.0-transmission)/sigma;}
    let excitation=select(1.0,field.a.y,(u.phenomena.outputs.x&2u)!=0u);
    return vec4f(field.color*density*excitation*u.phenomena.gains.z*integral,transmission);
}
