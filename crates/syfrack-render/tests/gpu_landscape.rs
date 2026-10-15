//! Transport, ground continuity and foreground occlusion for the desert portal.
use glam::Vec3;
use std::sync::Arc;
use syfrack_core::{
    Atmosphere, CameraMode, Color, Dunes, Finish, Fractal, Graph, GroundPlane, Light, PortalWorld,
    Scene,
};
use syfrack_render::{GpuContext, RenderConfig, Renderer};

#[test]
#[ignore = "needs GPU"]
fn dunes_are_continuous_and_portal_respects_the_aperture_and_foreground() {
    let renderer = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = Scene::example();
    scene.fractal = Fractal::Graph(Graph::new_minimal());
    scene.atmosphere = Atmosphere::none();
    scene.lights = vec![Light::Environment {
        zenith: Color::Linear([0.0; 3]),
        horizon: Color::Linear([0.0; 3]),
        ground: Color::Linear([0.0; 3]),
        intensity: 1.0,
        rotation: 0.0,
    }];
    scene.camera.mode = CameraMode::Free {
        position: Vec3::new(10.0, 8.0, 14.0),
        target: Vec3::new(10.0, 0.0, 0.0),
        roll: 0.0,
    };
    scene.camera.focal_mm = 75.0;
    scene.camera.f_number = 0.0;
    scene.material.finishes = vec![Finish {
        albedo: [0.0; 3],
        specular: 0.0,
        emission: [0.2, 0.1, 0.05],
        ..Finish::default()
    }];
    scene.atmosphere.ground = Some(GroundPlane {
        height: 0.0,
        finish: 1,
        dunes: Some(Dunes::default()),
    });
    let mut cfg = RenderConfig::new(32, 24, 2);
    cfg.max_bounces = 0;
    cfg.pixel_tolerance = 0.125;
    let render = |s: &Scene| renderer.render(s, &cfg, &mut |_| {}).unwrap();
    let dunes = render(&scene);
    assert!(dunes
        .depth
        .iter()
        .all(|d| d.is_finite() && *d > 1.0 && *d < 30.0));
    assert!(dunes
        .normals
        .iter()
        .all(|n| (Vec3::new(n[0], n[1], n[2]).length() - 1.0).abs() < 1e-4));
    assert!(dunes
        .normals
        .iter()
        .any(|n| n[0].abs() > 0.04 || n[2].abs() > 0.04));
    assert!(
        dunes.color.data.iter().all(|p| (p[0] - 0.2).abs() < 1e-5),
        "terrain must not leak sky through exhausted height queries"
    );
    scene.camera.mode = CameraMode::Free {
        position: Vec3::new(100.0, 12.0, 200.0),
        target: Vec3::new(100.0, -3.0, -300.0),
        roll: 0.0,
    };
    scene.camera.focal_mm = 500.0;
    let horizon = render(&scene);
    assert!(
        horizon.color.data.iter().all(|p| (p[0] - 0.2).abs() < 1e-5),
        "grazing rays must not expose holes in distant dunes"
    );

    scene.atmosphere.ground = None;
    scene.lights = vec![Light::Environment {
        zenith: Color::Linear([0.0; 3]),
        horizon: Color::Linear([0.0; 3]),
        ground: Color::Linear([0.0; 3]),
        intensity: 1.0,
        rotation: 0.0,
    }];
    scene.camera.mode = CameraMode::Free {
        position: Vec3::new(8.0, 5.0, 10.0),
        target: Vec3::new(8.0, 5.0, 0.0),
        roll: 0.0,
    };
    scene.camera.focal_mm = 28.0;
    scene.atmosphere.portal = Some(PortalWorld {
        center: Vec3::new(8.0, 5.0, 0.0),
        radius: 2.0,
        ..PortalWorld::default()
    });
    let portal = render(&scene);
    let center = 12 * 32 + 16;
    assert!(portal.color.data[center][2] > portal.color.data[center][0] * 3.0);
    assert!(
        portal.color.data[0][..3].iter().all(|v| *v == 0.0),
        "outside the circular aperture must remain in the original world"
    );
    let mut absorbing = scene.clone();
    absorbing.atmosphere.fog_density = 10.0;
    absorbing.atmosphere.fog_falloff = 0.0;
    absorbing.atmosphere.fog_color = [0.0; 3];
    let fogged = render(&absorbing);
    assert!(
        fogged
            .color
            .data
            .iter()
            .all(|p| p[..3].iter().all(|v| *v == 0.0)),
        "the membrane must not bypass absorption in front of the aperture"
    );
    scene.atmosphere.portal.as_mut().unwrap().radius = 0.4;
    scene.atmosphere.portal.as_mut().unwrap().center = Vec3::ZERO;
    scene.camera.mode = CameraMode::Free {
        position: Vec3::new(0.0, 0.0, 4.0),
        target: Vec3::ZERO,
        roll: 0.0,
    };
    scene.camera.focal_mm = 120.0;
    scene.material.albedo = [0.0; 3];
    scene.material.specular = 0.0;
    let occluded = render(&scene);
    assert!(
        occluded
            .color
            .data
            .iter()
            .all(|p| p[..3].iter().all(|v| *v == 0.0)),
        "the foreground sphere must occlude the portal behind it"
    );

    // With no other illumination and no continuation bounce, only explicit
    // sampling of the membrane can illuminate this ground-only camera view.
    scene.camera.mode = CameraMode::Free {
        position: Vec3::new(8.0, 4.0, 8.0),
        target: Vec3::new(8.0, 0.0, 3.0),
        roll: 0.0,
    };
    scene.camera.focal_mm = 75.0;
    scene.material.albedo = [0.5; 3];
    scene.atmosphere.ground = Some(GroundPlane::default());
    scene.atmosphere.portal = Some(PortalWorld {
        center: Vec3::new(8.0, 3.0, 0.0),
        radius: 2.0,
        energy: 0.0,
        ..PortalWorld::default()
    });
    let unlit = render(&scene);
    assert!(unlit
        .color
        .data
        .iter()
        .all(|p| p[..3].iter().all(|v| *v == 0.0)));
    scene.atmosphere.portal.as_mut().unwrap().energy = 2.0;
    let spill = render(&scene);
    assert!(spill
        .color
        .data
        .iter()
        .all(|p| p[..3].iter().all(|v| v.is_finite() && *v >= 0.0)));
    let sum = spill
        .color
        .data
        .iter()
        .fold(Vec3::ZERO, |acc, p| acc + Vec3::new(p[0], p[1], p[2]));
    assert!(
        sum.z > 0.1 && sum.z > sum.x * 3.0,
        "the membrane must cast blue illumination onto the ground"
    );
}
