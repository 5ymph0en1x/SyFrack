use std::sync::Arc;
use std::time::{Duration, Instant};

use syfrack_app_lib::preview::Preview;
use syfrack_core::{clay_scene, Fractal, Graph, NodeKind, Scene};
use syfrack_render::graph_wgsl::fingerprint;
use syfrack_render::{GpuContext, Renderer};

fn with_graph(scene: &Scene, g: syfrack_core::Graph) -> Scene {
    let mut s = scene.clone();
    s.fractal = Fractal::Graph(g);
    s
}

/// Polls until the compilation under way is adopted (or fails), ticking the
/// old pipeline meanwhile as the app does.
fn wait_swap(preview: &mut Preview) -> Result<Duration, String> {
    let start = Instant::now();
    loop {
        if let Some(r) = preview.poll_compile() {
            return r;
        }
        assert!(
            start.elapsed() < Duration::from_secs(120),
            "compile timeout"
        );
        preview.tick(Duration::from_millis(5)).unwrap();
    }
}

#[test]
#[ignore = "needs GPU"]
fn hdri_assets_load_asynchronously_and_bad_files_leave_the_preview_editable() {
    use syfrack_core::{HdrImage, Light};
    let dir = std::env::temp_dir().join(format!("syfrack-hdri-preview-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let asset = dir.join("studio.exr");
    let mut pixels = HdrImage::new(16, 8);
    pixels.data.fill([2.0, 1.0, 0.5, 1.0]);
    syfrack_color::write_exr(&asset, &pixels).unwrap();
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = Scene::example();
    scene.lights = vec![Light::Hdri {
        path: asset.clone(),
        intensity: 1.0,
        rotation: 0.0,
    }];
    let mut preview = Preview::new(r, &scene, (96, 64)).unwrap();
    assert!(preview.compiling());
    assert!(preview.session().is_none());
    wait_swap(&mut preview).unwrap();
    let pipeline = preview.session().unwrap().fingerprint();
    scene.set_param("lights[0].hdri.rotation", 90.0).unwrap();
    preview.set_scene(&scene).unwrap();
    assert!(!preview.compiling());
    assert_eq!(preview.session().unwrap().fingerprint(), pipeline);
    let mut bad = scene.clone();
    if let Light::Hdri { path, .. } = &mut bad.lights[0] {
        *path = dir.join("missing.exr");
    }
    preview.set_scene(&bad).unwrap();
    assert!(preview.compiling());
    assert!(wait_swap(&mut preview).is_err());
    assert!(preview.session().is_some(), "keep the previous valid image");
    // An external content change has the same graph but different resources.
    pixels.data.fill([4.0, 0.5, 0.25, 1.0]);
    syfrack_color::write_exr(&asset, &pixels).unwrap();
    preview.set_scene(&scene).unwrap();
    assert!(preview.compiling());
    wait_swap(&mut preview).unwrap();
    assert_eq!(preview.session().unwrap().fingerprint(), pipeline);
    assert!(preview.known_failure().is_none());
    drop(preview);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "needs GPU"]
fn texture_changes_compile_asynchronously_and_scalar_edits_keep_the_pipeline() {
    use syfrack_core::FractalTexture;
    use syfrack_render::graph_wgsl::scene_fingerprint;
    let renderer = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = Scene::example();
    let mut preview = Preview::new(renderer, &scene, (96, 64)).unwrap();
    let plain = preview.session().unwrap().fingerprint();
    scene.material.texture = Some(FractalTexture::default());
    preview.set_scene(&scene).unwrap();
    assert!(preview.compiling());
    assert_eq!(preview.session().unwrap().fingerprint(), plain);
    // The last scalar edit during compilation must be carried into the adopted scene.
    scene.set_param("material.texture.bump", 0.7).unwrap();
    preview.set_scene(&scene).unwrap();
    wait_swap(&mut preview).unwrap();
    assert_eq!(preview.session().unwrap().scene(), &scene);
    assert_eq!(
        preview.session().unwrap().fingerprint(),
        scene_fingerprint(&scene)
    );
    scene.set_param("material.texture.scale", 9.0).unwrap();
    preview.set_scene(&scene).unwrap();
    assert!(!preview.compiling());
    scene.material.texture.as_mut().unwrap().graph = Graph::bundled("menger").unwrap();
    preview.set_scene(&scene).unwrap();
    wait_swap(&mut preview).unwrap();
    assert_eq!(
        preview.session().unwrap().fingerprint(),
        scene_fingerprint(&scene)
    );
}

#[test]
#[ignore = "needs GPU"]
fn a_topology_change_compiles_in_the_background_and_values_do_not() {
    let renderer = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let base = Scene::example();
    let a = Graph::bundled("mandelbox").unwrap();
    let scene_a = with_graph(&base, a.clone());
    let mut preview = Preview::new(renderer, &scene_a, (128, 72)).unwrap();
    let fp_a = preview.session().unwrap().fingerprint();
    assert_eq!(fp_a, fingerprint(&a));

    // A value: same pipeline, no compilation.
    let mut a2 = a.clone();
    for n in &mut a2.nodes {
        if let NodeKind::Scale { factor } = &mut n.kind {
            *factor = 2.4;
        }
    }
    preview.set_scene(&with_graph(&base, a2.clone())).unwrap();
    assert!(!preview.compiling());
    assert_eq!(preview.session().unwrap().fingerprint(), fp_a);
    // Clay is a values-only change too.
    preview
        .set_scene(&clay_scene(&with_graph(&base, a2.clone())))
        .unwrap();
    assert!(!preview.compiling());

    // A topology: requested, the old pipeline keeps rendering meanwhile.
    let b = Graph::bundled("mandelbulb").unwrap();
    preview.set_scene(&with_graph(&base, b.clone())).unwrap();
    assert!(preview.compiling());
    assert_eq!(
        preview.session().unwrap().fingerprint(),
        fp_a,
        "still the old one"
    );
    preview.tick(Duration::from_millis(10)).unwrap();
    // Superseded by another topology before it lands: only the last counts.
    let c = Graph::bundled("menger").unwrap();
    preview.set_scene(&with_graph(&base, c.clone())).unwrap();
    wait_swap(&mut preview).expect("compiled");
    assert!(!preview.compiling());
    assert_eq!(preview.session().unwrap().fingerprint(), fingerprint(&c));
    assert!(preview.last_compile_ms().is_some());
    while preview.samples_min() < 1 {
        preview.tick(Duration::from_millis(20)).unwrap();
    }

    // Back on the running topology while another compiles: the ticket is
    // dropped and nothing is swapped later.
    preview.set_scene(&with_graph(&base, b)).unwrap();
    assert!(preview.compiling());
    preview.set_scene(&with_graph(&base, c.clone())).unwrap();
    assert!(!preview.compiling(), "a stale compilation is dropped");
    assert!(preview.poll_compile().is_none());
    assert_eq!(preview.session().unwrap().fingerprint(), fingerprint(&c));
}

#[test]
#[ignore = "needs GPU"]
fn a_saved_form_gets_a_clay_thumbnail() {
    let renderer = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let dir = std::env::temp_dir().join(format!("syfrack-thumb-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let g = Graph::bundled("mandelbox").unwrap();
    let ron = syfrack_app_lib::library::save(&dir, "Boîte", &g).unwrap();
    let png = ron.with_extension("png");
    syfrack_app_lib::library::render_thumbnail(&renderer, &g, &png).unwrap();
    let (w, h, _) = syfrack_app_lib::gallery::thumbnail(&png, 320).unwrap();
    assert_eq!((w, h), (320, 180));
    let entries = syfrack_app_lib::library::scan(&dir);
    assert_eq!(entries[0].name, "Boîte");
    assert_eq!(entries[0].png.as_deref(), Some(png.as_path()));
    std::fs::remove_dir_all(&dir).unwrap();
}
