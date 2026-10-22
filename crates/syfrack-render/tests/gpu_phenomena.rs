//! Native control/export integration of the same bounded natural field.
use glam::Vec3;
use std::sync::Arc;
use syfrack_core::{scene::phenomena::*, Phenomena, Scene};
use syfrack_render::{ComputeIntegrator, GpuContext, RenderConfig, Renderer};
fn node(id: u32, op: FieldOp, inputs: &[u32]) -> FieldNode {
    FieldNode {
        id,
        op,
        inputs: inputs.into(),
        name: format!("field{id}"),
        pos: [0.0; 2],
    }
}
#[test]
#[ignore = "needs GPU"]
fn spectral_strata_remove_chromatic_noise_from_a_uniform_environment() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = presets::scene("dispersion").unwrap();
    scene.camera.mode = syfrack_core::CameraMode::Free {
        position: Vec3::new(0.0, 0.0, 3.0),
        target: Vec3::new(0.0, 0.0, 5.0),
        roll: 0.0,
    };
    scene.atmosphere = syfrack_core::Atmosphere::none();
    scene.lights = vec![syfrack_core::Light::Environment {
        zenith: syfrack_core::Color::Linear([0.5; 3]),
        horizon: syfrack_core::Color::Linear([0.5; 3]),
        ground: syfrack_core::Color::Linear([0.5; 3]),
        intensity: 1.0,
        rotation: 0.0,
    }];
    let mut cfg = RenderConfig::new(8, 6, 9);
    cfg.tile_size = 5;
    cfg.max_bounces = 0;
    for integrator in [ComputeIntegrator::Megakernel, ComputeIntegrator::Wavefront] {
        cfg.integrator = integrator;
        let image = r.render(&scene, &cfg, &mut |_| {}).unwrap();
        for pixel in &image.color.data {
            for value in &pixel[..3] {
                assert!((value - 0.5).abs() < 2e-5, "{integrator:?}: {pixel:?}");
            }
        }
    }
}
#[test]
#[ignore = "needs GPU"]
fn quaternion_empty_margin_remains_empty_in_both_gpu_evaluators() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let scene = presets::scene("quaternion").unwrap();
    let points = [Vec3::X, Vec3::Y, Vec3::Z, -Vec3::X, -Vec3::Y, -Vec3::Z].map(|p| p * 1.7);
    let native = r.probe_distances(&scene, &points).unwrap();
    let control = r.control_distances(&scene, &points).unwrap();
    for (a, b) in native.iter().zip(&control) {
        assert!(a.distance > 0.05, "false domain surface: {a:?}");
        assert!((a.distance - b.distance).abs() < 2e-6);
    }
}
#[test]
#[ignore = "needs GPU"]
fn field_isosurface_control_and_specialized_probe_agree_and_bounds_do_not_become_faces() {
    let renderer = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = Scene::example();
    let mut p = Phenomena::default();
    p.domain.resolution = [17, 17, 17];
    p.domain.half = Vec3::splat(2.0);
    p.fields.nodes = vec![
        node(0, FieldOp::Position, &[]),
        node(
            1,
            FieldOp::SphereDistance {
                center: Vec3::ZERO,
                radius: 0.7,
            },
            &[0],
        ),
    ];
    p.bindings.geometry = Some(1);
    p.bindings.geometry_mode = FieldGeometry::Replace;
    scene.phenomena = Some(p);
    let points: Vec<_> = (-16..=16)
        .map(|i| Vec3::new(i as f32 * 0.15, 0.0, 0.0))
        .collect();
    let native = renderer.probe_distances(&scene, &points).unwrap();
    let control = renderer.control_distances(&scene, &points).unwrap();
    for (a, b) in native.iter().zip(&control) {
        assert!((a.distance - b.distance).abs() < 2e-6, "{a:?} {b:?}");
    }
    assert!(native[16].distance < 0.0);
    assert!(native[0].distance > 0.0);
    assert!(native[32].distance > 0.0);
}
#[test]
#[ignore = "needs GPU"]
fn native_two_dimensional_field_is_visible_without_a_planar_texture() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut s = presets::scene("interference").unwrap();
    s.dimension = syfrack_core::Dimension::TwoD;
    s.material.texture = None;
    s.phenomena.as_mut().unwrap().domain.resolution = [32, 32, 1];
    let out = r
        .render(&s, &RenderConfig::new(32, 24, 4), &mut |_| {})
        .unwrap();
    assert_eq!(out.samples_min, 4);
    assert!(out
        .color
        .data
        .iter()
        .all(|v| v.iter().all(|x| x.is_finite())));
    let min = out
        .color
        .data
        .iter()
        .map(|v| v[0])
        .fold(f32::INFINITY, f32::min);
    let max = out.color.data.iter().map(|v| v[0]).fold(0.0, f32::max);
    assert!(max - min > 0.02);
}
#[test]
#[ignore = "needs GPU"]
fn self_emitting_fields_survive_wavefront_partition_and_reference_transport() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut s = presets::scene("interference").unwrap();
    s.phenomena.as_mut().unwrap().domain.resolution = [16, 16, 16];
    let mut cfg = RenderConfig::new(24, 16, 4);
    cfg.max_bounces = 0;
    cfg.max_steps = 128;
    let reference = r.render(&s, &cfg, &mut |_| {}).unwrap();
    cfg.integrator = ComputeIntegrator::Wavefront;
    let wave = r.render(&s, &cfg, &mut |_| {}).unwrap();
    let mut error = 0.0f64;
    let mut energy = 0.0f64;
    assert!(reference.layers.is_some() && wave.layers.is_some());
    for (a, b) in reference.color.data.iter().zip(&wave.color.data) {
        for k in 0..3 {
            assert!(a[k].is_finite() && b[k].is_finite());
            error += (a[k] - b[k]).powi(2) as f64;
            energy += a[k].abs() as f64;
        }
    }
    assert!(energy > 0.1);
    assert!(
        (error / (24.0 * 16.0 * 3.0)).sqrt() < 1e-4,
        "reference/wavefront mismatch {error}"
    );
}
#[test]
#[ignore = "needs GPU"]
fn compute_units_evolve_both_pdes_like_the_independent_host_reference() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    for model in [
        SimulationModel::GrayScott {
            diffusion: [0.16, 0.08],
            feed: 0.035,
            kill: 0.062,
        },
        SimulationModel::GrossPitaevskii {
            dispersion: 0.05,
            interaction: 0.2,
            damping: 0.0,
        },
    ] {
        let mut s = presets::scene("morphogenesis").unwrap();
        let p = s.phenomena.as_mut().unwrap();
        p.domain.resolution = [17, 17, 1];
        p.domain.half = Vec3::ONE;
        p.simulations[0].model = model;
        p.simulations[0].steps = 10;
        p.simulations[0].dt = 0.01;
        if matches!(
            p.simulations[0].model,
            SimulationModel::GrossPitaevskii { .. }
        ) {
            p.fields.nodes[1].op = FieldOp::Snapshot {
                simulation: 0,
                channel: 2,
            };
        }
        let p = s.phenomena.as_ref().unwrap();
        let (host, _) = syfrack_render::phenomena::simulation::prepare(&s, p, 0, &[]).unwrap();
        let gpu = r.prepare_phenomena(&s).unwrap().unwrap();
        let mut worst = 0.0f32;
        for (a, b) in host.data.iter().zip(&gpu.snapshots[0].data) {
            worst = worst.max((*a - *b).abs().max_element());
        }
        assert!(worst < 2e-4, "PDE CPU/GPU maximum error {worst}");
        let d = &gpu.diagnostics[0];
        if matches!(
            p.simulations[0].model,
            SimulationModel::GrossPitaevskii { .. }
        ) {
            assert!((d.mass_after / d.mass_before - 1.0).abs() < 1e-4);
        }
    }
}

