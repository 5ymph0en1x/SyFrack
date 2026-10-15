use std::{sync::Arc, time::Duration};
use syfrack_app_lib::preview::Preview;
use syfrack_core::Scene;
use syfrack_render::{GpuContext, Renderer};

#[test]
#[ignore = "needs GPU"]
fn softening_neon_keeps_the_converged_preview_and_updates_exposure() {
    let renderer = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = Scene::example_2d();
    let mut preview = Preview::new(renderer, &scene, (128, 80)).unwrap();
    while preview.samples_min() == 0 {
        preview.tick(Duration::from_millis(30)).unwrap();
    }
    let before = preview.samples_min();
    scene.look.neon_softness = 0.9;
    scene.look.exposure_ev = 0.5;
    preview.set_scene(&scene).unwrap();
    assert_eq!(
        preview.samples_min(),
        before,
        "colour grading must not recalculate the fractal"
    );
    assert!((preview.exposure_gain() - 2_f32.sqrt()).abs() < 1e-6);
}
