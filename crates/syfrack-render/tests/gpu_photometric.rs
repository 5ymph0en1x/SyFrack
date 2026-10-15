//! Physical regressions: capsule emission, additive sources, grazing concrete,
//! and analytical height-fog transport. Requires a native GPU.
use glam::Vec3;
use std::sync::Arc;
use syfrack_core::{
    Atmosphere, CameraMode, Color, Coloring, Finish, FinishDetail, FinishPattern, Fractal, Graph,
    Light, Look, NodeKind, Scene,
};
use syfrack_render::{GpuContext, RenderConfig, Renderer, Uniforms};
use wgpu::util::DeviceExt;

fn base(kind: NodeKind) -> Scene {
    let mut s = Scene::example();
    let mut graph = Graph::new_minimal();
    graph.node_mut(1).unwrap().kind = kind;
    s.fractal = Fractal::Graph(graph);
    s.camera.mode = CameraMode::Free {
        position: Vec3::new(0.0, 0.0, 3.0),
        target: Vec3::ZERO,
        roll: 0.0,
    };
    s.camera.focal_mm = 500.0;
    s.camera.f_number = 0.0;
    s.material.albedo = [0.5; 3];
    s.material.specular = 0.0;
    s.material.metallic = 0.0;
    s.material.coloring = Coloring::None;
    s.material.texture = None;
    s.material.surface = None;
    s.atmosphere = Atmosphere::none();
    s.look = Look::neutral();
    s.lights.clear();
    s
}
fn tube(x: f32, color: [f32; 3]) -> Light {
    Light::Tube {
        start: Vec3::new(x, -1.5, 2.0),
        end: Vec3::new(x, 1.5, 2.0),
        radius: 0.16,
        color: Color::Linear(color),
        intensity: 15.0,
    }
}
fn mean(r: &Renderer, s: &Scene, samples: u32, bounces: u32) -> [f32; 3] {
    let mut cfg = RenderConfig::new(24, 24, samples);
    cfg.max_bounces = bounces;
    let out = r.render(s, &cfg, &mut |_| {}).unwrap();
    assert!(out.color.data.iter().flatten().all(|v| v.is_finite()));
    let mut m = [0.0; 3];
    for p in &out.color.data {
        for c in 0..3 {
            m[c] += p[c] / out.color.data.len() as f32;
        }
    }
    m
}

#[test]
#[ignore = "needs GPU"]
fn tubes_are_visible_and_red_blue_sources_add_on_concrete() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut s = base(NodeKind::Plane {
        normal: Vec3::Z,
        offset: 0.0,
    });
    let red = tube(-2.0, [1.0, 0.0, 0.0]);
    let blue = tube(2.0, [0.0, 0.0, 1.0]);
    s.lights = vec![red.clone()];
    let a = mean(&r, &s, 512, 0);
    s.lights = vec![blue.clone()];
    let b = mean(&r, &s, 512, 0);
    s.lights = vec![red, blue];
    let both = mean(&r, &s, 1024, 0);
    assert!(a[0] > 0.01 && b[2] > 0.01, "{a:?} {b:?}");
    for c in [0, 2] {
        let expected = a[c] + b[c];
        assert!(
            (both[c] - expected).abs() < expected * 0.10,
            "{both:?}, expected {a:?}+{b:?}"
        );
    }
    s.fractal.graph_mut().node_mut(1).unwrap().kind = NodeKind::Sphere { radius: 0.01 };
    s.lights = vec![Light::Tube {
        start: Vec3::new(-2.0, 0.0, 1.0),
        end: Vec3::new(2.0, 0.0, 1.0),
        radius: 0.7,
        color: Color::Linear([1.0, 0.1, 0.025]),
        intensity: 4.0,
    }];
    let visible = mean(&r, &s, 4, 0);
    for (actual, expected) in visible.into_iter().zip([4.0, 0.4, 0.1]) {
        assert!((actual - expected).abs() < 1e-4, "{visible:?}");
    }
}

#[test]
#[ignore = "needs GPU"]
fn gradient_sky_lights_a_surface_without_double_counting_continuation() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut s = base(NodeKind::Plane {
        normal: Vec3::Z,
        offset: 0.0,
    });
    s.lights = vec![Light::Environment {
        zenith: Color::Linear([1.0; 3]),
        horizon: Color::Linear([1.0; 3]),
        ground: Color::Linear([1.0; 3]),
        intensity: 1.0,
        rotation: 0.0,
    }];
    let direct = mean(&r, &s, 512, 0);
    let continued = mean(&r, &s, 512, 1);
    assert!((direct[0] - 0.5).abs() < 0.012, "{direct:?}");
    assert!((continued[0] - 0.5).abs() < 0.025, "{continued:?}");
}

