// Bounded emission/absorption quadrature, with a fractal density field in 3D.
// No extra path bounces or scattering noise. Midpoint integration is stable
// across tiles/checkpoints; pixel footprints suppress unresolved octaves.
@group(0) @binding(7) var particle_density: texture_3d<f32>;
@group(0) @binding(8) var volume_sampler: sampler;
@group(0) @binding(9) var particle_lighting: texture_3d<f32>;
@group(0) @binding(10) var nebular_microstructure: texture_3d<f32>;
@group(0) @binding(11) var microstructure_sampler: sampler;

// A periodic 3D atlas contains signed noise gradients and their scalar value.
// The half-texel offset aligns the baked sample positions; repeat filtering
// keeps both the field and its derivatives continuous at the tile boundary.
fn nebula_micro_noise(p:vec3f) -> vec4f {
    return textureSampleLevel(nebular_microstructure,microstructure_sampler,(p+0.125)/16.0,0.0);
}

// Filter a tube (intersection of two scalar bands) over the pixel cone AND
// the integration segment. Covariance retains correlated gradients; no
// unresolved noise is sharpened into scintillation or camera-facing sheets.
fn nebula_micro_tube(a:vec4f,b:vec4f,pixel:f32,segment:vec3f,width:f32) -> f32 {
    let direction=segment/max(length(segment),1e-8);
    let pa=dot(a.xyz,direction);let pb=dot(b.xyz,direction);
    let variance=width*width;
    let aa=variance+(pixel*pixel*max(0.0,dot(a.xyz,a.xyz)-pa*pa)+pow(dot(a.xyz,segment),2.0))/12.0;
    let bb=variance+(pixel*pixel*max(0.0,dot(b.xyz,b.xyz)-pb*pb)+pow(dot(b.xyz,segment),2.0))/12.0;
    let ab=(pixel*pixel*(dot(a.xyz,b.xyz)-pa*pb)+dot(a.xyz,segment)*dot(b.xyz,segment))/12.0;
    let determinant=max(aa*bb-ab*ab,variance*variance);
    let x=a.w-0.5;let y=b.w-0.5;
    return variance/sqrt(determinant)*exp(-0.5*max(0.0,bb*x*x-2.0*ab*x*y+aa*y*y)/determinant);
}

