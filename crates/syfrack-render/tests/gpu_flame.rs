use std::{sync::Arc, time::Duration};
use syfrack_core::{Flame, Scene};
use syfrack_render::{GpuContext, RenderConfig, RenderSession, Renderer};

#[test]
#[ignore = "needs GPU"]
fn flame_session_checkpoint_matches_uninterrupted_native_render() {
    let renderer = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = Scene::example_2d();
    scene.canvas.flame = Some(Flame::default());
    let mut cfg = RenderConfig::new(32, 24, 5);
    cfg.samples_per_dispatch = 2;
    let mut full = RenderSession::new(renderer.clone(), &scene, &cfg).unwrap();
    while !full.step(Duration::from_millis(10)).unwrap() {}
    let reference = full.readback().unwrap();
    let mut partial = RenderSession::new(renderer.clone(), &scene, &cfg).unwrap();
    assert!(!partial.step(Duration::from_millis(10)).unwrap());
    while partial.progress().samples_min < 2 {
        partial.step(Duration::from_millis(10)).unwrap();
    }
    let folder =
        std::env::temp_dir().join(format!("syfrack-flame-checkpoint-{}", std::process::id()));
    partial.save(&folder).unwrap();
    let mut resumed = RenderSession::resume(renderer, &folder).unwrap();
    while !resumed.step(Duration::from_millis(10)).unwrap() {}
    let result = resumed.readback().unwrap();
    assert_eq!(reference.color.data, result.color.data);
    assert_eq!((result.samples_min, result.samples_max), (5, 5));
    assert_eq!(
        syfrack_render::denoise::session_output(&resumed, &result).data,
        result.color.data
    );
    assert!(result.color.data.iter().flatten().all(|v| v.is_finite()));
}

#[test]
#[ignore = "needs GPU"]
fn editing_a_flame_discards_the_previous_background_pass() {
    let renderer = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = Scene::example_for("flame").unwrap();
    let mut cfg = RenderConfig::new(96, 64, 4);
    cfg.samples_per_dispatch = 2;
    let mut session = RenderSession::new(renderer.clone(), &scene, &cfg).unwrap();
    session.step(Duration::ZERO).unwrap();
    scene.canvas.center = [0.25, -0.12];
    scene.seed = 42;
    session.reset(&scene).unwrap();
    while !session.step(Duration::from_millis(10)).unwrap() {}
    let edited = session.readback().unwrap();
    let reference = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert_eq!(edited.color.data, reference.color.data);
    assert_eq!((edited.samples_min, edited.samples_max), (4, 4));
}
