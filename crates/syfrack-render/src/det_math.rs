//! Experimental shared polynomial arithmetic, NOT a portable bit-exact promise.
//! WGSL permits non-fused `fma`, different division rounding and subnormal
//! flushing. Only the parity gate may qualify an implementation for a corpus.
//! These candidates are deliberately not substituted into production shaders.
//!
//! sin/cos accept finite |x| <= 8192; log2 positive normal inputs; exp2 accepts
//! [-126,128); pow accepts positive normal bases with a product in that range.
//! Outside those domains return canonical NaN (overflow exp2 returns infinity).
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

// One coefficient table is used in Rust and emitted as exact hexadecimal WGSL
// floats. Taylor series on reduced intervals; no unsubstantiated minimax claim.
const C: [f32; 34] = [
    std::f32::consts::FRAC_2_PI,
    f32::from_bits(0x3fc90fda),
    7.549_789_4e-8,
    -1.0 / 39_916_800.0,
    1.0 / 362_880.0,
    -1.0 / 5_040.0,
    1.0 / 120.0,
    -1.0 / 6.0,
    -1.0 / 3_628_800.0,
    1.0 / 40_320.0,
    -1.0 / 720.0,
    1.0 / 24.0,
    -0.5,
    std::f32::consts::FRAC_PI_4,
    std::f32::consts::FRAC_PI_2,
    std::f32::consts::PI,
    std::f32::consts::SQRT_2 - 1.0,
    1.0 / 13.0,
    -1.0 / 11.0,
    1.0 / 9.0,
    -1.0 / 7.0,
    1.0 / 5.0,
    -1.0 / 3.0,
    std::f32::consts::SQRT_2,
    2.0 * std::f32::consts::LOG2_E,
    1.0 / 11.0,
    1.0 / 9.0,
    1.0 / 7.0,
    1.0 / 5.0,
    1.0 / 3.0,
    std::f32::consts::LN_2,
    1.0 / 5_040.0,
    1.0 / 720.0,
    1.0 / 24.0,
];
const NAN: f32 = f32::from_bits(0x7fc0_0000);

pub fn det_sincos(x: f32) -> (f32, f32) {
    if x == 0.0 {
        return (x, 1.0);
    }
    if !x.is_finite() || x.abs() > 8192.0 {
        return (NAN, NAN);
    }
    let n = (x * C[0] + 0.5).floor();
    let r = (-n).mul_add(C[2], (-n).mul_add(C[1], x));
    let z = r * r;
    let mut s = C[3];
    for c in &C[4..8] {
        s = s.mul_add(z, *c);
    }
    let s = (r * z).mul_add(s, r);
    let mut c = C[8];
    for k in &C[9..13] {
        c = c.mul_add(z, *k);
    }
    let c = z.mul_add(c, 1.0);
    match (n as i32) & 3 {
        0 => (s, c),
        1 => (c, -s),
        2 => (-s, -c),
        _ => (-c, s),
    }
}
pub fn det_sin(x: f32) -> f32 {
    det_sincos(x).0
}
pub fn det_cos(x: f32) -> f32 {
    det_sincos(x).1
}

pub fn det_atan2(y: f32, x: f32) -> f32 {
    if !x.is_finite() || !y.is_finite() {
        return NAN;
    }
    let (ax, ay) = (x.abs(), y.abs());
    let mut r = if ax.max(ay) == 0.0 {
        0.0
    } else {
        ax.min(ay) / ax.max(ay)
    };
    let reduced = r > C[16];
    if reduced {
        r = (r - 1.0) / (r + 1.0);
    }
    let z = r * r;
    let mut a = C[17];
    for c in &C[18..23] {
        a = a.mul_add(z, *c);
    }
    a = (r * z).mul_add(a, r);
    if reduced {
        a += C[13];
    }
    if ay > ax {
        a = C[14] - a;
    }
    if x.to_bits() >> 31 != 0 {
        a = C[15] - a;
    }
    f32::from_bits(a.to_bits() ^ (y.to_bits() & 0x8000_0000))
}

