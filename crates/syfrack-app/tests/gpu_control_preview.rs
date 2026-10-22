use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use syfrack_app_lib::preview::Preview;
use syfrack_core::{Fractal, Graph, Scene};
use syfrack_render::{GpuContext, Renderer};

#[test]
#[ignore = "needs GPU"]
fn natural_control_starts_asynchronously_coalesces_edits_and_preserves_specialization() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = syfrack_core::scene::phenomena::presets::scene("interference").unwrap();
    scene.phenomena.as_mut().unwrap().domain.resolution = [12, 12, 12];
    let start = Instant::now();
    let mut preview = Preview::new_control(r, &scene, (64, 48)).unwrap();
    assert!(start.elapsed() < Duration::from_secs(1));
    assert!(preview.preparing_fields());
    preview.pause_fields(true);
    scene.phenomena.as_mut().unwrap().time = 0.25;
    preview.set_scene(&scene).unwrap();
    preview.pause_fields(false);
    let deadline = Instant::now() + Duration::from_secs(300);
    while preview.compiling() {
        preview.poll_compile().transpose().unwrap();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(preview.is_control());
    assert!(preview.inactive_reason().is_none());
    preview.request_specialization();
    while !preview.is_specialized() {
        preview.poll_compile().transpose().unwrap();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    scene.phenomena.as_mut().unwrap().time = 0.5;
    preview.set_scene(&scene).unwrap();
    while preview.compiling() {
        preview.poll_compile().transpose().unwrap();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(preview.is_specialized());
}

#[test]
#[ignore = "needs GPU"]
fn native_compilation_is_explicit_and_stale_requests_cannot_replace_the_new_graph() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = Scene::example();
    scene.fractal = Fractal::Graph(Graph::new_minimal());
    let mut p = Preview::new_control(r.clone(), &scene, (128, 72)).unwrap();
    assert!(p.is_control());
    assert!(!p.compiling());
    assert!(p.session().is_none());
    for name in ["mandelbox", "menger", "sphere"] {
        scene.fractal = Fractal::Graph(Graph::bundled(name).unwrap());
        p.set_scene(&scene).unwrap();
        assert!(p.is_control());
        assert!(!p.compiling());
        assert_eq!(r.cached_pipelines(), 0);
    }
    p.request_specialization();
    assert!(p.compiling());
    assert!(p.is_control());
    scene.fractal = Fractal::Graph(Graph::bundled("menger").unwrap());
    p.set_scene(&scene).unwrap();
    assert!(!p.specialization_requested());
    let deadline = Instant::now() + Duration::from_secs(120);
    while p.compiling() {
        p.tick(Duration::from_millis(5)).unwrap();
        p.poll_compile().transpose().unwrap();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(p.is_control());
    assert!(
        p.session().is_none(),
        "stale sphere compilation must not replace menger"
    );
    p.request_specialization();
    while !p.is_specialized() {
        p.tick(Duration::from_millis(5)).unwrap();
        p.poll_compile().transpose().unwrap();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!p.is_control());
    assert_eq!(p.session().unwrap().scene().fractal, scene.fractal);
    p.use_control().unwrap();
    assert!(p.is_control());
    let valid_size = p.size();
    let mut invalid = scene.clone();
    let Fractal::Graph(g) = &mut invalid.fractal;
    g.links.clear();
    p.set_scene(&invalid).unwrap();
    assert!(p.active());
    assert!(p.inactive_reason().is_some());
    assert_eq!(p.size(), valid_size);
    p.resize((256, 144)).unwrap();
    assert_eq!(
        p.size(),
        valid_size,
        "resize must retain the last valid image"
    );
    p.set_scene(&scene).unwrap();
    assert!(p.inactive_reason().is_none());
    assert!(p.is_control());
    assert_eq!(p.size(), (64, 36));
}

#[test]
#[ignore = "needs GPU"]
fn control_default_preserves_the_native_2d_workshop() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let scene = Scene::example_2d();
    let mut preview = Preview::new_control(r, &scene, (128, 80)).unwrap();
    assert!(!preview.is_control());
    assert!(!preview.can_specialize());
    assert!(preview.session().is_some());
    while preview.samples_min() == 0 {
        preview.tick(Duration::from_millis(30)).unwrap();
    }
    let before = preview.samples_min();
    let mut graded = scene.clone();
    graded.look.exposure_ev = 0.5;
    preview.set_scene(&graded).unwrap();
    assert_eq!(preview.samples_min(), before);
    assert!((preview.exposure_gain() - 2_f32.sqrt()).abs() < 1e-6);
}