fn nebula_spectral_medium(p:vec3f,pixel:f32,segment:vec3f,volume:vec4f) -> NebulaMedium {
    let gas=volume.x*u.nebula.extent.w;let dust=volume.y*u.nebula.warm.w;
    if (gas<1e-10) {return NebulaMedium(vec3f(0.0),dust,dust*0.15);}
    let warmth=clamp(volume.z/max(volume.x,1e-10),0.0,1.0);
    let excited=max(volume.w,0.0)*u.nebula.extent.w;
    let strength=f32(u.nebula.quality.z>>1u)/65535.0;
    let scale=vec3f(0.85,0.90,0.38)*u.nebula.settings.y*0.28;
    let shift=vec3f(f32(pcg(u.nebula.quality.x+31u)>>8u),
        f32(pcg(u.nebula.quality.x+71u)>>8u),f32(pcg(u.nebula.quality.x+113u)>>8u))/1048576.0;
    let q=p*scale+shift;
    let wx=nebula_micro_noise(q*0.42);
    let wy=nebula_micro_noise(q*0.42+vec3f(5.7,9.1,2.8));
    let wz=nebula_micro_noise(q*0.42+vec3f(11.3,3.2,7.6));
    let warp=q+vec3f(wx.w-0.5,wy.w-0.5,wz.w-0.5)*1.4;
    let jacobian=mat3x3f(vec3f(1.0,0.0,0.0)+vec3f(wx.x,wy.x,wz.x)*0.588,
        vec3f(0.0,1.0,0.0)+vec3f(wx.y,wy.y,wz.y)*0.588,
        vec3f(0.0,0.0,1.0)+vec3f(wx.z,wy.z,wz.z)*0.588);
    let compression=smoothstep(0.25,1.4,excited/max(gas,1e-8));
    var frequency=1.0;var weight=1.0;var threads=0.0;var sharp=0.0;var knots=0.0;var total=0.0;
    for (var octave=0u;octave<3u;octave++) {
        var a=nebula_micro_noise(warp*frequency+vec3f(3.1,7.6,12.3)*f32(octave));
        var b=nebula_micro_noise(warp*frequency+vec3f(7.9,2.3,11.2)+vec3f(6.3,11.1,4.7)*f32(octave));
        a=vec4f(transpose(jacobian)*a.xyz*frequency*scale,a.w);
        b=vec4f(transpose(jacobian)*b.xyz*frequency*scale,b.w);
        threads+=weight*nebula_micro_tube(a,b,pixel,segment,0.055);
        let narrow=nebula_micro_tube(a,b,pixel,segment,0.028);
        // A third band interrupts the collision filaments into compact knots.
        // Their width also includes the pixel/segment footprint, so they do
        // not turn into unresolved point highlights when the camera recedes.
        let c=nebula_micro_noise(warp*frequency+vec3f(12.7,4.9,8.1));
        let gradient=transpose(jacobian)*c.xyz*frequency*scale;
        let direction=segment/max(length(segment),1e-8);
        let projected=dot(gradient,direction);
        let variance=0.08*0.08+(pixel*pixel*max(0.0,dot(gradient,gradient)-projected*projected)
            +pow(dot(gradient,segment),2.0))/12.0;
        let node=0.08/sqrt(variance)*exp(-0.5*pow(c.w-0.5,2.0)/variance);
        sharp+=weight*narrow;knots+=weight*narrow*node;
        total+=weight;frequency*=2.07;weight*=0.55;
    }
    // Bounded emissive redistribution, independent of the absorption grid.
    // A residual diffuse component keeps this a gaseous medium, not a mesh.
    let filament=clamp(threads/(total*0.10),0.0,10.0);
    let collision_filament=clamp(sharp/(total*0.028),0.0,20.0);
    let collision_knots=clamp(knots/(total*0.010),0.0,28.0);
    let ionization=clamp(excited/max(gas*4.0,1e-8),0.0,1.0);
    let hydrogen=pow(warmth,1.8);
    let oxygen=pow(1.0-warmth,1.5)*(0.75+0.25*ionization);
    let sulfur=hydrogen*(1.0-ionization)*smoothstep(0.45,0.7,wx.w)*0.35;
    // Synthetic, narrow-band-inspired display colours. These weights are
    // morphology proxies, not solved ionic abundances or telescope data.
    let line_color=(hydrogen*mix(u.nebula.warm.xyz,vec3f(1.0,0.11,0.025),0.35)
        +oxygen*mix(u.nebula.cool.xyz,vec3f(0.015,0.75,0.90),0.30)
        +sulfur*vec3f(1.0,0.48,0.06))/max(0.65,hydrogen+oxygen+sulfur);
    let old_color=mix(u.nebula.cool.xyz,u.nebula.warm.xyz,warmth);
    let color=mix(old_color,line_color,strength);
    let hot_color=mix(mix(old_color,vec3f(0.85,0.93,1.0),0.12),
        mix(line_color,vec3f(0.75,0.94,1.0),0.06),strength);
    let diffuse=(0.12+0.88*filament)*(1.0-0.55*compression);
    let collision=mix(0.18+0.82*filament,
        0.035+0.60*collision_filament+0.365*collision_knots,compression);
    let source=u.nebula.cool.w*(gas*color*0.46*mix(1.0,diffuse,strength)
        +excited*hot_color*mix(1.0,collision,strength));
    let gas_opacity=u.nebula.settings.x;
    return NebulaMedium(source,gas*(0.08+u.nebula.scattering.x)*gas_opacity+dust,
        gas*u.nebula.scattering.x*gas_opacity+dust*0.15);
}

