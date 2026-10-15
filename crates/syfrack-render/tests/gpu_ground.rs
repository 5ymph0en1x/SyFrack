//! Analytic ground: nearest-hit selection, material, normals and shadow opacity.
use glam::Vec3;
use std::sync::Arc;
use syfrack_core::{
    Atmosphere, CameraMode, Color, Finish, Fractal, Graph, GroundPlane, Light, Scene,
};
use syfrack_render::{GpuContext, RenderConfig, Renderer};

#[test]
#[ignore = "needs GPU"]
fn ground_intersects_exactly_and_blocks_primary_and_shadow_rays() {
    let renderer = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = Scene::example_for("sphere").unwrap();
    // The bundled sphere includes a graph plane; isolate a sphere here so it
    // cannot accidentally occlude the light during the ground-off reference.
    scene.fractal = Fractal::Graph(Graph::new_minimal());
    scene.atmosphere = Atmosphere::none();
    scene.atmosphere.ground = Some(GroundPlane {
        height: 0.0,
        finish: 1,
        ..GroundPlane::default()
    });
    scene.material.finishes = vec![Finish {
        albedo: [0.0; 3],
        specular: 0.0,
        emission: [1.0, 0.3, 0.1],
        ..Finish::default()
    }];
    scene.camera.mode = CameraMode::Free {
        position: Vec3::new(8.0, 5.0, 4.0),
        target: Vec3::new(8.0, 0.0, 0.0),
        roll: 0.0,
    };
    scene.camera.focal_mm = 500.0;
    scene.camera.f_number = 0.0;
    scene.lights = vec![Light::Area {
        position: Vec3::new(8.0, -2.0, 0.0),
        normal: Vec3::Y,
        size: [40.0; 2],
        color: Color::Linear([1.0; 3]),
        intensity: 20.0,
    }];
    let mut cfg = RenderConfig::new(24, 16, 4);
    cfg.max_bounces = 0;
    // At this tiny resolution, the default secondary-ray offset can cross the
    // 0.2-unit sphere/ground gap. Resolve that gap for the opacity assertion.
    cfg.pixel_tolerance = 0.125;
    cfg.tile_size = 8;
    let render = |scene: &Scene| renderer.render(scene, &cfg, &mut |_| {}).unwrap();
    let ground = render(&scene);
    assert!(ground.depth.iter().all(|d| *d > 6.2 && *d < 6.7));
    assert!(ground.normals.iter().all(|n| (n[1] - 1.0).abs() < 1e-6));
    for p in &ground.color.data {
        for (actual, expected) in p.iter().zip([1.0, 0.3, 0.1]) {
            assert!(
                (actual - expected).abs() < 1e-5,
                "ground finish must hide the lower emitter: {p:?}"
            );
        }
    }
    scene.atmosphere.ground = None;
    assert!(render(&scene).color.data.iter().all(|p| p[0] > 19.9));

    // A visible sphere receives light from below only when the ground is absent.
    scene.camera.mode = CameraMode::Free {
        position: Vec3::new(0.0, 0.0, 5.0),
        target: Vec3::ZERO,
        roll: 0.0,
    };
    scene.camera.focal_mm = 50.0;
    scene.material.albedo = [0.8; 3];
    scene.material.specular = 0.0;
    scene.material.finishes[0].emission = [0.0; 3];
    scene.lights = vec![Light::Area {
        position: Vec3::new(0.0, -3.0, 3.0),
        normal: Vec3::Y,
        size: [3.0; 2],
        color: Color::Linear([1.0; 3]),
        intensity: 30.0,
    }];
    let open = render(&scene);
    assert!(open.color.data.iter().any(|p| p[0] > 0.1));
    scene.atmosphere.ground = Some(GroundPlane {
        height: -1.2,
        finish: 1,
        ..GroundPlane::default()
    });
    let closed = render(&scene);
    let maximum = closed
        .color
        .data
        .iter()
        .map(|p| p[0])
        .fold(0.0_f32, f32::max);
    assert!(
        closed
            .color
            .data
            .iter()
            .all(|p| p[..3].iter().all(|v| v.abs() < 1e-6)),
        "blocked radiance: {maximum}"
    );
    let center = 8 * 24 + 12;
    assert!(
        (closed.depth[center] - open.depth[center]).abs() < 1e-5,
        "foreground sphere must still win the closest-hit query"
    );
}
