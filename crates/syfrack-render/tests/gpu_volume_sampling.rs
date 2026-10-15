//! Variance regression for saturated local lights in a thin participating medium.
use glam::Vec3;
use std::sync::Arc;
use syfrack_core::{Atmosphere, CameraMode, Color, Fractal, Graph, Light, NodeKind, Scene};
use syfrack_render::{GpuContext, RenderConfig, RenderSession, Renderer};

#[test]
#[ignore = "needs GPU"]
fn thin_fog_local_lights_match_quadrature_without_chromatic_blotch_variance() {
    let renderer = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = Scene::example();
    let mut graph = Graph::new_minimal();
    graph.node_mut(1).unwrap().kind = NodeKind::Plane {
        normal: Vec3::X,
        offset: 100.0,
    };
    scene.fractal = Fractal::Graph(graph);
    scene.camera.mode = CameraMode::Free {
        position: Vec3::ZERO,
        target: Vec3::Z,
        roll: 0.0,
    };
    scene.camera.focal_mm = 1200.0;
    scene.camera.f_number = 0.0;
    scene.material.albedo = [0.0; 3];
    scene.material.specular = 0.0;
    scene.material.metallic = 0.0;
    scene.material.texture = None;
    scene.material.surface = None;
    scene.material.coloring = syfrack_core::Coloring::None;
    scene.atmosphere = Atmosphere::none();
    scene.atmosphere.fog_density = 0.014;
    scene.atmosphere.fog_color = [0.9; 3];
    scene.atmosphere.fog_falloff = 0.0;
    scene.atmosphere.anisotropy = 0.15;
    scene.lights.clear();
    for (x, rgb) in [(-3.0, [1.0, 0.0, 0.0]), (3.0, [0.0, 0.0, 1.0])] {
        scene.lights.push(Light::Area {
            position: Vec3::new(x, 0.0, 10.0),
            normal: Vec3::new(-x.signum(), 0.0, 0.0),
            size: [2.0; 2],
            color: Color::Linear(rgb),
            intensity: 50.0,
        });
    }
    let mut cfg = RenderConfig::new(32, 32, 32);
    cfg.max_bounces = 0;
    let output = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert!(output.color.data.iter().flatten().all(|v| v.is_finite()));
    let layers = output.layers.as_ref().expect("fog radiance layers");
    assert!(layers
        .volume_variance
        .iter()
        .flatten()
        .all(|v| v.is_finite() && *v >= 0.0));
    let components = layers.components.as_ref().unwrap();
    assert!(components
        .direct_variance
        .iter()
        .chain(&components.indirect_variance)
        .flatten()
        .all(|v| v.is_finite() && *v >= 0.0));
    for (d, v) in components.direct.data.iter().zip(&layers.volume.data) {
        assert!(d[..3].iter().all(|v| *v >= 0.0));
        // With a black empty background, the medium has only primary direct energy.
        for c in 0..3 {
            assert!((d[c] - v[c]).abs() < 1e-6);
        }
    }
    for (i, p) in output.color.data.iter().enumerate() {
        for c in 0..3 {
            assert!((layers.surface.data[i][c] + layers.volume.data[i][c] - p[c]).abs() < 1e-6);
            assert!(
                layers.surface.data[i][c].abs() < 1e-6,
                "empty black background has no surface energy"
            );
        }
    }
    for c in [0, 2] {
        let n = output.color.data.len() as f64;
        let mean = output.color.data.iter().map(|p| p[c] as f64).sum::<f64>() / n;
        let variance = output
            .color
            .data
            .iter()
            .map(|p| (p[c] as f64 - mean).powi(2))
            .sum::<f64>()
            / n;
        // Independent 512 x 32 x 32 Gauss-Legendre integration of the area
        // and distance integrals on the centre ray, converged to 1e-12.
        let reference = 0.10281313317282229;
        assert!(
            (mean / reference - 1.0).abs() < 0.05,
            "channel {c}: mean {mean}, quadrature {reference}"
        );
        let cv = variance.sqrt() / mean;
        eprintln!("volume channel {c}: mean={mean}, coefficient of variation={cv}");
        // The old collision-only estimator measured 0.481 red / 0.545 blue.
        assert!(
            cv < 0.13,
            "chromatic volume variance remains too high: {cv}"
        );
    }
    let Fractal::Graph(ref mut graph) = scene.fractal;
    graph.node_mut(1).unwrap().kind = NodeKind::Plane {
        normal: Vec3::NEG_Z,
        offset: -6.0,
    };
    let blocked = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert!(
        blocked
            .color
            .data
            .iter()
            .all(|p| p[..3].iter().all(|v| v.abs() < 1e-6)),
        "a black wall in front of both lights must occlude the entire illuminated segment"
    );
}

