use std::sync::Arc;
use std::time::Duration;

use syfrack_app_lib::preview::Preview;
use syfrack_core::Scene;
use syfrack_render::{GpuContext, Renderer};

/// The exhibition scene has fog: some first events are scattering points.
fn exhibition_preview() -> Preview {
    let renderer = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenes/exhibition.ron");
    let scene = Scene::load(path).unwrap();
    let mut preview = Preview::new(renderer, &scene, (256, 144)).unwrap();
    while preview.samples_min() < 1 {
        preview.tick(Duration::from_millis(50)).unwrap();
    }
    preview
}

#[test]
#[ignore = "needs GPU"]
fn every_pick_is_a_surface_with_steps_and_a_unit_normal() {
    let preview = exhibition_preview();
    let mut surfaces = 0;
    let mut fog_only = 0;
    for j in 0..18 {
        for i in 0..32 {
            let (u, v) = ((i as f32 + 0.5) / 32.0, (j as f32 + 0.5) / 18.0);
            match preview.pick(u, v).unwrap() {
                Some(p) => {
                    surfaces += 1;
                    assert!(p.depth > 0.0);
                    assert!(p.steps >= 1.0, "steps {} at ({u}, {v})", p.steps);
                    let n = glam::Vec3::from(p.normal);
                    assert!((n.length() - 1.0).abs() < 1e-3, "normal {n} at ({u}, {v})");
                }
                None => {
                    if preview.depth(u, v).unwrap().is_some() {
                        fog_only += 1;
                    }
                }
            }
        }
    }
    eprintln!("surfaces {surfaces}, fog scattering only {fog_only}");
    assert!(surfaces > 0, "the exhibition shows the fractal");
}