#[test]
#[ignore = "needs GPU"]
fn kerr_compute_geodesics_follow_the_independent_hamiltonian_reference() {
    let ctx = GpuContext::new().unwrap();
    let d = &ctx.device;
    let source=format!("struct Lens {{lens_center:vec4f,lens_axis:vec4f,lens_config:vec4f}}\nstruct Settings {{phenomena:Lens}}\n@group(0) @binding(0) var<uniform> u:Settings;\n@group(0) @binding(1) var<storage,read_write> output:array<vec4f>;\n{}\n@compute @workgroup_size(1) fn main(){{var s=kerr_launch(vec3f(2.0,1.0,8.0),normalize(vec3f(-0.2,0.0,-1.0)));for(var i=0u;i<100u;i++){{s=kerr_advance(s,0.02);}}output[0]=s.x;output[1]=s.k;output[2]=vec4f(kerr_hamiltonian(s.x.xyz,s.k));}}",syfrack_render::shader_source::GEODESIC);
    let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Kerr independent probe"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("Kerr independent probe"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let buffer = |size, usage| {
        d.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size,
            usage,
            mapped_at_creation: false,
        })
    };
    let info = buffer(
        48,
        wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    );
    let output = buffer(
        48,
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    );
    let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: info.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: output.as_entire_binding(),
            },
        ],
    });
    for spin in [0.0, 0.8, -0.8] {
        let lens = RelativisticLens {
            spin,
            ..Default::default()
        };
        let values: [f32; 12] = [
            0.0,
            0.0,
            0.0,
            lens.mass,
            0.0,
            1.0,
            0.0,
            spin,
            lens.step,
            lens.max_steps as f32,
            lens.escape_radius,
            1.0,
        ];
        ctx.queue
            .write_buffer(&info, 0, bytemuck::cast_slice(&values));
        let mut encoder = d.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        ctx.queue.submit([encoder.finish()]);
        let raw = ctx.read_buffer(&output, 48).unwrap();
        let gpu: Vec<[f32; 4]> = bytemuck::pod_collect_to_vec(&raw);
        let mut cpu = lens.launch_past(Vec3::new(2.0, 1.0, 8.0), Vec3::new(-0.2, 0.0, -1.0));
        for _ in 0..100 {
            cpu = lens.advance(cpu, 0.02);
        }
        assert!(
            (glam::Vec4::from_array(gpu[0]) - cpu.position)
                .abs()
                .max_element()
                < 5e-4,
            "spin {spin}: {:?} {:?}",
            gpu[0],
            cpu.position
        );
        assert!(
            (glam::Vec4::from_array(gpu[1]) - cpu.momentum)
                .abs()
                .max_element()
                < 5e-4
        );
        assert!(gpu[2][0].abs() < 1e-3);
    }
    ctx.check().unwrap();
}