pub fn det_log2(x: f32) -> f32 {
    if x == 0.0 {
        return f32::NEG_INFINITY;
    }
    if !x.is_finite() || x < f32::MIN_POSITIVE {
        return NAN;
    }
    let bits = x.to_bits();
    let mut e = ((bits >> 23) as i32 - 127) as f32;
    let mut m = f32::from_bits((bits & 0x7fffff) | 0x3f800000);
    if m > C[23] {
        m *= 0.5;
        e += 1.0;
    }
    let t = (m - 1.0) / (m + 1.0);
    let z = t * t;
    let mut p = C[25];
    for c in &C[26..30] {
        p = p.mul_add(z, *c);
    }
    p = p.mul_add(z, 1.0);
    (C[24] * t).mul_add(p, e)
}

pub fn det_exp2(x: f32) -> f32 {
    if x >= 128.0 {
        return f32::INFINITY;
    }
    if !x.is_finite() || x < -126.0 {
        return NAN;
    }
    let n = x.floor();
    let r = (x - n) * C[30];
    let mut p: f32 = 1.0 / 362_880.0;
    for c in [
        1.0 / 40_320.0,
        C[31],
        C[32],
        1.0 / 120.0,
        C[33],
        1.0 / 6.0,
        0.5,
        1.0,
        1.0,
    ] {
        p = p.mul_add(r, c);
    }
    p * f32::from_bits(((n as i32 + 127) as u32) << 23)
}

pub fn det_pow(x: f32, y: f32) -> f32 {
    if y == 0.0 {
        return 1.0;
    }
    if x == 0.0 && y > 0.0 {
        return 0.0;
    }
    det_exp2(y * det_log2(x))
}

/// An exact WGSL hex literal representing an ordinary finite binary32 value.
fn hex(x: f32) -> String {
    let b = x.to_bits();
    format!(
        "{}0x1.{:06x}p{:+}f",
        if x.is_sign_negative() { "-" } else { "" },
        (b & 0x7fffff) << 1,
        ((b >> 23) & 255) as i32 - 127
    )
}

