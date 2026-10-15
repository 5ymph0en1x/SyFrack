//! The 2D workshop must not inherit illumination, perspective or surface shading.
use std::{sync::Arc, time::Duration};
use syfrack_core::{CameraMode, Scene};
use syfrack_render::{GpuContext, RenderConfig, RenderSession, Renderer};

fn image(renderer: &Renderer, scene: &Scene) -> syfrack_core::HdrImage {
    let mut config = RenderConfig::new(64, 40, 8);
    config.tile_size = 64;
    let mut session = RenderSession::new(renderer.clone(), scene, &config).unwrap();
    while !session.step(Duration::from_millis(100)).unwrap() {}
    let output = session.readback().unwrap();
    assert_eq!((output.samples_min, output.samples_max), (8, 8));
    assert!(output.color.data.iter().flatten().all(|v| v.is_finite()));
    output.color
}

#[test]
#[ignore = "needs GPU"]
fn flat_canvas_ignores_3d_shading_and_responds_to_framing_and_palette() {
    let gpu = Arc::new(GpuContext::new().unwrap());
    let renderer = Renderer::new(gpu).unwrap();
    let scene = Scene::example_2d();
    let reference = image(&renderer, &scene);
    let mut physical = scene.clone();
    physical.camera.mode = CameraMode::Free {
        position: glam::Vec3::new(50.0, 30.0, 20.0),
        target: glam::Vec3::ZERO,
        roll: 80.0,
    };
    physical.material.albedo = [0.001; 3];
    physical.material.metallic = 1.0;
    physical.material.roughness = 0.99;
    physical.material.texture.as_mut().unwrap().bump = 0.2;
    physical.lights = Scene::example().lights;
    physical.atmosphere.fog_density = 0.1;
    assert_eq!(
        reference.data,
        image(&renderer, &physical).data,
        "2D colour must be independent of 3D settings"
    );
    let mut framed = scene.clone();
    framed.canvas.zoom = 1.5;
    framed.canvas.center = [0.2, -0.1];
    assert_ne!(
        reference.data,
        image(&renderer, &framed).data,
        "framing must move the visible motif"
    );
    let mut coloured = scene;
    coloured.material.texture.as_mut().unwrap().ramp =
        syfrack_core::scene::palette::neon_palette(3);
    assert_ne!(
        reference.data,
        image(&renderer, &coloured).data,
        "palette must change the actual rendered pixels"
    );
}

#[test]
#[ignore = "needs GPU"]
fn flat_canvas_large_initial_tiles_preserve_samples_and_reset() {
    let renderer = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = Scene::example_2d();
    scene.canvas.zoom = 4.0;
    scene.material.texture.as_mut().unwrap().planar = Some(syfrack_core::PlanarFractal {
        julia: [-0.835, -0.2321],
        iterations: 384,
        ..Default::default()
    });
    let render = |session: &mut RenderSession| {
        while !session.step(Duration::from_millis(100)).unwrap() {}
        let output = session.readback().unwrap();
        assert_eq!((output.samples_min, output.samples_max), (8, 8));
        output.color.data
    };
    let mut cfg = RenderConfig::new(131, 79, 8);
    cfg.tile_size = 16;
    let mut small = RenderSession::new(renderer.clone(), &scene, &cfg).unwrap();
    let reference = render(&mut small);
    cfg.tile_size = 64;
    let mut large = RenderSession::new(renderer, &scene, &cfg).unwrap();
    assert_eq!(large.progress().tiles, 6);
    assert_eq!(reference, render(&mut large));
    large.reset(&scene).unwrap();
    assert_eq!(large.progress().tiles, 6);
    assert_eq!(reference, render(&mut large));
}