#[test]
#[ignore = "needs GPU"]
fn curved_and_dispersed_paths_resume_across_wavefront_stages() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    for effect in ["kerr", "dispersion", "iridescent-glass"] {
        let mut s = presets::scene(if effect == "iridescent-glass" {
            "dispersion"
        } else {
            effect
        })
        .unwrap();
        if effect == "iridescent-glass" {
            let p = s.phenomena.as_mut().unwrap();
            p.dispersion = None;
            p.film = Some(ThinFilm {
                substrate_ior: 1.0,
                ..Default::default()
            });
            s.material.optics.as_mut().unwrap().thin_walled = true;
        }
        s.fractal = syfrack_core::Fractal::Graph(syfrack_core::Graph::new_minimal());
        let mut cfg = RenderConfig::new(16, 12, 6);
        cfg.max_bounces = 2;
        cfg.max_steps = 128;
        if let Some(lens) = s.phenomena.as_mut().unwrap().lens.as_mut() {
            lens.mass = 0.15;
            lens.center = Vec3::new(0.0, 0.0, 2.0);
            lens.step = 0.06;
            lens.max_steps = 1024;
            lens.escape_radius = 12.0;
        }
        let reference = r.render(&s, &cfg, &mut |_| {}).unwrap();
        cfg.integrator = ComputeIntegrator::Wavefront;
        let wave = r.render(&s, &cfg, &mut |_| {}).unwrap();
        let mut error = 0.0f64;
        for (a, b) in reference.color.data.iter().zip(&wave.color.data) {
            for k in 0..3 {
                assert!(a[k].is_finite() && b[k].is_finite(), "{effect}");
                error += (a[k] - b[k]).powi(2) as f64;
            }
        }
        assert!(
            (error / (16.0 * 12.0 * 3.0)).sqrt() < 1e-4,
            "{effect}: partition mismatch {error}"
        );
        if effect == "kerr" {
            let mut control =
                syfrack_render::control::ControlSession::new(r.clone(), &s, (16, 12)).unwrap();
            while !control.complete() {
                control.tick().unwrap();
                std::thread::yield_now();
            }
            assert_eq!(control.completed_pixels(), 16 * 12);
        }
    }
}
