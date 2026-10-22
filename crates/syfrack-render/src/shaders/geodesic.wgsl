const KERR_ENABLED: bool = true;
// Null Hamiltonian integration in stationary Cartesian Kerr-Schild coordinates.
// Spatial vectors are in world axes; .w is coordinate time / covariant energy.
struct KerrState { x:vec4f, k:vec4f }
struct KerrMetric { h:f32, l:vec4f }
fn kerr_radius(p:vec3f)->f32 {
    let q=p-u.phenomena.lens_center.xyz;let a=u.phenomena.lens_center.w*u.phenomena.lens_axis.w;
    let z=dot(q,u.phenomena.lens_axis.xyz);let v=dot(q,q)-a*a;
    return sqrt(max(1e-20,0.5*(v+sqrt(v*v+4.0*a*a*z*z))));
}
fn kerr_horizon()->f32 {let s=u.phenomena.lens_axis.w;return u.phenomena.lens_center.w*(1.0+sqrt(1.0-s*s));}
fn kerr_metric(p:vec3f)->KerrMetric {
    let m=u.phenomena.lens_center.w;let axis=u.phenomena.lens_axis.xyz;
    let q=p-u.phenomena.lens_center.xyz;let a=m*u.phenomena.lens_axis.w;
    let r=max(kerr_radius(p),1e-5*max(m,1e-3));let z=dot(q,axis);let transverse=q-axis*z;
    let l=(r*transverse-a*cross(axis,transverse))/(r*r+a*a)+axis*z/r;
    return KerrMetric(m*r*r*r/max(r*r*r*r+a*a*z*z,1e-30),vec4f(l,-1.0));
}
fn kerr_hamiltonian(p:vec3f,k:vec4f)->f32 {let g=kerr_metric(p);let v=dot(g.l,k);return 0.5*(dot(k.xyz,k.xyz)-k.w*k.w-2.0*g.h*v*v);}
fn kerr_launch(p:vec3f,d:vec3f)->KerrState {
    let g=kerr_metric(p);let inv=sqrt(1.0+2.0*g.h);let physical=normalize(d)+g.l.xyz*dot(g.l.xyz,normalize(d))*(1.0/inv-1.0);
    let spatial=physical+g.l.xyz*(2.0*g.h/inv);let contraction=-inv+dot(g.l.xyz,spatial);
    return KerrState(vec4f(p,0.0),vec4f(spatial+g.l.xyz*(2.0*g.h*contraction),inv+2.0*g.h*contraction));
}
fn kerr_rhs(s:KerrState)->KerrState {
    let g=kerr_metric(s.x.xyz);let velocity=vec4f(s.k.xyz,-s.k.w)-g.l*(2.0*g.h*dot(g.l,s.k));
    let e=max(u.phenomena.lens_center.w*0.0005,0.00001);var gradient=vec3f(0.0);
    for(var axis=0u;axis<3u;axis++){var d=vec3f(0.0);d[axis]=e;gradient[axis]=(kerr_hamiltonian(s.x.xyz+d,s.k)-kerr_hamiltonian(s.x.xyz-d,s.k))/(2.0*e);}
    return KerrState(velocity,vec4f(-gradient,0.0));
}
fn kerr_add(a:KerrState,b:KerrState,t:f32)->KerrState{return KerrState(a.x+b.x*t,a.k+b.k*t);}
fn kerr_advance(s:KerrState,dt:f32)->KerrState {
    let a=kerr_rhs(s);let b=kerr_rhs(kerr_add(s,a,dt*0.5));let c=kerr_rhs(kerr_add(s,b,dt*0.5));let d=kerr_rhs(kerr_add(s,c,dt));
    return KerrState(s.x+(a.x+2.0*b.x+2.0*c.x+d.x)*(dt/6.0),s.k+(a.k+2.0*b.k+2.0*c.k+d.k)*(dt/6.0));
}
fn kerr_step_size(s:KerrState,distance:f32)->f32 {
    let r=kerr_radius(s.x.xyz);let m=u.phenomena.lens_center.w;let speed=max(length(kerr_rhs(s).x.xyz),1e-5);
    // Step shrinks near the horizon and near geometry; distant vacuum is cheap.
    let vacuum=u.phenomena.lens_config.x*max(1.0,r/max(4.0*m,1e-4));
    return max(1e-6,min(vacuum,max(0.0,distance)*0.65/speed));
}
fn kerr_enabled()->bool{return KERR_ENABLED&&u.phenomena.lens_config.w>0.0&&u.phenomena.lens_center.w>0.0;}