pub fn wgsl() -> String {
    let mut source = String::new();
    for (i, c) in C.iter().enumerate() {
        source.push_str(&format!("const DC{i}: f32 = {};\n", hex(*c)));
    }
    source.push_str(include_str!("shaders/det_math.wgsl"));
    source
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MathParity {
    pub operation: String,
    pub inputs: usize,
    pub mismatches: usize,
    pub first_input: Option<[f32; 4]>,
    pub first_bits: Option<[u32; 2]>,
}

/// Isolate the candidate polynomials and rounding primitives from graph order.
pub fn audit(renderer: &crate::Renderer, count: usize) -> Result<Vec<MathParity>> {
    ensure!(
        (16..=1_000_000).contains(&count),
        "invalid math corpus size"
    );
    let mut input: Vec<_> = crate::parity::corpus(count, 2.0, 19)?
        .into_iter()
        .map(|v| [v.x * 4096.0, v.y.abs() + 0.125, v.z * 4.0, 0.0])
        .collect();
    // Runtime-loaded fused/non-fused witness; its product loses 2^-46 when
    // rounded before the subtraction. Never allow constant folding this test.
    input[0] = [
        f32::from_bits(0x3f800001),
        f32::from_bits(0x3f7ffffe),
        -1.0,
        0.0,
    ];
    let mut source = wgsl();
    source.push_str(r#"
@group(0) @binding(0) var<storage,read> input: array<vec4f>;
@group(0) @binding(1) var<storage,read_write> output: array<vec4u>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id:vec3u) {
    let i=id.x; if (i>=arrayLength(&input)) { return; }
    let p=input[i]; let sc=det_sincos(p.x);
    output[2u*i]=bitcast<vec4u>(vec4f(sc,det_atan2(p.z,p.x),det_log2(p.y)));
    output[2u*i+1u]=bitcast<vec4u>(vec4f(det_exp2(p.z),det_pow(p.y,p.z),fma(p.x,p.y,p.z),sqrt(p.y)));
}
"#);
    let ctx = renderer.context();
    let bytes = count as u64 * 32;
    let limits = ctx.device.limits();
    ensure!(
        bytes <= limits.max_storage_buffer_binding_size as u64
            && (count as u32).div_ceil(64) <= limits.max_compute_workgroups_per_dimension,
        "math corpus exceeds device limits"
    );
    let raw = renderer.with_scopes(|| {
        let module = ctx
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("arithmetic parity"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
        let pipeline = ctx
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("arithmetic parity"),
                layout: None,
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
        let input_buf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("arithmetic inputs"),
            size: count as u64 * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        ctx.queue
            .write_buffer(&input_buf, 0, bytemuck::cast_slice(&input));
        let output = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("arithmetic bits"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("arithmetic parity"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: input_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: output.as_entire_binding(),
                },
            ],
        });
        let mut enc = ctx.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups((count as u32).div_ceil(64), 1, 1);
        }
        ctx.queue.submit([enc.finish()]);
        let raw = ctx.read_buffer(&output, bytes)?;
        ctx.check()?;
        Ok(raw)
    })?;
    let gpu: Vec<[u32; 8]> = bytemuck::pod_collect_to_vec(&raw);
    let mut reports: Vec<_> = ["sin", "cos", "atan2", "log2", "exp2", "pow", "fma", "sqrt"]
        .into_iter()
        .map(|name| MathParity {
            operation: name.into(),
            inputs: count,
            mismatches: 0,
            first_input: None,
            first_bits: None,
        })
        .collect();
    for (p, actual) in input.iter().zip(gpu) {
        let expected = [
            det_sin(p[0]),
            det_cos(p[0]),
            det_atan2(p[2], p[0]),
            det_log2(p[1]),
            det_exp2(p[2]),
            det_pow(p[1], p[2]),
            p[0].mul_add(p[1], p[2]),
            p[1].sqrt(),
        ];
        for k in 0..8 {
            if expected[k].to_bits() != actual[k] {
                let r = &mut reports[k];
                r.mismatches += 1;
                if r.first_input.is_none() {
                    r.first_input = Some(*p);
                    r.first_bits = Some([expected[k].to_bits(), actual[k]]);
                }
            }
        }
    }
    Ok(reports)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn candidates_preserve_accuracy_on_their_documented_domains() {
        for i in 0..100_000 {
            let x = (i as f32 / 99_999.0 * 2.0 - 1.0) * 8192.0;
            assert!((det_sin(x) - x.sin()).abs() < 8e-7, "sin {x}");
            assert!((det_cos(x) - x.cos()).abs() < 8e-7, "cos {x}");
            let y = i as f32 / 99_999.0 * 16.0 + 0.01;
            assert!((det_atan2(y, x) - y.atan2(x)).abs() < 5e-7, "atan {y} {x}");
            assert!((det_log2(y) - y.log2()).abs() < 1e-6, "log {y}");
            let z = i as f32 / 99_999.0 * 32.0 - 16.0;
            assert!((det_exp2(z) / z.exp2() - 1.0).abs() < 8e-7, "exp {z}");
            assert!(
                (det_pow(y, 0.7) / y.powf(0.7) - 1.0).abs() < 1e-6,
                "pow {y}"
            );
        }
        assert!(det_sin(8193.0).is_nan());
        assert!(det_log2(-1.0).is_nan());
        assert!(det_exp2(-127.0).is_nan());
    }
    #[test]
    fn shared_hex_constants_and_wgsl_validate() {
        let source = wgsl();
        let module = wgpu::naga::front::wgsl::parse_str(&source).unwrap();
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        assert!(source.contains("0x1."));
    }
}
