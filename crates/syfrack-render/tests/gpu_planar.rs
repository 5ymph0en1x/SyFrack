use glam::Vec2;
use syfrack_core::{PlanarFamily, PlanarFractal};
use syfrack_render::{uniforms::GpuPlanar, GpuContext};
use wgpu::util::DeviceExt;

#[test]
#[ignore = "needs GPU"]
fn complex_fields_match_cpu_on_classical_orbits() {
    let ctx = GpuContext::new().unwrap();
    let common = include_str!("../src/shaders/common.wgsl");
    let structure = common
        .split("struct PlanarFractal {")
        .nth(1)
        .unwrap()
        .split("struct FractalTexture")
        .next()
        .unwrap();
    let source = format!(
        "const PI:f32=3.14159265358979;\nstruct PlanarFractal {{{structure}\n\
        struct DeSample {{ distance:f32,traps:vec4f,iterations:u32,material:u32,flags:u32 }}\n\
        {}\n@group(0) @binding(0) var<storage,read_write> output:array<vec4f>;\n\
        @group(0) @binding(1) var<uniform> fractal:PlanarFractal;\n\
        @compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id:vec3u) {{\n\
        if(id.x<289u) {{ let point=vec2f(f32(id.x%17u)-8.0,f32(id.x/17u)-8.0)*0.125;\n\
        let result=planar_sample(fractal,point); output[id.x*2u]=result.traps;\n\
        output[id.x*2u+1u]=vec4f(result.distance,f32(result.iterations),0.0,0.0); }} }}",
        include_str!("../src/shaders/planar.wgsl")
    );
    let shader = ctx
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("complex parity"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
    let pipeline = ctx
        .device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("complex parity"),
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
    let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 289 * 32,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    for family in [
        PlanarFamily::Julia,
        PlanarFamily::Mandelbrot,
        PlanarFamily::BurningShip,
        PlanarFamily::Newton,
        PlanarFamily::Nova,
        PlanarFamily::Phoenix,
    ] {
        let fractal = PlanarFractal {
            family,
            power: if matches!(family, PlanarFamily::Newton | PlanarFamily::Nova) {
                3
            } else {
                2
            },
            iterations: 12,
            ..PlanarFractal::default()
        };
        let data = GpuPlanar::from_planar(Some(&fractal));
        let uniform = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::bytes_of(&data),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        let mut encoder = ctx.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(5, 1, 1);
        }
        ctx.queue.submit([encoder.finish()]);
        let bytes = ctx.read_buffer(&buffer, 289 * 32).unwrap();
        let values: Vec<[f32; 4]> = bytemuck::pod_collect_to_vec(&bytes);
        for i in 0..289 {
            let point = Vec2::new((i % 17) as f32 - 8.0, (i / 17) as f32 - 8.0) * 0.125;
            let cpu = fractal.sample(point);
            assert!(
                values[i * 2].iter().all(|v| v.is_finite()),
                "{family:?} {point}"
            );
            for c in 0..4 {
                let delta = (cpu.traps[c] - values[i * 2][c]).abs();
                let delta = if c == 3 {
                    delta.min((1.0 - delta).abs())
                } else {
                    delta
                };
                // Float32 Newton residuals near a root can differ by one convergence step.
                let tolerance =
                    if c == 0 && matches!(family, PlanarFamily::Newton | PlanarFamily::Nova) {
                        0.09
                    } else {
                        0.006
                    };
                assert!(
                    delta < tolerance,
                    "{family:?} at {point}, channel {c}: cpu {:?}, gpu {:?}",
                    cpu.traps,
                    values[i * 2]
                );
            }
            assert!((cpu.distance - values[i * 2 + 1][0]).abs() < 0.006);
        }
    }
}