#[test]
#[ignore = "needs GPU"]
fn red_and_blue_tubes_add_in_scattering_air() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut s = base(NodeKind::Plane {
        normal: Vec3::X,
        offset: 100.0,
    });
    s.camera.mode = CameraMode::Free {
        position: Vec3::ZERO,
        target: Vec3::Z,
        roll: 0.0,
    };
    s.atmosphere.fog_density = 0.01;
    s.atmosphere.fog_falloff = 0.0;
    s.atmosphere.fog_color = [0.8; 3];
    s.atmosphere.anisotropy = 0.25;
    let red = tube(-1.2, [1.0, 0.0, 0.0]);
    let blue = tube(1.2, [0.0, 0.0, 1.0]);
    s.lights = vec![red.clone()];
    let a = mean(&r, &s, 1024, 0);
    s.lights = vec![blue.clone()];
    let b = mean(&r, &s, 1024, 0);
    s.lights = vec![red, blue];
    let mixed = mean(&r, &s, 2048, 0);
    for c in [0, 2] {
        let expected = a[c] + b[c];
        assert!(expected > 0.001);
        assert!(
            (mixed[c] - expected).abs() < expected * 0.13,
            "mixed {mixed:?}, separate {a:?}+{b:?}"
        );
    }
    let multiple = mean(&r, &s, 2048, 3);
    assert!(
        multiple[0] > mixed[0] * 0.9 && multiple[2] > mixed[2] * 0.9,
        "multiple scattering lost energy: {multiple:?}, {mixed:?}"
    );
}

#[test]
#[ignore = "needs GPU"]
fn vertical_concrete_bump_changes_raking_light_preserving_geometry() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut s = base(NodeKind::Plane {
        normal: Vec3::Z,
        offset: 0.0,
    });
    s.camera.focal_mm = 60.0;
    s.material.finishes = vec![Finish {
        albedo: [0.45; 3],
        roughness: 0.9,
        detail: Some(FinishDetail {
            pattern: FinishPattern::Concrete,
            scale: 1.0,
            strength: 0.0,
            bump: 0.0,
            ..FinishDetail::default()
        }),
        ..Finish::default()
    }];
    // Global material uses the same detail through an explicit material tag.
    let g = s.fractal.graph_mut();
    let id = g.nodes.len() as u32;
    g.nodes.push(syfrack_core::Node {
        id,
        kind: NodeKind::MaterialTag { index: 1 },
        name: "concrete".into(),
        pos: [0.0; 2],
        enabled: true,
    });
    g.links.retain(|l| l.to.0 != 2);
    g.links.push(syfrack_core::Link {
        from: (1, 0),
        to: (id, 0),
    });
    g.links.push(syfrack_core::Link {
        from: (id, 0),
        to: (2, 0),
    });
    s.lights = vec![Light::Sun {
        direction: Vec3::new(0.98, 0.0, 0.20).normalize(),
        color: Color::Linear([1.0; 3]),
        intensity: 3.0,
        angular_size: 0.1,
    }];
    let mut cfg = RenderConfig::new(48, 48, 16);
    cfg.max_bounces = 0;
    let plain = r.render(&s, &cfg, &mut |_| {}).unwrap();
    s.material.finishes[0].detail.as_mut().unwrap().bump = 0.07;
    let rough = r.render(&s, &cfg, &mut |_| {}).unwrap();
    assert_eq!(plain.depth, rough.depth);
    assert_eq!(plain.normals, rough.normals);
    let difference = plain
        .color
        .data
        .iter()
        .zip(&rough.color.data)
        .map(|(a, b)| (a[0] - b[0]).abs())
        .sum::<f32>()
        / 2304.0;
    assert!(
        difference > 0.008,
        "raking relief disappeared: {difference}"
    );
    assert!(rough.color.data.iter().flatten().all(|v| v.is_finite()));
    // Export guides must include the actual bump and leave both transport and
    // saved geometry intact, including a later continuation of the render.
    let mut session = syfrack_render::RenderSession::new(r.clone(), &s, &cfg).unwrap();
    while session.progress().samples_min < 1 {
        session.step(std::time::Duration::ZERO).unwrap();
    }
    let before = session.readback().unwrap();
    let (albedo, normals) = session.export_guides().unwrap();
    let cached = session.export_guides().unwrap();
    assert_eq!(albedo.data, cached.0.data);
    assert_eq!(normals.data, cached.1.data);
    assert!(albedo.data.iter().all(|p| p[..3]
        .iter()
        .all(|v| v.is_finite() && (0.0..=1.0).contains(v))));
    assert!(normals.data.iter().all(|p| p[..3]
        .iter()
        .all(|v| v.is_finite() && (-1.0..=1.0).contains(v))));
    assert!(normals.data.iter().any(|p| p[0].abs() + p[1].abs() > 0.01));
    let after = session.readback().unwrap();
    assert_eq!(before.color.data, after.color.data);
    assert_eq!(before.depth, after.depth);
    assert_eq!(before.normals, after.normals);
    while !session.is_done() {
        session.step(std::time::Duration::from_millis(20)).unwrap();
    }
    assert_eq!(session.readback().unwrap().color.data, rough.color.data);
    s.material.finishes[0].detail.as_mut().unwrap().bump = 0.0;
    session.reset(&s).unwrap();
    assert!(session.export_guides().is_err());
    while session.progress().samples_min < 1 {
        session.step(std::time::Duration::ZERO).unwrap();
    }
    let (_, flat_normals) = session.export_guides().unwrap();
    assert!(flat_normals
        .data
        .iter()
        .all(|p| p[0].abs() + p[1].abs() < 0.000001));
}

