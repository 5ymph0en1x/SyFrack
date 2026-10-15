use syfrack_core::{PaletteInterpolation, Ramp};
use syfrack_render::{uniforms::GpuRamp, GpuContext};
use wgpu::util::DeviceExt;

#[test]
#[ignore = "needs GPU"]
fn palette_shader_matches_cpu_including_seams_and_negative_coordinates() {
    let ctx = GpuContext::new().unwrap();
    // Exercise the production sampling helper, not a test-only implementation.
    let common = include_str!("../src/shaders/common.wgsl");
    let structure = common
        .split("struct Palette {")
        .nth(1)
        .unwrap()
        .split("struct Coloring")
        .next()
        .unwrap();
    let helper = common
        .split("fn palette_mix")
        .nth(1)
        .unwrap()
        .split("fn ramp_sample")
        .next()
        .unwrap();
    let source = format!("struct Palette {{{structure}\nfn palette_mix{helper}\n\
        @group(0) @binding(0) var<storage,read_write> output: array<vec4f>;\n\
        @group(0) @binding(1) var<uniform> palette: Palette;\n\
        @compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id: vec3u) {{\n\
        if (id.x < 513u) {{ output[id.x] = vec4f(palette_sample(palette,f32(id.x)/128.0-2.0),1.0); }} }}");
    let shader = ctx
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("palette parity"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
    let pipeline = ctx
        .device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("palette parity"),
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
    let output = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 513 * 16,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let base = Ramp {
        stops: vec![
            [0.9, 0.1, 0.05],
            [0.02, 0.6, 0.9],
            [0.1, 0.01, 0.3],
            [1.0, 0.8, 0.2],
        ],
        ..Ramp::default()
    };
    let palettes = [
        base.clone(),
        Ramp {
            cyclic: true,
            frequency: 13.0,
            phase: -0.37,
            ..base.clone()
        },
        Ramp {
            cyclic: true,
            frequency: 21.0,
            phase: 0.21,
            gamma: 0.7,
            positions: vec![0.05, 0.31, 0.6, 0.91],
            interpolation: PaletteInterpolation::Smooth,
            ..base.clone()
        },
        Ramp {
            cyclic: true,
            stops: (0..16).map(|i| [i as f32 / 16.0, 0.2, 0.7]).collect(),
            frequency: 7.0,
            ..Ramp::default()
        },
    ];
    for ramp in palettes {
        ramp.validate("test").unwrap();
        let data = GpuRamp::from_ramp(&ramp);
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
                    resource: output.as_entire_binding(),
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
            pass.dispatch_workgroups(9, 1, 1);
        }
        ctx.queue.submit([encoder.finish()]);
        let bytes = ctx.read_buffer(&output, 513 * 16).unwrap();
        let values: Vec<[f32; 4]> = bytemuck::pod_collect_to_vec(&bytes);
        for (i, gpu) in values.iter().enumerate() {
            let cpu = ramp.sample(i as f32 / 128.0 - 2.0);
            for c in 0..3 {
                assert!(
                    (cpu[c] - gpu[c]).abs() < 1e-4,
                    "palette {ramp:?}, sample {i}: cpu {cpu:?} gpu {gpu:?}"
                );
            }
        }
    }
}
