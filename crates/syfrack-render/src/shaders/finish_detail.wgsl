// World-space material detail. xyz is its analytic gradient; w is its height.
// One evaluation supplies colour, roughness and normal detail together.
fn finish_hash(cell: vec3i, seed: u32) -> f32 {
    let c = bitcast<vec3u>(cell);
    let h = pcg(c.x ^ pcg(c.y + 0x9e3779b9u) ^ pcg(c.z + seed));
    return f32(h >> 8u) / 16777216.0;
}
fn finish_noise(p: vec3f, seed: u32) -> vec4f {
    let cell = vec3i(floor(p)); let f = fract(p);
    let t = f*f*(3.0-2.0*f); let dt = 6.0*f*(1.0-f);
    let a0=finish_hash(cell,seed); let a1=finish_hash(cell+vec3i(1,0,0),seed);
    let b0=finish_hash(cell+vec3i(0,1,0),seed); let b1=finish_hash(cell+vec3i(1,1,0),seed);
    let c0=finish_hash(cell+vec3i(0,0,1),seed); let c1=finish_hash(cell+vec3i(1,0,1),seed);
    let d0=finish_hash(cell+vec3i(0,1,1),seed); let d1=finish_hash(cell+vec3i(1,1,1),seed);
    let a=mix(a0,a1,t.x); let b=mix(b0,b1,t.x);
    let c=mix(c0,c1,t.x); let d=mix(d0,d1,t.x);
    let e=mix(a,b,t.y); let g=mix(c,d,t.y);
    let dx=mix(mix(a1-a0,b1-b0,t.y),mix(c1-c0,d1-d0,t.y),t.z);
    return vec4f(vec3f(dx,mix(b-a,d-c,t.z),g-e)*dt,mix(e,g,t.z));
}
fn finish_fbm(p: vec3f, width: f32, seed: u32) -> vec4f {
    var frequency=1.0; var weight=0.5; var result=vec4f(0.0);
    for (var i=0u;i<3u;i++) {
        let visible=1.0-smoothstep(0.35,0.85,width*frequency);
        if (visible>0.0) {
            let n=finish_noise(p*frequency,seed+i*97u);
            result+=weight*vec4f(n.xyz*frequency*visible,mix(0.5,n.w,visible));
        } else { result.w+=weight*0.5; }
        frequency*=2.13; weight*=0.5;
    }
    return result/0.875;
}
fn finish_smooth_derivative(a: f32,b: f32,x: f32) -> f32 {
    let t=clamp((x-a)/(b-a),0.0,1.0);
    return 6.0*t*(1.0-t)/(b-a);
}
fn finish_eval(f: Finish,p: vec3f,footprint: f32) -> vec4f {
    let scale=f.detail_mapping.x;
    let q=p*scale; let width=footprint*scale; let seed=f.detail_kind.y;
    if (f.detail_kind.x==12u) {
        // Broad depressions fill with rain; porous high spots remain rough.
        let basin=finish_fbm(q*vec3f(0.31,0.31,0.22),width*0.31,seed+911u);
        let wet=smoothstep(0.47,0.59,basin.w);
        let grit=finish_fbm(q*16.0,width*16.0,seed+113u);
        let visible=1.0-smoothstep(0.15,0.6,width*3.0);
        let phase=dot(q,vec3f(2.8,0.0,4.1));
        let ripple=vec3f(2.8,0.0,4.1)*cos(phase)*0.018*visible;
        // Gradient is a shading slope: water suppresses the substrate relief.
        return vec4f(mix(grit.xyz*16.0,ripple,wet)*scale,wet);
    }
    if (f.detail_kind.x==10u || f.detail_kind.x==11u) {
        // Centimetre aggregate, pinholes, metre-scale stains and board marks.
        // Each band has its own footprint; distant concrete never sparkles.
        let stains=finish_fbm(q*0.14,width*0.14,seed+17u);
        let aggregate=finish_fbm(q*7.0,width*7.0,seed+113u);
        let pore=finish_noise(q*23.0,seed+307u);
        let visible=1.0-smoothstep(0.25,0.75,width*23.0);
        let pit=1.0-smoothstep(0.24,0.43,pore.w);
        let pit_gradient=-finish_smooth_derivative(0.24,0.43,pore.w)*23.0*pore.xyz*visible;
        let phase=fract(q.y/1.25+0.5)-0.5;
        let board_visible=1.0-smoothstep(0.03,0.15,width);
        let joint=(1.0-smoothstep(0.006,0.027,abs(phase)))*board_visible;
        let joint_gradient=-finish_smooth_derivative(0.006,0.027,abs(phase))*sign(phase)/1.25*board_visible;
        let height=0.18*stains.w+0.65*aggregate.w-0.14*mix(0.2,pit,visible)-0.09*joint;
        let gradient=(0.18*0.14*stains.xyz+0.65*7.0*aggregate.xyz-0.14*pit_gradient-vec3f(0.0,0.09*joint_gradient,0.0))*scale;
        if (f.detail_kind.x==11u) {
            let axes=vec3f(1.9,0.055,1.9);
            let rain=finish_fbm(q*axes,width*1.9,seed+601u);
            let deposit=smoothstep(0.33,0.68,rain.w);
            let rain_gradient=rain.xyz*axes*scale*finish_smooth_derivative(0.33,0.68,rain.w);
            return vec4f(gradient+0.12*rain_gradient,
                clamp(0.72*deposit+0.28*stains.w,0.0,1.0));
        }
        return vec4f(gradient,clamp(height+0.12,0.0,1.0));
    }
    if (f.detail_kind.x==9u) {
        // Domain warping breaks up the deposits without following slab UVs.
        // Keep the Jacobian: all bump derivatives are analytic, even in the warp.
        let wa=finish_fbm(q*0.17,width*0.17,seed+17u);
        let wb=finish_fbm(q*0.17,width*0.17,seed+53u);
        let warped=q+2.8*vec3f(wa.w-0.5,0.0,wb.w-0.5);
        // A conservative footprint also filters the stretched parts of the warp.
        let cloud=finish_fbm(warped*0.16,width*0.48,seed+211u);
        let axes=vec3f(0.65,0.18,0.09);
        let streak=finish_fbm(warped*axes,width*1.95,seed+397u);
        let density=0.78*cloud.w+0.22*streak.w;
        let g=0.78*0.16*cloud.xyz+0.22*axes*streak.xyz;
        let density_gradient=(g+2.8*0.17*(g.x*wa.xyz+g.z*wb.xyz))*scale;
        let deposit=smoothstep(0.36,0.64,density);
        let deposit_gradient=density_gradient*finish_smooth_derivative(0.36,0.64,density);
        let grain=finish_fbm(p*48.0,footprint*48.0,seed+739u);
        let powder=0.85+0.15*grain.w;
        return vec4f(deposit_gradient*powder+deposit*0.15*48.0*grain.xyz,
            deposit*powder);
    }
    if (f.detail_kind.x==8u) {
        let wind=u.atmosphere.dunes.z;
        let xz=rot2(p.xz,wind)*scale;
        let warp=finish_noise(vec3f(xz.x*0.045,0.0,xz.y*0.045),seed);
        let phase=2.0*PI*(xz.x+1.7*warp.w+0.12*sin(xz.y*0.22));
        let visible=exp(-pow(width*3.0,2.0));
        let ripple=0.5+0.36*sin(phase)+0.10*sin(2.0*phase);
        let gradient=2.0*PI*(0.36*cos(phase)+0.20*cos(2.0*phase))
            *(vec2f(1.0,0.0264*cos(xz.y*0.22))+0.0765*warp.xz)*scale*visible;
        let grain=finish_fbm(p*90.0,footprint*90.0,seed+139u);
        let grad=rot2(gradient,-wind);
        return vec4f(vec3f(grad.x,0.0,grad.y)+grain.xyz*2.5,
            0.70*mix(0.5,ripple,visible)+0.30*grain.w);
    }
    if (f.detail_kind.x==6u) {
        let tile=fract(q.xz+vec2f(0.5))-vec2f(0.5);
        let r=max(length(tile),1e-5); let angle=atan2(tile.y,tile.x);
        let flower=r-0.25-0.07*cos(angle*8.0); let ring=r-0.45;
        let angular=vec2f(-tile.y,tile.x)/(r*r);
        var grad2=sign(flower)*(tile/r+0.56*sin(angle*8.0)*angular);
        var distance=abs(flower);
        if (abs(ring)<distance) { distance=abs(ring); grad2=sign(ring)*tile/r; }
        let visible=1.0-smoothstep(0.10,0.40,width);
        let carving=1.0-smoothstep(0.012,0.045,distance);
        grad2*=-finish_smooth_derivative(0.012,0.045,distance)*visible*scale;
        return vec4f(grad2.x,0.0,grad2.y,mix(0.2,carving,visible));
    }
    if (f.detail_kind.x==3u) {
        let axes=vec3f(18.0,0.45,3.0);
        let n=finish_fbm(q*axes,width*18.0,seed);
        return vec4f(n.xyz*axes*scale,n.w);
    }
    let n=finish_fbm(q,width,seed); var result=n;
    switch f.detail_kind.x {
        case 1u: {
            let axis=vec3f(0.16,0.11,0.08);
            let phase=(dot(q,axis)+n.w*2.8)*PI*2.0;
            let wave=sin(phase);
            let veins=1.0-smoothstep(0.035,0.19,abs(wave));
            let gradient=-finish_smooth_derivative(0.035,0.19,abs(wave))*sign(wave)*cos(phase)*PI*2.0*(axis+n.xyz*2.8);
            let visible=1.0-smoothstep(0.18,0.45,width);
            result=mix(n,0.65*n+0.35*vec4f(gradient,veins),visible);
        }
        case 2u: {
            result=vec4f(n.xyz*finish_smooth_derivative(0.35,0.67,n.w),smoothstep(0.35,0.67,n.w));
        }
        case 4u, 7u: {
            let grain=finish_fbm(q*3.7,width*3.7,seed+53u);
            result=0.8*n+0.2*vec4f(grain.xyz*3.7,grain.w);
        }
        case 5u: {
            let a=q.x+0.75*n.w; let b=q.z*1.37+q.x*0.4;
            let visible=1.0-smoothstep(0.35,1.0,width);
            let gradient=(0.32*cos(a)*(vec3f(1.0,0.0,0.0)+0.75*n.xyz)+0.18*cos(b)*vec3f(0.4,0.0,1.37))*visible;
            result=vec4f(gradient,0.5+(0.32*sin(a)+0.18*sin(b))*visible);
        }
        default: {}
    }
    return vec4f(result.xyz*scale,result.w);
}
fn finish_normal(f: Finish,n: vec3f,gradient: vec3f) -> vec3f {
    let slope=gradient-n*dot(gradient,n);
    let limit=0.75/max(length(slope),1e-5);
    return safe_normalize(n-slope*clamp(f.detail_mapping.z,-limit,limit));
}