fn particle_uv(p:vec3f) -> vec3f {return (p/u.nebula.extent.xyz+1.0)*0.5;}
fn nebula_local(p: vec3f) -> vec3f {
    return vec3f(rot2(p.xy, -u.nebula.center.w), p.z);
}

fn nebula_fbm(p: vec3f, footprint: f32, seed: u32) -> f32 {
    var q = p; var amplitude = 0.55; var value = 0.0; var weight = 0.0;
    var width = footprint;
    for (var octave = 0u; octave < 6u; octave++) {
        let visible = 1.0 - smoothstep(0.25, 0.8, width);
        if (visible <= 0.0) { value += amplitude * 0.5; }
        else { value += amplitude * mix(0.5, finish_noise(q, seed + octave * 97u).w, visible); }
        weight += amplitude;
        q = q * 2.07 + vec3f(3.7, 7.1, 1.8);
        amplitude *= 0.52; width *= 2.07;
    }
    return value / weight;
}

struct NebulaMedium {
    emission: vec3f,
    extinction: f32,
    scattering: f32,
}

fn nebula_medium(p: vec3f, footprint: f32, segment:vec3f) -> NebulaMedium {
    if (u.nebula.quality.w!=0u) {
        let volume=textureSampleLevel(particle_density,volume_sampler,particle_uv(p),0.0);
        if (u.nebula.quality.w==4u) {return nebula_spectral_medium(p,footprint,segment,volume);}
        let gas=volume.x*u.nebula.extent.w;
        let dust=volume.y*u.nebula.warm.w;
        let warmth=clamp(volume.z/max(volume.x,1e-10),0.0,1.0);
        if (u.nebula.quality.w==3u) {
            // Associated excitation mass: local stream compression and shock
            // heating emit independently of the central star's visibility.
            // Density/opacity stay unchanged when their emissive gain changes.
            let excited=max(volume.w,0.0)*u.nebula.extent.w;
            let color=mix(u.nebula.cool.xyz,u.nebula.warm.xyz,warmth);
            let hot_color=mix(color,vec3f(0.85,0.93,1.0),0.12);
            let source=u.nebula.cool.w*(gas*color*0.46+excited*hot_color);
            let gas_opacity=u.nebula.settings.x;
            return NebulaMedium(source,gas*(0.08+u.nebula.scattering.x)*gas_opacity+dust,
                gas*u.nebula.scattering.x*gas_opacity+dust*0.15);
        }
        if (u.nebula.quality.w==2u) {
            let front=min(volume.w,volume.x)*u.nebula.extent.w;
            let haze=max(0.0,gas-front);
            let visibility=textureSampleLevel(particle_lighting,volume_sampler,particle_uv(p),0.0);
            let delta=p-u.nebula.core.xyz;
            let fluence=u.nebula.settings.z*u.nebula.core.w*u.nebula.core.w/
                (dot(delta,delta)+u.nebula.core.w*u.nebula.core.w);
            // Only the exposed ionized skin re-emits the central radiation.
            // Dense molecular interiors absorb it instead of glowing through
            // the far side. The fourth density channel is associated front mass.
            let excitation=fluence/(1.0+fluence)*visibility.x;
            let skin_color=mix(u.nebula.cool.xyz,u.nebula.warm.xyz,warmth*warmth);
            let haze_color=mix(u.nebula.cool.xyz*0.16,u.nebula.warm.xyz,warmth);
            let source=u.nebula.cool.w*(front*skin_color*excitation*2.5+haze*haze_color*0.6);
            return NebulaMedium(source,gas*(0.08+u.nebula.scattering.x)+dust,
                gas*u.nebula.scattering.x+dust*0.15);
        }
        // Distinct emission bands: a low-excitation transition must not turn
        // blue + orange into a bright grey veil. The associated scalar still
        // filters with density; the palette only controls emitted radiance.
        let color=u.nebula.cool.xyz*pow(1.0-warmth,3.0)+u.nebula.warm.xyz*pow(warmth,3.0);
        let core_delta=(p-u.nebula.core.xyz)/u.nebula.core.w;
        let core=exp(-dot(core_delta,core_delta)*0.65);
        let source=gas*(u.nebula.cool.w*color+core*u.nebula.settings.z*vec3f(1.0,0.55,0.16));
        return NebulaMedium(source,gas*(0.35+u.nebula.scattering.x)+dust,gas*u.nebula.scattering.x);
    }
    let seed = u.nebula.quality.x;
    let r = p / u.nebula.extent.xyz;
    let envelope = 1.0 - smoothstep(0.45, 1.0, dot(r, r));
    if (envelope <= 0.0) { return NebulaMedium(vec3f(0.0),0.0,0.0); }
    let scale = u.nebula.settings.y;
    let q = p * vec3f(0.30, 0.50, 0.045) * scale;
    let warp = vec3f(finish_noise(q * 0.6, seed + 31u).w,
        finish_noise(q * 0.6 + vec3f(11.0), seed + 71u).w,
        finish_noise(q * 0.6 + vec3f(23.0), seed + 113u).w) - 0.5;
    let w = q + u.nebula.settings.x * warp * 3.5;
    let n = nebula_fbm(w, footprint * scale * 0.50, seed);
    let ridge = pow(max(0.0, 1.0 - abs(2.0 * n - 1.0)), 14.0);
    let fine_visibility = 1.0 - smoothstep(0.25, 0.8, footprint * scale * 8.5);
    let fine = mix(0.5, finish_noise(w * 17.0, seed + 211u).w, fine_visibility);
    let threads = ridge * (0.2 + 0.8 * pow(fine, 2.0));
    let gas = envelope * u.nebula.extent.w * (0.008 + 1.8 * threads);
    let lane = p.y - (0.5 * sin(p.x * 0.2) + warp.x * 2.0);
    let dust_lane = exp(-lane * lane * 1.8) * (0.35 + 1.6 * n * n);
    let dark_knots = smoothstep(0.56, 0.71, n);
    let dust = envelope * u.nebula.warm.w * (dust_lane + dark_knots * 0.7)
        * (0.25 + 0.75 * smoothstep(-1.5, 1.0, p.z));
    let to_core = (p - u.nebula.core.xyz) / u.nebula.core.w;
    let core = exp(-dot(to_core, to_core) * 0.65);
    let warm = smoothstep(-4.0, 5.0, p.x) * clamp(0.65 - 0.20 * p.y + 0.4 * warp.y, 0.0, 1.0);
    let color = mix(u.nebula.cool.xyz, u.nebula.warm.xyz, warm);
    let glow = gas * u.nebula.cool.w * color;
    let core_glow = gas * core * u.nebula.settings.z * vec3f(1.0, 0.70, 0.30);
    let scattering = gas*u.nebula.scattering.x;
    return NebulaMedium(glow+core_glow,gas*0.35+dust+scattering,scattering);
}

