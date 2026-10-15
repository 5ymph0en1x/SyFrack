use std::sync::Arc;
use std::time::Duration;

use syfrack_app_lib::preview::Preview;
use syfrack_core::{Fractal, Graph, Scene};
use syfrack_render::{GpuContext, Renderer};

#[test]
#[ignore = "needs GPU"]
fn an_incomplete_graph_leaves_the_preview_inactive_until_it_is_wired() {
    let renderer = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = Scene::example();
    let mut g = Graph::new_minimal();
    g.links.retain(|l| l.to.0 != 2);
    scene.fractal = Fractal::Graph(g.clone());
    let mut preview = Preview::new(renderer, &scene, (128, 72)).expect("Ok while inactive");
    assert!(preview.inactive_reason().is_some());
    assert!(preview.session().is_none());
    preview.tick(Duration::from_millis(10)).unwrap();
    assert!(preview.pick(0.5, 0.5).unwrap().is_none());
    assert!(preview.depth(0.5, 0.5).unwrap().is_none());
    preview.resize((256, 144)).unwrap();
    // Wiring the hole activates it.
    scene.fractal = Fractal::Graph(Graph::new_minimal());
    preview.set_scene(&scene).unwrap();
    assert!(preview.inactive_reason().is_none());
    // Its pipeline compiles in the background: the session comes once it lands.
    let start = std::time::Instant::now();
    while preview.poll_compile().is_none() {
        assert!(
            start.elapsed() < Duration::from_secs(120),
            "compile timeout"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(preview.session().is_some());
    while preview.samples_min() < 1 {
        preview.tick(Duration::from_millis(50)).unwrap();
    }
    // The resize made while inactive holds: quarter or (promoted) half of it.
    assert!(
        [(64, 36), (128, 72)].contains(&preview.size()),
        "{:?}",
        preview.size()
    );
    // And breaking it again deactivates it.
    scene.fractal = Fractal::Graph(g);
    preview.set_scene(&scene).unwrap();
    assert!(preview.inactive_reason().is_some());
}

#[test]
#[ignore = "needs GPU"]
fn a_valid_form_reactivates_the_preview_without_compiling_on_the_caller() {
    let renderer = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = Scene::example();
    let mut g = Graph::new_minimal();
    g.links.retain(|l| l.to.0 != 2);
    scene.fractal = Fractal::Graph(g);
    let mut preview = Preview::new(renderer.clone(), &scene, (128, 72)).unwrap();
    assert!(preview.inactive_reason().is_some());
    // A topology the renderer has never compiled.
    let menger = Graph::bundled("menger").unwrap();
    assert_eq!(renderer.cached_pipelines(), 0);
    scene.fractal = Fractal::Graph(menger);
    preview.set_scene(&scene).unwrap();
    assert!(preview.compiling(), "requested, not built on the caller");
    assert!(preview.session().is_none());
    assert!(
        preview.inactive_reason().is_none(),
        "no longer incomplete while it compiles"
    );
    let start = std::time::Instant::now();
    let adopted = loop {
        if let Some(r) = preview.poll_compile() {
            break r;
        }
        assert!(
            start.elapsed() < Duration::from_secs(120),
            "compile timeout"
        );
        preview.tick(Duration::from_millis(5)).unwrap();
    };
    adopted.expect("compiled");
    assert!(preview.session().is_some(), "activated");
    assert!(!preview.compiling());
    assert!(preview.last_compile_ms().is_some());
    while preview.samples_min() < 1 {
        preview.tick(Duration::from_millis(20)).unwrap();
    }
}