#[test]
#[ignore = "needs GPU"]
fn projector_segment_integral_matches_independent_quadrature() {
    let renderer = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = Scene::example();
    let mut graph = Graph::new_minimal();
    graph.node_mut(1).unwrap().kind = NodeKind::Plane {
        normal: Vec3::X,
        offset: 100.0,
    };
    scene.fractal = Fractal::Graph(graph);
    scene.camera.mode = CameraMode::Free {
        position: Vec3::ZERO,
        target: Vec3::Z,
        roll: 0.0,
    };
    scene.camera.focal_mm = 1200.0;
    scene.camera.f_number = 0.0;
    scene.material.albedo = [0.0; 3];
    scene.material.specular = 0.0;
    scene.material.texture = None;
    scene.material.surface = None;
    scene.material.coloring = syfrack_core::Coloring::None;
    scene.atmosphere = Atmosphere::none();
    scene.atmosphere.fog_density = 0.014;
    scene.atmosphere.fog_color = [0.9; 3];
    scene.atmosphere.fog_falloff = 0.0;
    scene.atmosphere.anisotropy = 0.15;
    scene.lights.clear();
    for (x, rgb) in [(-3.0, [1.0, 0.0, 0.0]), (3.0, [0.0, 0.0, 1.0])] {
        scene.lights.push(Light::Projector {
            position: Vec3::new(x, 0.0, 10.0),
            normal: Vec3::new(-x.signum(), 0.0, 0.0),
            size: [2.0; 2],
            color: Color::Linear(rgb),
            intensity: 50.0,
            inner_angle: 10.0,
            outer_angle: 18.0,
        });
    }
    let mut cfg = RenderConfig::new(32, 32, 32);
    cfg.max_bounces = 0;
    let output = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    for c in [0, 2] {
        let n = output.color.data.len() as f64;
        let mean = output
            .color
            .data
            .iter()
            .map(|p| f64::from(p[c]))
            .sum::<f64>()
            / n;
        let variance = output
            .color
            .data
            .iter()
            .map(|p| (f64::from(p[c]) - mean).powi(2))
            .sum::<f64>()
            / n;
        // Independent area x line Gauss-Legendre quadrature, 128x128x256.
        let reference = 0.015958553887736832;
        eprintln!(
            "projector channel {c}: mean={mean}, cv={}, energy error={}",
            variance.sqrt() / mean,
            mean / reference - 1.0
        );
        assert!((mean / reference - 1.0).abs() < 0.03);
        assert!(variance.sqrt() / mean < 0.16);
    }
    // Looking at the emitter from outside its cone sees no emitted radiance.
    // Disable fog here: an oblique segment can cross beams from other points
    // of the finite rectangle, which is a valid volume contribution.
    scene.camera.mode = CameraMode::Free {
        position: Vec3::ZERO,
        target: Vec3::new(-3.0, 0.0, 10.0),
        roll: 0.0,
    };
    scene.lights.pop();
    scene.atmosphere.fog_density = 0.0;
    let off_axis = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert!(off_axis
        .color
        .data
        .iter()
        .all(|p| p[..3].iter().all(|v| v.abs() < 1e-6)));
    scene.camera.mode = CameraMode::Free {
        position: Vec3::new(3.0, 0.0, 10.0),
        target: Vec3::new(-3.0, 0.0, 10.0),
        roll: 0.0,
    };
    let on_axis = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert!(
        on_axis
            .color
            .data
            .iter()
            .all(|p| (p[0] - 50.0).abs() < 1e-5 && p[1] == 0.0 && p[2] == 0.0),
        "camera rays must see the same on-axis radiance as next-event paths"
    );
}

