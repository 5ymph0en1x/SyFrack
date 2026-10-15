use glam::Vec3;
use syfrack_core::{Color, FinishDetail, FinishPattern, Light, Scene};

#[test]
fn neon_tube_roundtrips_and_exposes_its_actual_shape() {
    let mut scene = Scene::example();
    scene.lights = vec![Light::Tube {
        start: Vec3::new(-1.0, 1.0, 0.0),
        end: Vec3::new(-1.0, 5.0, 0.0),
        radius: 0.04,
        color: Color::Linear([1.0, 0.01, 0.003]),
        intensity: 40.0,
    }];
    scene.validate().unwrap();
    let paths = scene.params();
    assert!(paths.iter().any(|p| p.path == "lights[0].tube.radius"));
    assert_eq!(scene, Scene::from_ron(&scene.to_ron()).unwrap());
    if let Light::Tube { start, end, .. } = &mut scene.lights[0] {
        *end = *start;
    }
    assert!(scene.validate().is_err());
}

#[test]
fn concrete_is_saved_as_a_dedicated_surface_not_a_dust_deposit() {
    let detail = FinishDetail {
        pattern: FinishPattern::Concrete,
        ..FinishDetail::default()
    };
    let encoded = ron::to_string(&detail).unwrap();
    assert!(encoded.contains("Concrete"));
    assert_eq!(detail, ron::from_str(&encoded).unwrap());
}
