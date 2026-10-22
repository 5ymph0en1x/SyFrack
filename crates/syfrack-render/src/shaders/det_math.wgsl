// Experimental companion of det_math.rs. DC constants are emitted from the
// shared binary32 table. This file is never included in production transport.
fn det_nan() -> f32 { return bitcast<f32>(0x7fc00000u); }
fn det_sincos(x:f32) -> vec2f {
    if (x==0.0) {return vec2f(x,1.0);}
    if (abs(x)>8192.0 || (bitcast<u32>(x)&0x7f800000u)==0x7f800000u) { return vec2f(det_nan()); }
    let n=floor(x*DC0+0.5); let r=fma(-n,DC2,fma(-n,DC1,x)); let z=r*r;
    var s=DC3; s=fma(s,z,DC4); s=fma(s,z,DC5); s=fma(s,z,DC6); s=fma(s,z,DC7);
    s=fma(r*z,s,r);
    var c=DC8; c=fma(c,z,DC9); c=fma(c,z,DC10); c=fma(c,z,DC11); c=fma(c,z,DC12);
    c=fma(z,c,1.0);
    switch (i32(n)&3) { case 0: {return vec2f(s,c);} case 1: {return vec2f(c,-s);}
        case 2: {return vec2f(-s,-c);} default: {return vec2f(-c,s);} }
}
fn det_sin(x:f32)->f32 { return det_sincos(x).x; }
fn det_cos(x:f32)->f32 { return det_sincos(x).y; }
fn det_atan2(y:f32,x:f32)->f32 {
    if ((bitcast<u32>(x)&0x7f800000u)==0x7f800000u || (bitcast<u32>(y)&0x7f800000u)==0x7f800000u) { return det_nan(); }
    let ax=abs(x); let ay=abs(y); var r=0.0;
    if (max(ax,ay)>0.0) {r=min(ax,ay)/max(ax,ay);}
    let reduced=r>DC16; if (reduced) {r=(r-1.0)/(r+1.0);}
    let z=r*r; var a=DC17; a=fma(a,z,DC18); a=fma(a,z,DC19); a=fma(a,z,DC20);
    a=fma(a,z,DC21); a=fma(a,z,DC22); a=fma(r*z,a,r);
    if (reduced) {a+=DC13;} if (ay>ax) {a=DC14-a;}
    if ((bitcast<u32>(x)>>31u)!=0u) {a=DC15-a;}
    return bitcast<f32>(bitcast<u32>(a)^(bitcast<u32>(y)&0x80000000u));
}
fn det_log2(x:f32)->f32 {
    if (x==0.0) {return bitcast<f32>(0xff800000u);}
    let bits=bitcast<u32>(x);
    if (x<0x1p-126f || (bits&0x7f800000u)==0x7f800000u) {return det_nan();}
    var e=f32(i32(bits>>23u)-127); var m=bitcast<f32>((bits&0x7fffffu)|0x3f800000u);
    if (m>DC23) {m*=0.5; e+=1.0;}
    let t=(m-1.0)/(m+1.0); let z=t*t;
    var p=DC25; p=fma(p,z,DC26); p=fma(p,z,DC27); p=fma(p,z,DC28);
    p=fma(p,z,DC29); p=fma(p,z,1.0); return fma(DC24*t,p,e);
}
fn det_exp2(x:f32)->f32 {
    if (x>=128.0) {return bitcast<f32>(0x7f800000u);}
    if (x < -126.0 || (bitcast<u32>(x)&0x7f800000u)==0x7f800000u) {return det_nan();}
    let n=floor(x); let r=(x-n)*DC30;
    var p=DC4; p=fma(p,r,DC9); p=fma(p,r,DC31); p=fma(p,r,DC32);
    p=fma(p,r,DC6); p=fma(p,r,DC33); p=fma(p,r,-DC7);
    p=fma(p,r,0.5); p=fma(p,r,1.0); p=fma(p,r,1.0);
    return p*bitcast<f32>(u32(i32(n)+127)<<23u);
}
fn det_pow(x:f32,y:f32)->f32 {
    if (y==0.0) {return 1.0;} if (x==0.0 && y>0.0) {return 0.0;}
    return det_exp2(y*det_log2(x));
}