// Finite stellar points with a Gaussian sensor response. Candidate lookup is
// spatial, so hundreds of cluster stars do not expand the SDF graph. Each star
// is added at its own depth, through the same extinction as the gas/dust.
fn nebula_cluster_segment(origin:vec3f,direction:vec3f,a:f32,b:f32,cone:f32,sigma_t:f32) -> vec3f {
    if (u.nebula.quality.w<2u || u.nebula.scattering.w==0.0 || abs(direction.z)<0.05) {return vec3f(0.0);}
    let mid=origin+direction*((a+b)*0.5);
    if (length(mid.xy-u.nebula.core.xy)>4.5 || abs(mid.z-u.nebula.core.z)>u.nebula.extent.z*0.6+(b-a)) {return vec3f(0.0);}
    let spacing=0.10;
    let cell=vec2i(floor((mid.xy-u.nebula.core.xy)/spacing));
    var sum=vec3f(0.0);
    // A bright foreground member has an approximate telescope PSF: a small
    // Gaussian core and six main diffraction rays plus two weaker horizontal
    // rays. These are sensor/aperture features, not emitting gas filaments.
    let hero=u.nebula.core.xyz+vec3f(-2.4,1.4,u.nebula.extent.z*0.35);
    let hero_t=(hero.z-origin.z)/direction.z;
    if (hero_t>=a && hero_t<b) {
        let offset=(origin+direction*hero_t).xy-hero.xy;
        let pixel_variance=pow(hero_t*cone,2.0)/12.0;
        let variance=0.008*0.008+pixel_variance;
        var psf=exp(-0.5*dot(offset,offset)/variance)*(0.008*0.008/variance);
        let spike_variance=0.0015*0.0015+pixel_variance;
        for (var k=0u;k<4u;k++) {
            let angle=select(PI/6.0+f32(k)*PI/3.0,0.0,k==3u);
            let axis=vec2f(cos(angle),sin(angle));
            let along=dot(offset,axis);let side=dot(offset,vec2f(-axis.y,axis.x));
            psf+=select(0.018,0.006,k==3u)*0.0015/sqrt(spike_variance)
                *exp(-0.5*side*side/spike_variance)/pow(1.0+abs(along)/0.22,2.0);
        }
        sum+=vec3f(1.0,0.91,0.65)*220.0*psf*exp(-sigma_t*(hero_t-a));
    }
    let radius=clamp(i32(ceil(3.0*sqrt(0.012*0.012+pow((a+b)*0.5*cone,2.0)/12.0)/spacing)),1,16);
    for (var y=-radius;y<=radius;y++) {for (var x=-radius;x<=radius;x++) {
        let c=cell+vec2i(x,y);
        let h=pcg(bitcast<u32>(c.x)^pcg(bitcast<u32>(c.y))^u.nebula.quality.x);
        let jitter=vec2f(f32(pcg(h+13u)>>8u),f32(pcg(h+37u)>>8u))/16777216.0;
        let xy=(vec2f(c)+jitter)*spacing+u.nebula.core.xy;
        let r2=dot(xy-u.nebula.core.xy,xy-u.nebula.core.xy);
        let probability=min(1.0,u.nebula.scattering.w*(0.85*exp(-r2/1.35)+0.08*exp(-r2/9.0)));
        if (f32(h>>8u)/16777216.0>probability) {continue;}
        let z=u.nebula.core.z+(f32(pcg(h+59u)>>8u)/16777216.0-0.5)*u.nebula.extent.z;
        let t=(z-origin.z)/direction.z;
        if (t<a || t>=b) {continue;}
        let hit=origin+direction*t;
        let offset=hit.xy-xy;
        let size=0.006+0.006*f32(pcg(h+71u)>>8u)/16777216.0;
        let variance=size*size+(t*cone)*(t*cone)/12.0;
        let psf=exp(-0.5*dot(offset,offset)/variance)*(size*size/variance);
        let bright=f32(pcg(h+83u)>>8u)/16777216.0;
        let color=mix(vec3f(0.28,0.56,1.0),vec3f(0.8,0.9,1.0),bright*bright);
        sum+=color*(7.0+pow(bright,8.0)*55.0)*psf*exp(-sigma_t*(t-a));
    }}
    return sum;
}

