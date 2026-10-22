// Fixed, typed field bytecode, common to the live machine and final renderer.
fn creation_planar(at:u32)->PlanarFractal {
    return PlanarFractal(vec4u(gp_row(at)),gp_row(at+1u),gp_row(at+2u),gp_row(at+3u),gp_row(at+4u));
}
fn creation_palette(at:u32)->Palette {
    var palette:Palette;palette.kind=vec4u(gp_row(at));palette.mapping=gp_row(at+1u);
    for(var i=0u;i<16u;i++){palette.stops[i]=gp_row(at+2u+i);}return palette;
}
fn creation_texture(at:u32)->FractalTexture {
    return FractalTexture(vec4u(gp_row(at)),gp_row(at+1u),gp_row(at+2u),gp_row(at+3u),
        creation_palette(at+4u),gp_row(at+22u),gp_row(at+23u),gp_row(at+24u),gp_row(at+25u),gp_row(at+26u),creation_planar(at+27u));
}
fn creation_grid(metadata:vec4f,p:vec3f,zero_outside:bool)->vec4f {
    let uv=(p-u.phenomena.center.xyz+u.phenomena.half.xyz)/(2.0*u.phenomena.half.xyz);
    if(zero_outside&&(any(uv<vec3f(0.0))||any(uv>vec3f(1.0)))){return vec4f(0.0);}
    let r=vec3u(metadata.yzw);let span=r-vec3u(1u);let q=clamp(uv,vec3f(0.0),vec3f(1.0))*vec3f(span);
    let low=vec3u(floor(q));let high=min(low+vec3u(1u),span);let f=fract(q);var result=vec4f(0.0);
    for(var z=0u;z<select(2u,1u,span.z==0u);z++){for(var y=0u;y<2u;y++){for(var x=0u;x<2u;x++){
        let c=vec3u(select(low.x,high.x,x==1u),select(low.y,high.y,y==1u),select(low.z,high.z,z==1u));
        let w=select(1.0-f.x,f.x,x==1u)*select(1.0-f.y,f.y,y==1u)*select(1.0-f.z,f.z,z==1u);
        result+=gp_row(u32(metadata.x)+(c.z*r.y+c.y)*r.x+c.x)*w;
    }}}return result;
}
fn creation_flame(at:u32,p:vec3f)->vec4f {
    let atlas=gp_row(at);let frame=gp_row(at+1u);let angle=gp_row(at+2u);
    let delta=p.xy-frame.xy;let uv=vec2f(angle.y*delta.x+angle.x*delta.y,-angle.x*delta.x+angle.y*delta.y)/(2.0*frame.zw)+vec2f(0.5);
    if(any(uv<vec2f(0.0))||any(uv>vec2f(1.0))){return vec4f(0.0);}
    let r=u32(atlas.y);let q=clamp(vec2f(uv.x,1.0-uv.y)*f32(r)-vec2f(0.5),vec2f(0.0),vec2f(f32(r-1u)));
    let low=vec2u(floor(q));let high=min(low+vec2u(1u),vec2u(r-1u));let f=fract(q);let offset=u32(atlas.x);
    let v=mix(mix(gp_row(offset+low.y*r+low.x),gp_row(offset+low.y*r+high.x),f.x),
        mix(gp_row(offset+high.y*r+low.x),gp_row(offset+high.y*r+high.x),f.x),f.y);
    if(angle.z==1.0){return vec4f(v.w,0.0,0.0,0.0);}return vec4f(v.xyz,0.0);
}
fn creation_complex(a:vec2f,b:vec2f)->vec2f{return vec2f(a.x*b.x-a.y*b.y,a.x*b.y+a.y*b.x);}
fn creation_register(values:ptr<function,array<vec4f,64>>,index:f32)->vec4f {
    if(index<0.0){return vec4f(0.0);}return (*values)[u32(index)];
}
fn creation_field(p:vec3f,base:NatureValue)->NatureValue {
    var registers:array<vec4f,64>;
    for(var i=0u;i<u.phenomena.program.y;i++){
        let command=gp_row(u.phenomena.program.x+i*2u);let at=u32(gp_row(u.phenomena.program.x+i*2u+1u).x);
        let a=creation_register(&registers,command.y);let b=creation_register(&registers,command.z);let c=creation_register(&registers,command.w);
        let data=gp_row(at);var v=vec4f(0.0);
        switch u32(command.x) {
            case 0u:{v=vec4f(p,0.0);}
            case 1u,2u:{v=data;}
            case 3u:{v=a+b;}case 4u:{v=a-b;}case 5u:{v=a*b;}case 6u:{v=a*b.x;}
            case 7u:{v.x=dot(a.xyz,b.xyz);}case 8u:{v=vec4f(cross(a.xyz,b.xyz),0.0);}
            case 9u:{v.x=length(a.xyz);}case 10u:{let len=length(a.xyz);if(len>0.0){v=vec4f(a.xyz/len,0.0);}}
            case 11u:{v.x=sin(a.x);}case 12u:{v.x=cos(a.x);}case 13u:{v.x=exp(a.x);}case 14u:{v.x=abs(a.x);}
            case 15u:{v.x=clamp(a.x,data.x,data.y);}case 16u:{v.x=pow(a.x,data.x);}
            case 17u:{v=vec4f(creation_complex(a.xy,b.xy),0.0,0.0);}
            case 18u:{var z=vec2f(1.0,0.0);for(var j=0u;j<u32(data.x);j++){z=creation_complex(z,a.xy);}v=vec4f(z,0.0,0.0);}
            case 19u:{v.x=atan2(a.y,a.x)/6.2831853+0.5;}
            case 20u:{v.x=dot(a.xy,a.xy);}
            case 21u:{let settings=gp_row(at+1u);let phase=dot(data.xyz,a.xyz)-data.w*settings.y+settings.x;v=vec4f(cos(phase),sin(phase),0.0,0.0);}
            case 22u:{let wave=gp_row(at+1u);let q=a.xyz-data.xyz;let phase=dot(wave.xyz,q)-wave.w*gp_row(at+2u).x;
                v=vec4f(exp(-dot(q,q)/(2.0*data.w*data.w))*vec2f(cos(phase),sin(phase)),0.0,0.0);}
            case 23u:{let sample=creation_grid(data,a.xyz,true);v.x=sample[u32(gp_row(at+1u).x)];}
            case 24u:{let model=creation_planar(at);let sample=planar_sample(model,a.xy);let weights=gp_row(at+5u);let extra=gp_row(at+6u);
                v.x=dot(weights,sample.traps)+extra.x*f32(sample.iterations)/f32(max(model.kind.z,1u))+extra.y*0.5;
                if(extra.z!=0.0){v.x=sign(v.x)*log(1.0+extra.z*abs(v.x))/log(1.0+extra.z);}}
            case 25u:{v=creation_flame(at,a.xyz);}
            case 26u:{v=vec4f(palette_sample(creation_palette(at),a.x),0.0);}
            case 27u:{v=mix(a,b,clamp(c.x,0.0,1.0));}
            case 28u:{v.x=dot(a.xyz,vec3f(0.2126,0.7152,0.0722));}
            case 29u:{let second=gp_row(at+1u);let numerator=creation_complex(data.xy,a.xy)+data.zw;
                let denominator=creation_complex(second.xy,a.xy)+second.zw;
                v=vec4f(creation_complex(numerator,vec2f(denominator.x,-denominator.y))/max(dot(denominator,denominator),1e-20),0.0,0.0);}
            case 30u:{v=vec4f(a.x,b.x,0.0,0.0);}
            case 31u:{v.x=distance(a.xyz,data.xyz)-data.w;}
            case 32u:{v.x=pattern_value(creation_texture(at),a.xyz);}
            case 50u:{v=creation_grid(data,p,false);}
            default:{}
        }
        if(!all(abs(v)<vec4f(1e30))){v=vec4f(0.0);}registers[i]=v;
    }
    let a=gp_row(u.phenomena.program.z);let b=gp_row(u.phenomena.program.z+1u);var result=base;
    result.a.x=creation_register(&registers,a.x).x;result.a.y=max(0.0,creation_register(&registers,a.y).x);
    result.a.z=max(0.0,creation_register(&registers,a.z).x);result.b.y=creation_register(&registers,b.y).x;
    result.color=palette_sample(u.phenomena.palette,result.a.x);
    if(b.z>=0.0){result.color=max(creation_register(&registers,b.z).xyz,vec3f(0.0));}
    return result;
}