#[test]
#[ignore = "needs GPU"]
fn fog_export_guides_encode_transmission_and_ignore_random_sky_collision_depth() {
    let renderer = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = Scene::example();
    let mut graph = Graph::new_minimal();
    graph.node_mut(1).unwrap().kind = NodeKind::Plane {
        normal: Vec3::Z,
        offset: 0.0,
    };
    scene.fractal = Fractal::Graph(graph);
    scene.camera.mode = CameraMode::Free {
        position: Vec3::new(0.0, 0.0, 3.0),
        target: Vec3::ZERO,
        roll: 0.0,
    };
    scene.camera.f_number = 0.0;
    scene.camera.focal_mm = 1200.0;
    scene.atmosphere = Atmosphere::none();
    let cfg = RenderConfig::new(32, 32, 1);
    let run = |s: &Scene| {
        let mut session = RenderSession::new(renderer.clone(), s, &cfg).unwrap();
        while !session.is_done() {
            session.step(std::time::Duration::from_millis(20)).unwrap();
        }
        let before = session.readback().unwrap();
        let guides = session.export_guides().unwrap();
        let (surface_a, surface_n) = session.export_surface_guides().unwrap();
        if s.atmosphere.fog_density > 0.0 {
            for (i, n) in surface_n.data.iter().enumerate() {
                if before.normals[i][..3].iter().map(|v| v * v).sum::<f32>() > 0.5 {
                    assert!((n[..3].iter().map(|v| v * v).sum::<f32>() - 1.0).abs() < 1e-4);
                    assert!(n[3] > 0.0 && n[3] <= 1.0);
                    assert!(surface_a.data[i][..3]
                        .iter()
                        .all(|v| *v >= 0.0 && *v <= 1.0));
                }
            }
        }
        assert_eq!(before.color.data, session.readback().unwrap().color.data);
        (before, guides)
    };
    let (_, (clear_a, clear_n)) = run(&scene);
    scene.atmosphere.fog_density = 0.1;
    scene.atmosphere.fog_falloff = 0.0;
    scene.atmosphere.fog_color = [0.9, 0.8, 0.7];
    let (fog, (fog_a, fog_n)) = run(&scene);
    for i in 0..fog_a.data.len() {
        let t = (-0.1 * fog.depth[i]).exp();
        for c in 0..3 {
            assert!(
                (fog_a.data[i][c]
                    - (clear_a.data[i][c] * t + scene.atmosphere.fog_color[c] * (1.0 - t)))
                    .abs()
                    < 1e-5
            );
            assert!((fog_n.data[i][c] - clear_n.data[i][c] * t).abs() < 1e-5);
        }
    }
    let Fractal::Graph(ref mut graph) = scene.fractal;
    graph.node_mut(1).unwrap().kind = NodeKind::Plane {
        normal: Vec3::X,
        offset: 100.0,
    };
    let (sky, (sky_a, sky_n)) = run(&scene);
    assert!(sky.depth.iter().any(|d| *d > 0.0));
    for i in 0..sky_a.data.len() {
        for c in 0..3 {
            assert!((sky_a.data[i][c] - scene.atmosphere.fog_color[c]).abs() < 1e-6);
            assert_eq!(sky_n.data[i][c], 0.0);
        }
    }
}