// Stable integral of an emitting segment: -expm1(-tau)/tau. At very low
// opacity, subtracting exp(-tau) from 1 can round to zero and erase the gas.
fn nebula_beer_weight(tau:f32) -> f32 {
    if (tau<0.01) {return 1.0-tau*(0.5-tau*(1.0/6.0-tau/24.0));}
    return (1.0-exp(-tau))/tau;
}

// xyz = integrated emission, w = remaining background/surface transmittance.
fn nebula_segment(ro: vec3f, rd: vec3f, limit: f32, pixel_cone: f32) -> vec4f {
    if (!NEBULA_ENABLED || u.nebula.quality.z == 0u || u.nebula.extent.w == 0.0) { return vec4f(0.0, 0.0, 0.0, 1.0); }
    let origin = nebula_local(ro - u.nebula.center.xyz);
    let direction = nebula_local(rd);
    var start=0.0;var end=limit;
    if (u.nebula.quality.w!=0u) {
        // The particle domain is a box. An ellipsoid would cut off transported
        // matter and carve an artificial curved silhouette into the smoke.
        for (var axis=0u;axis<3u;axis++) {
            if (abs(direction[axis])<1e-8) {
                if (abs(origin[axis])>u.nebula.extent[axis]) {return vec4f(0.0,0.0,0.0,1.0);}
            } else {
                let a=(-u.nebula.extent[axis]-origin[axis])/direction[axis];
                let b=(u.nebula.extent[axis]-origin[axis])/direction[axis];
                start=max(start,min(a,b));end=min(end,max(a,b));
            }
        }
    } else {
        let o = origin / u.nebula.extent.xyz;
        let d = direction / u.nebula.extent.xyz;
        let a = dot(d, d); let b = dot(o, d); let c = dot(o, o) - 1.0;
        let discriminant = b * b - a * c;
        if (discriminant <= 0.0) { return vec4f(0.0, 0.0, 0.0, 1.0); }
        let root = sqrt(discriminant);
        start = max(0.0, (-b - root) / a);
        end = min(limit, (-b + root) / a);
    }
    if (start >= end) { return vec4f(0.0, 0.0, 0.0, 1.0); }
    var count = u.nebula.quality.y;
    if (u.nebula.quality.w==4u) {
        // Thin collision structures need longitudinal resolution as well as
        // more output pixels; doubling midpoint samples avoids depth smearing.
        count=min(768u,count*2u);
    }
    if (u.nebula.quality.w!=0u) {
        let voxel=2.0*u.nebula.extent.xyz/vec3f(textureDimensions(particle_density));
        let crossings=(end-start)*max(max(abs(direction.x)/voxel.x,abs(direction.y)/voxel.y),abs(direction.z)/voxel.z);
        count=max(count,min(768u,u32(ceil(crossings))));
    }
    let step = (end - start) / f32(count);
    var emission = vec3f(0.0); var transmittance = 1.0; var optical_depth=0.0;
    for (var i = 0u; i < count; i++) {
        let t = start + (f32(i) + 0.5) * step;
        let medium = nebula_medium(origin + direction * t, t * pixel_cone, direction*step);
        var source = medium.emission;
        if (medium.scattering>0.0 && u.nebula.settings.z>0.0) {
            let p = origin+direction*t;
            let to_core = u.nebula.core.xyz-p;
            let distance = max(length(to_core),1e-4);
            var visibility=vec4f(1.0,1.0,0.0,0.0);
            if (u.nebula.quality.w!=0u) {
                visibility=textureSampleLevel(particle_lighting,volume_sampler,particle_uv(p),0.0);
            } else {
                var optical_depth = 0.0;
                for (var j=0u;j<4u;j++) {optical_depth += nebula_medium(p+to_core*(f32(j)+0.5)/4.0,t*pixel_cone,to_core/4.0).extinction*distance/4.0;}
                visibility.x=exp(-optical_depth);
            }
            let phase = hg_phase(dot(direction,to_core/distance),u.nebula.scattering.y);
            let intensity = u.nebula.settings.z*u.nebula.core.w*u.nebula.core.w
                / (distance*distance+u.nebula.core.w*u.nebula.core.w);
            let core_color=select(vec3f(1.0,0.70,0.30),vec3f(0.85,0.92,1.0),u.nebula.quality.w>=2u);
            source += medium.scattering*core_color*intensity*
                (phase*visibility.x+visibility.z*u.nebula.scattering.z/(4.0*PI));
        }
        if (medium.scattering>0.0 && u.nebula.quality.w!=0u) {
            let p=origin+direction*t;
            let visibility=textureSampleLevel(particle_lighting,volume_sampler,particle_uv(p),0.0);
            // Visibility cache covers the first directional light. Other lights
            // use a short texture-only shadow march, without regenerating noise.
            var first_sun=true;
            for (var j=0u;j<u.light_count;j++) {
                let light=u.lights[j];
                if (light.kind.x!=KIND_SUN) {continue;}
                let ld=nebula_local(light.a.xyz);
                var tr=visibility.y;
                if (!first_sun) {
                    var optical_depth=0.0;
                    var distance=1e30;
                    for (var axis=0u;axis<3u;axis++) {
                        if (abs(ld[axis])>1e-6) {distance=min(distance,(sign(ld[axis])*u.nebula.extent[axis]-p[axis])/ld[axis]);}
                    }
                    for (var k=0u;k<32u;k++) {
                        let a=pow(f32(k)/32.0,2.0);let b=pow(f32(k+1u)/32.0,2.0);
                        let q=p+ld*distance*(a+b)*0.5;
                        optical_depth+=nebula_medium(q,0.0,ld*distance*(b-a)).extinction*distance*(b-a);
                        if (optical_depth>12.0) {break;}
                    }
                    tr=exp(-optical_depth);
                }
                let phase=hg_phase(dot(direction,ld),u.nebula.scattering.y);
                source+=medium.scattering*light.color.xyz*(phase*tr+
                    select(0.0,visibility.w*u.nebula.scattering.z/(4.0*PI),first_sun));
                first_sun=false;
            }
        }
        let extinction = max(medium.extinction, 0.0);
        emission+=transmittance*nebula_cluster_segment(origin,direction,
            start+f32(i)*step,start+f32(i+1u)*step,pixel_cone,extinction);
        let tau=extinction*step;
        emission += transmittance * source * step * nebula_beer_weight(tau);
        // Sum optical depth before exponentiation. Multiplying hundreds of
        // near-unit factors otherwise loses small but cumulative extinction.
        optical_depth+=tau;
        transmittance=exp(-optical_depth);
        if (transmittance < 0.001) { break; }
    }
    return vec4f(emission, transmittance);
}

