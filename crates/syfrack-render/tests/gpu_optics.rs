use glam::Vec3;
use std::sync::Arc;
use syfrack_core::{Atmosphere, CameraMode, Color, Fractal, Graph, Light, Optics, Scene};
use syfrack_render::{GpuContext, RenderConfig, Renderer};

fn mean_rgb(image: &syfrack_core::HdrImage) -> [f32; 3] {
    std::array::from_fn(|c| {
        image.data.iter().map(|p| p[c]).sum::<f32>() / image.pixel_count() as f32
    })
}

#[test]
#[ignore = "needs GPU"]
fn dielectric_coverage_beer_furnace_subsurface_and_total_internal_reflection() {
    let renderer = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = Scene::example();
    scene.fractal = Fractal::Graph(Graph::new_minimal());
    scene.atmosphere = Atmosphere::none();
    scene.camera.mode = CameraMode::Free {
        position: Vec3::new(0.0, 0.0, 3.0),
        target: Vec3::ZERO,
        roll: 0.0,
    };
    scene.camera.focal_mm = 1000.0;
    scene.camera.f_number = 0.0;
    scene.lights = vec![Light::Environment {
        zenith: Color::Linear([1.0; 3]),
        horizon: Color::Linear([1.0; 3]),
        ground: Color::Linear([1.0; 3]),
        intensity: 1.0,
        rotation: 0.0,
    }];
    scene.material.optics = Some(Optics {
        opacity: 0.0,
        ..Default::default()
    });
    let mut cfg = RenderConfig::new(24, 17, 32);
    cfg.max_bounces = 0;
    cfg.pixel_tolerance = 0.25;
    let cutout = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert!(cutout.color.data.iter().all(|p| p[..3] == [1.0; 3]));

    cfg.max_bounces = 8;
    scene.material.optics = Some(Optics {
        ior: 1.0,
        ..Default::default()
    });
    let clear = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert!(clear.color.data.iter().all(|p| p[..3] == [1.0; 3]));
    scene.material.optics.as_mut().unwrap().absorption_color = [0.5, 0.25, 1.0];
    scene.material.optics.as_mut().unwrap().absorption_distance = 2.0;
    let tinted = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    for (actual, expected) in mean_rgb(&tinted.color).into_iter().zip([0.5, 0.25, 1.0]) {
        assert!(
            (actual - expected).abs() < 0.002,
            "Beer: {actual} vs {expected}"
        );
    }

    scene.material.optics = Some(Optics::default());
    scene.material.roughness = 0.02;
    cfg.max_bounces = 16;
    let furnace = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert!(furnace
        .color
        .data
        .iter()
        .flatten()
        .all(|x| x.is_finite() && *x >= 0.0));
    for c in mean_rgb(&furnace.color) {
        assert!((c - 1.0).abs() < 0.015, "white furnace: {c}");
    }

    scene.material.optics = Some(Optics {
        ior: 1.0,
        subsurface: 1.0,
        subsurface_radius: 0.35,
        subsurface_color: [0.9, 0.5, 0.2],
        ..Default::default()
    });
    cfg.max_bounces = 32;
    let scatter = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    let [r, g, b] = mean_rgb(&scatter.color);
    assert!(
        r > g && g > b && r < 1.0 && b > 0.0,
        "random walk: {r} {g} {b}"
    );
    assert!(scatter
        .color
        .data
        .iter()
        .flatten()
        .all(|x| x.is_finite() && *x >= 0.0 && *x <= 1.0001));

    scene.material.optics = Some(Optics::default());
    scene.camera.mode = CameraMode::Free {
        position: Vec3::new(0.0, 0.0, 0.9),
        target: Vec3::new(1.0, 0.0, 0.9),
        roll: 0.0,
    };
    cfg.max_bounces = 8;
    let trapped = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert!(
        mean_rgb(&trapped.color)[0] < 0.001,
        "TIR should keep these paths inside"
    );
}