#[test]
#[ignore = "needs GPU"]
fn analytical_fog_matches_numerical_integral_and_inverts_collision_depth() {
    let gpu = GpuContext::new().unwrap();
    let mut s = Scene::example();
    s.atmosphere.fog_density = 0.03;
    s.atmosphere.fog_height = 2.0;
    s.atmosphere.fog_falloff = 0.17;
    let uniforms = Uniforms::from_scene(&s, 1, 1, 0, 128);
    let compiled = syfrack_render::graph_wgsl::compile(s.fractal.graph()).unwrap();
    let mut source = syfrack_render::shader_source::pathtrace(&compiled.wgsl);
    source.push_str(
        r#"
@group(0) @binding(12) var<storage,read_write> fog_result:array<vec4f>;
@compute @workgroup_size(1) fn fog_probe(@builtin(global_invocation_id) id:vec3u) {
    let ro=vec3f(0.0,f32(id.x/3u)*4.0-12.0,0.0);
    let y=f32(id.x%3u)-1.0;let rd=vec3f(sqrt(max(0.0,1.0-y*y)),y,0.0);
    let tau=fog_optical_depth(ro,rd,80.0);let split=fog_split(ro,rd,80.0);
    let first=fog_segment_tau(ro,rd,0.0,split);let desired_tau=tau*0.37;
    var t=fog_invert_segment(ro,rd,split,80.0,desired_tau-first);
    if (desired_tau<first) {t=fog_invert_segment(ro,rd,0.0,split,desired_tau);}
    var rng=19u;let tr=fog_transmittance(ro,rd,80.0,&rng);
    fog_result[id.x]=vec4f(tau,fog_optical_depth(ro,rd,t),tr,t);
}
"#,
    );
    let module = gpu
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fog probe"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
    let pipeline = gpu
        .device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &module,
            entry_point: Some("fog_probe"),
            compilation_options: Default::default(),
            cache: None,
        });
    let uniform = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::bytes_of(&uniforms),
            usage: wgpu::BufferUsages::UNIFORM,
        });
    let output = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 27 * 16,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 12,
                resource: output.as_entire_binding(),
            },
        ],
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(27, 1, 1);
    }
    gpu.queue.submit([encoder.finish()]);
    let bytes = gpu.read_buffer(&output, 27 * 16).unwrap();
    gpu.check().unwrap();
    let values: &[[f32; 4]] = bytemuck::cast_slice(&bytes);
    for (i, v) in values.iter().enumerate() {
        let height = (i / 3) as f64 * 4.0 - 12.0;
        let dy = (i % 3) as f64 - 1.0;
        let n = 20000;
        let dt = 80.0 / n as f64;
        let numeric = (0..n)
            .map(|j| {
                0.03 * ((-(height + dy * (j as f64 + 0.5) * dt - 2.0) * 0.17).min(0.0)).exp() * dt
            })
            .sum::<f64>() as f32;
        assert!(
            (v[0] - numeric).abs() < 2e-5,
            "case {i}: {v:?}, quadrature {numeric}"
        );
        assert!((v[1] - v[0] * 0.37).abs() < 2e-5, "inverse {i}: {v:?}");
        assert!((v[2] - (-numeric).exp()).abs() < 2e-5);
    }
    gpu.check().unwrap();
}
