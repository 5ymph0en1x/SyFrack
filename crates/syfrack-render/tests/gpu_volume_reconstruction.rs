//! Compare continuous RGB reconstruction to an independent high-sample image.
use glam::Vec3;
use std::{sync::Arc, time::Duration};
use syfrack_core::{
    Atmosphere, CameraMode, Color, Fractal, Graph, Light, Link, Node, NodeKind, Scene,
};
use syfrack_render::{GpuContext, RenderConfig, RenderSession, Renderer};

#[test]
#[ignore = "needs GPU"]
fn coloured_volume_and_occluder_converge_towards_high_sample_reference() {
    let renderer = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = Scene::example();
    let mut graph = Graph::new_minimal();
    graph.node_mut(1).unwrap().kind = NodeKind::Box {
        half: Vec3::new(0.4, 1.0, 0.4),
    };
    graph.nodes.push(Node {
        id: 3,
        name: "Occluder position".into(),
        pos: [0.0; 2],
        enabled: true,
        kind: NodeKind::Translate {
            offset: Vec3::new(1.1, 0.0, 6.0),
        },
    });
    graph.links.retain(|l| l.to != (1, 0));
    graph.links.extend([
        Link {
            from: (0, 0),
            to: (3, 0),
        },
        Link {
            from: (3, 0),
            to: (1, 0),
        },
    ]);
    scene.fractal = Fractal::Graph(graph);
    scene.camera.mode = CameraMode::Free {
        position: Vec3::ZERO,
        target: Vec3::Z,
        roll: 0.0,
    };
    scene.camera.focal_mm = 35.0;
    scene.camera.f_number = 0.0;
    scene.material.albedo = [0.0; 3];
    scene.material.specular = 0.0;
    scene.material.metallic = 0.0;
    scene.material.coloring = syfrack_core::Coloring::None;
    scene.material.texture = None;
    scene.material.surface = None;
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
    let run = |s: &Scene, samples| {
        let mut cfg = RenderConfig::new(64, 48, samples);
        cfg.max_bounces = 0;
        let mut session = RenderSession::new(renderer.clone(), s, &cfg).unwrap();
        while !session.is_done() {
            session.step(Duration::from_millis(100)).unwrap();
        }
        session
    };
    let session = run(&scene, 32);
    let low = session.readback().unwrap();
    let mut reference_scene = scene.clone();
    reference_scene.seed = 145;
    let reference = run(&reference_scene, 1024).readback().unwrap();
    let reference_volume = &reference.layers.as_ref().unwrap().volume;
    let layers = low.layers.as_ref().unwrap();
    let (_, normals) = session.export_surface_guides().unwrap();
    let transmission = normals.data.iter().map(|p| p[3]).collect::<Vec<_>>();
    let filtered = syfrack_render::volume_reconstruction::reconstruct(
        &layers.volume,
        &layers.volume_variance,
        &transmission,
    );
    // Visible emitters are surface radiance; compare the same volume component,
    // not a volume-only reconstruction against the combined beauty image.
    let mse = |image: &syfrack_core::HdrImage| {
        image
            .data
            .iter()
            .zip(&reference_volume.data)
            .map(|(a, b)| {
                [0, 2]
                    .into_iter()
                    .map(|c| f64::from(a[c] - b[c]).powi(2))
                    .sum::<f64>()
            })
            .sum::<f64>()
            / (image.data.len() * 2) as f64
    };
    let raw_error = mse(&layers.volume);
    let reconstructed_error = mse(&filtered);
    eprintln!("Volume shadow reference: raw MSE={raw_error}, reconstructed MSE={reconstructed_error}, ratio={}",reconstructed_error/raw_error);
    assert!(
        reconstructed_error < raw_error * 0.5,
        "continuous reconstruction must improve the physical shadow/colour field"
    );
    for c in [0, 2] {
        let expected = reference_volume
            .data
            .iter()
            .map(|p| f64::from(p[c]))
            .sum::<f64>();
        let actual = filtered.data.iter().map(|p| f64::from(p[c])).sum::<f64>();
        eprintln!(
            "Volume shadow channel {c}: integrated energy error={}",
            actual / expected - 1.0
        );
        assert!(
            (actual / expected - 1.0).abs() < 0.03,
            "reconstruction shifted energy too much"
        );
    }
    assert!(filtered.data.iter().flatten().all(|v| v.is_finite()));
    // Checkpoint/resume must preserve the additive layers and their dispersion.
    let path = std::env::temp_dir().join(format!("syfrack-volume-layers-{}", std::process::id()));
    session.save(&path).unwrap();
    let restored = RenderSession::resume(renderer, &path)
        .unwrap()
        .readback()
        .unwrap();
    assert_eq!(low.color.data, restored.color.data);
    let restored = restored.layers.unwrap();
    assert_eq!(layers.surface.data, restored.surface.data);
    assert_eq!(layers.volume.data, restored.volume.data);
    assert_eq!(layers.volume_variance, restored.volume_variance);
    let original = layers.components.as_ref().unwrap();
    let resumed = restored.components.as_ref().unwrap();
    assert_eq!(original.direct.data, resumed.direct.data);
    assert_eq!(original.direct_variance, resumed.direct_variance);
    assert_eq!(original.indirect_variance, resumed.indirect_variance);
    assert!(path
        .canonicalize()
        .unwrap()
        .starts_with(std::env::temp_dir().canonicalize().unwrap()));
    std::fs::remove_dir_all(path).unwrap();
}
