use syfrack_core::{Dimension, Scene, Tonemap};

#[test]
fn canvas_projects_are_saved_with_their_mode_and_need_no_lights() {
    let mut scene = Scene::example_2d();
    scene.canvas.center = [-0.3, 0.08];
    scene.canvas.zoom = 1.25;
    scene.canvas.rotation = 23.0;
    scene.validate().unwrap();
    assert!(scene.lights.is_empty());
    assert_eq!(Scene::from_ron(&scene.to_ron()).unwrap(), scene);
    assert_eq!(scene.dimension, Dimension::TwoD);
    assert_eq!(scene.look.tonemap, Tonemap::Direct);
    scene.material.texture = None;
    assert!(
        scene.validate().is_err(),
        "missing motif should not create an empty canvas"
    );
}

#[test]
fn canvas_rotation_and_zoom_preserve_the_world_point_at_the_centre() {
    let mut canvas = syfrack_core::Canvas2D::default();
    canvas.center = [0.7, -0.4];
    canvas.zoom = 3.0;
    canvas.rotation = 90.0;
    assert!((canvas.point([0.5, 0.5], 1.6) - glam::Vec2::new(0.7, -0.4)).length() < 1e-6);
    let right = canvas.point([1.0, 0.5], 1.6);
    assert!((right - glam::Vec2::new(0.7, 0.6)).length() < 1e-6);
}
