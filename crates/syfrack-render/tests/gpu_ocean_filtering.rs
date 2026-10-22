//! Regression of the actual WGSL capillary filter, independent of scene tracing.
use syfrack_core::{Graph, OceanSurface, Scene, Seascape};
use syfrack_render::{graph_wgsl, shader_source, uniforms::Uniforms, GpuContext};
use wgpu::util::DeviceExt;

#[test]
#[ignore = "needs GPU"]
fn filtered_capillary_slopes_retain_variance() {
    let gpu = GpuContext::new().unwrap();
    let mut source =
        shader_source::pathtrace(&graph_wgsl::compile(&Graph::new_minimal()).unwrap().wgsl);
    source.push_str(
        r#"
@group(1) @binding(0) var<storage, read_write> probe: array<vec4f>;
@compute @workgroup_size(64)
fn ocean_probe(@builtin(global_invocation_id) id: vec3u) {
    if (id.x >= 4096u) { return; }
    let p=vec2f(f32(id.x%64u)*0.713, f32(id.x/64u)*0.913);
    for (var level=0u; level<4u; level++) {
        var footprint=0.0;
        if (level==1u) { footprint=0.02; }
        if (level==2u) { footprint=0.12; }
        if (level==3u) { footprint=100.0; }
        let slope=ocean_micro_slope(p,footprint);
        probe[id.x*4u+level]=vec4f(slope,length(ocean_normal(p,footprint,true)));
    }
}
"#,
    );
    let module = gpu
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ocean filter regression"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
    let pipeline = gpu
        .device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ocean probe"),
            layout: None,
            module: &module,
            entry_point: Some("ocean_probe"),
            compilation_options: Default::default(),
            cache: None,
        });
    let size = 4096 * 4 * 16;
    let output = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let out_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(1),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: output.as_entire_binding(),
        }],
    });
    let mut scene = Scene::example();
    scene.atmosphere.seascape = Some(Seascape {
        surface: Some(OceanSurface {
            wave_height: 0.18,
            wavelength: 3.2,
            ripple_strength: 0.32,
            ..Default::default()
        }),
        ..Default::default()
    });
    let sample = |s: &Scene| {
        let uniform = Uniforms::from_scene(s, 16, 16, 2, 256);
        let buffer = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::bytes_of(&uniform),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.set_bind_group(1, &out_group, &[]);
            pass.dispatch_workgroups(64, 1, 1);
        }
        gpu.queue.submit([encoder.finish()]);
        let bytes = gpu.read_buffer(&output, size).unwrap();
        gpu.check().unwrap();
        bytemuck::cast_slice::<u8, [f32; 4]>(&bytes).to_vec()
    };
    let values = sample(&scene);
    let mut resolved = 0.0;
    let mut total_variance = 0.0;
    for levels in values.chunks_exact(4) {
        assert_eq!(levels[0][2], 0.0);
        for v in levels {
            assert!(v.iter().all(|x| x.is_finite()));
            assert!((v[3] - 1.0).abs() < 1e-5, "unit water normal");
        }
        assert!(
            levels.windows(2).all(|w| w[1][2] >= w[0][2]),
            "variance survives minification"
        );
        assert_eq!(levels[3][0], 0.0);
        assert_eq!(levels[3][1], 0.0);
        resolved += levels[0][0].powi(2) + levels[0][1].powi(2);
        total_variance += levels[3][2];
    }
    let ratio = resolved / total_variance;
    eprintln!("resolved mean-square slope / filtered variance: {ratio}");
    assert!(
        (0.80..1.20).contains(&ratio),
        "statistical variance must match the actual noise"
    );
    assert_eq!(values, sample(&scene), "deterministic filter");
    scene
        .atmosphere
        .seascape
        .as_mut()
        .unwrap()
        .surface
        .as_mut()
        .unwrap()
        .ripple_strength = 0.0;
    assert!(sample(&scene).iter().all(|v| v[..3] == [0.0; 3]));
    let ocean = scene
        .atmosphere
        .seascape
        .as_mut()
        .unwrap()
        .surface
        .as_mut()
        .unwrap();
    ocean.ripple_strength = 0.32;
    ocean.wave_height = 0.0;
    assert!(
        sample(&scene).iter().all(|v| v[..3] == [0.0; 3]),
        "flat-water compatibility"
    );
}
