// Integer angle tripling, shared by the specialized graph and fixed machine.
// theta = (cos(theta), sin(theta)); phi = (cos(phi), sin(phi)).
// The caller retains the generic small-radius branch below 1e-12.
fn cubic_polar(p:vec3f, r:f32)->vec4f {
    let rho=length(p.xy);
    let t=vec2f(clamp(p.z/r,-1.0,1.0),rho/r);
    var f=vec2f(1.0,0.0);
    if(rho>0.0){f=p.xy/rho;}
    let t2=vec2f(t.x*t.x-t.y*t.y,t.x*t.y+t.y*t.x);
    let f2=vec2f(f.x*f.x-f.y*f.y,f.x*f.y+f.y*f.x);
    let t3=vec2f(t.x*t2.x-t.y*t2.y,t.x*t2.y+t.y*t2.x);
    let f3=vec2f(f.x*f2.x-f.y*f2.y,f.x*f2.y+f.y*f2.x);
    let r2=r*r;
    return vec4f(vec3f(t3.y*f3.x,t3.y*f3.y,t3.x)*(r2*r),3.0*r2);
}