fn nebula_stars(dir: vec3f, pixel_cone: f32) -> vec3f {
    if (!NEBULA_ENABLED || u.nebula.quality.z == 0u || u.nebula.settings.w == 0.0) { return vec3f(0.0); }
    let uv = vec2f(atan2(dir.z, dir.x) / (2.0 * PI) + 0.5,
        acos(clamp(dir.y, -1.0, 1.0)) / PI) * vec2f(2048.0, 1024.0);
    let cell = vec2i(floor(uv));
    let pixel_width = pixel_cone * 326.0;
    var result = vec3f(0.0);
    for (var y = -1; y <= 1; y++) { for (var x = -1; x <= 1; x++) {
        let c = cell + vec2i(x, y);
        // Wrap longitude so the distribution is seamless through atan2's cut.
        let wrapped = vec2u(u32((c.x + 2048) % 2048), u32(clamp(c.y, 0, 1023)));
        let h = pcg(wrapped.x ^ pcg(wrapped.y + u.nebula.quality.x * 131u));
        let choice = f32(h >> 8u) / 16777216.0;
        if (choice < u.nebula.settings.w * 0.012) {
            let h2 = pcg(h); let h3 = pcg(h2);
            let position = vec2f(c) + 0.2 + vec2f(f32(h2 & 65535u), f32(h2 >> 16u)) / 65535.0 * 0.6;
            let sigma = 0.055 + f32(h3 & 255u) / 255.0 * 0.10;
            let variance = sigma * sigma + pixel_width * pixel_width * 0.12;
            let delta = (uv - position) * vec2f(sqrt(max(0.01, 1.0 - dir.y * dir.y)), 1.0);
            let flux = exp(-dot(delta, delta) / (2.0 * variance)) * sigma * sigma / variance;
            let bright = 0.3 + pow(f32((h3 >> 8u) & 65535u) / 65535.0, 8.0) * 12.0;
            let tint = mix(vec3f(0.65, 0.79, 1.0), vec3f(1.0, 0.80, 0.59), f32(h3 >> 24u) / 255.0);
            result += tint * flux * bright;
        }
    } }
    return result;
}
