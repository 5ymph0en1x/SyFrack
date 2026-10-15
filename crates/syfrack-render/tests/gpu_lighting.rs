//! Transport regressions: visible emitters, matching area/BSDF PDFs and finite
//! visibility segments. Run with --ignored --test-threads=1 on a GPU host.
use glam::Vec3;
use std::sync::Arc;
use syfrack_core::{Atmosphere, CameraMode, Color, Fractal, Graph, Light, Look, NodeKind, Scene};
use syfrack_render::{
    GpuContext, RenderConfig, RenderOutput, RenderSession, Renderer, SessionState,
};

fn renderer() -> Renderer {
    Renderer::new(Arc::new(GpuContext::new().expect("GPU"))).unwrap()
}

fn scene(kind: NodeKind) -> Scene {
    let mut s = Scene::example();
    let mut g = Graph::new_minimal();
    g.node_mut(1).unwrap().kind = kind;
    s.fractal = Fractal::Graph(g);
    s.camera.mode = CameraMode::Free {
        position: Vec3::new(0.0, 0.0, 3.0),
        target: Vec3::ZERO,
        roll: 0.0,
    };
    s.camera.focal_mm = 500.0;
    s.camera.f_number = 0.0;
    s.material.albedo = [0.5; 3];
    s.material.specular = 0.0;
    s.material.metallic = 0.0;
    s.material.coloring = syfrack_core::Coloring::None;
    s.material.texture = None;
    s.material.surface = None;
    s.lights.clear();
    s.atmosphere = Atmosphere::none();
    s.look = Look::neutral();
    s
}

fn area(position: Vec3, normal: Vec3, size: [f32; 2], intensity: f32) -> Light {
    Light::Area {
        position,
        normal,
        size,
        color: Color::Linear([1.0; 3]),
        intensity,
    }
}

fn render(r: &Renderer, s: &Scene, samples: u32, bounces: u32) -> RenderOutput {
    let mut cfg = RenderConfig::new(32, 32, samples);
    cfg.max_bounces = bounces;
    r.render(s, &cfg, &mut |_| {}).unwrap()
}

fn centre(out: &RenderOutput) -> f32 {
    (12..20)
        .flat_map(|y| (12..20).map(move |x| out.color.get(x, y)[0]))
        .sum::<f32>()
        / 64.0
}

#[test]
#[ignore = "needs GPU"]
fn area_camera_sees_front_radiance_and_opaque_black_back() {
    let r = renderer();
    let mut s = scene(NodeKind::Sphere { radius: 0.01 });
    s.camera.mode = CameraMode::Free {
        position: Vec3::new(0.0, 0.0, 5.0),
        target: Vec3::new(0.0, 0.0, 6.0),
        roll: 0.0,
    };
    let mut lamp = area(Vec3::new(0.0, 0.0, 7.0), Vec3::NEG_Z, [2.0; 2], 4.0);
    if let Light::Area { color, .. } = &mut lamp {
        *color = Color::Linear([0.5, 0.25, 1.0]);
    }
    s.lights.push(lamp);
    s.lights.push(Light::Environment {
        zenith: Color::Linear([1.0; 3]),
        horizon: Color::Linear([1.0; 3]),
        ground: Color::Linear([1.0; 3]),
        intensity: 1.0,
        rotation: 0.0,
    });
    let front = render(&r, &s, 4, 0);
    for p in &front.color.data {
        for (actual, expected) in p[..3].iter().zip([2.0, 1.0, 4.0]) {
            assert!((actual - expected).abs() < 1e-5, "{p:?}");
        }
    }
    assert!(front.depth.iter().all(|d| (d - 2.0).abs() < 0.01));
    assert!(front.normals.iter().all(|n| (n[2] + 1.0).abs() < 1e-5));
    if let Light::Area { normal, .. } = &mut s.lights[0] {
        *normal = Vec3::Z;
    }
    let back = render(&r, &s, 4, 0);
    assert!(back.color.data.iter().all(|p| p[..3] == [0.0; 3]));
}

#[test]
#[ignore = "needs GPU"]
fn area_is_hidden_by_a_nearer_fractal_surface() {
    let r = renderer();
    let mut s = scene(NodeKind::Sphere { radius: 1.0 });
    s.camera.focal_mm = 50.0;
    s.lights
        .push(area(Vec3::new(0.0, 0.0, -2.0), Vec3::Z, [8.0; 2], 4.0));
    let out = render(&r, &s, 8, 0);
    assert_eq!(centre(&out), 0.0, "sphere occludes the emitter");
    assert!((out.color.get(0, 0)[0] - 4.0).abs() < 1e-5);
    assert!((out.depth[16 * 32 + 16] - 2.0).abs() < 0.02);
}

#[test]
#[ignore = "needs GPU"]
fn area_segment_stops_before_geometry_behind_the_emitter() {
    let r = renderer();
    let mut s = scene(NodeKind::Plane {
        normal: Vec3::Z,
        offset: 10.0,
    });
    s.camera.mode = CameraMode::Free {
        position: Vec3::ZERO,
        target: Vec3::Z,
        roll: 0.0,
    };
    s.lights
        .push(area(Vec3::new(0.0, 0.0, 2.0), Vec3::NEG_Z, [2.0; 2], 3.0));
    let mut cfg = RenderConfig::new(32, 32, 1);
    cfg.max_steps = 1;
    cfg.max_bounces = 0;
    let out = r.render(&s, &cfg, &mut |_| {}).unwrap();
    assert!(out.color.data.iter().all(|p| (p[0] - 3.0).abs() < 1e-5));
}

#[test]
#[ignore = "needs GPU"]
fn area_shadow_query_does_not_exhaust_beyond_the_sampled_light() {
    let r = renderer();
    let mut s = scene(NodeKind::Plane {
        normal: Vec3::Z,
        offset: 0.0,
    });
    s.lights
        .push(area(Vec3::new(0.0, 0.0, 4.0), Vec3::NEG_Z, [0.1; 2], 4.0));
    let mut cfg = RenderConfig::new(32, 32, 8);
    cfg.max_bounces = 0;
    let reference = r.render(&s, &cfg, &mut |_| {}).unwrap();
    cfg.max_steps = 8;
    let bounded = r.render(&s, &cfg, &mut |_| {}).unwrap();
    assert!(centre(&reference) > 0.0);
    assert_eq!(reference.color.data, bounded.color.data);
}

#[test]
#[ignore = "needs GPU"]
fn opaque_rectangle_blocks_area_and_sun_shadow_rays() {
    let r = renderer();
    let mut s = scene(NodeKind::Plane {
        normal: Vec3::Z,
        offset: 0.0,
    });
    s.camera.mode = CameraMode::Free {
        position: Vec3::new(6.0, 0.0, 3.0),
        target: Vec3::ZERO,
        roll: 0.0,
    };
    let mut blocker = area(Vec3::new(0.0, 0.0, 2.0), Vec3::Z, [4.0; 2], 1.0);
    if let Light::Area { color, .. } = &mut blocker {
        *color = Color::Linear([0.0; 3]);
    }
    for source in [
        area(Vec3::new(0.0, 0.0, 4.0), Vec3::NEG_Z, [1.0; 2], 4.0),
        Light::Sun {
            direction: Vec3::Z,
            color: Color::Linear([1.0; 3]),
            intensity: 2.0,
            angular_size: 0.5,
        },
    ] {
        s.lights = vec![source];
        let unblocked = render(&r, &s, 32, 0);
        assert!(centre(&unblocked) > 0.03);
        s.lights.push(blocker.clone());
        let blocked = render(&r, &s, 32, 0);
        assert_eq!(centre(&blocked), 0.0);
    }
}

fn mirror_scene() -> Scene {
    let mut s = scene(NodeKind::Plane {
        normal: Vec3::Z,
        offset: 0.0,
    });
    s.material.albedo = [0.8; 3];
    s.material.metallic = 1.0;
    s.material.roughness = 0.02;
    s.lights
        .push(area(Vec3::new(0.0, 0.0, 5.0), Vec3::NEG_Z, [4.0; 2], 3.0));
    s
}

#[test]
#[ignore = "needs GPU"]
fn area_mirror_reflection_recovers_radiance_without_fireflies() {
    let r = renderer();
    let out = render(&r, &mirror_scene(), 128, 1);
    let measured = centre(&out);
    // The narrow GGX lobe lies entirely inside the softbox: rho * Le = 2.4.
    eprintln!("softbox reflection {measured}, expected 2.4");
    assert!((measured - 2.4).abs() < 0.10);
    assert!(out
        .color
        .data
        .iter()
        .all(|p| p[0] < 3.0 && p[..3].iter().all(|v| v.is_finite())));
}

// Independent midpoint quadrature of a rectangle's irradiance at the origin.
// Receiver and emitter are parallel, so cos_receiver*cos_light/r^2 = z^2/r^4.
fn rectangle_irradiance(x: f64, z: f64, width: f64, height: f64, radiance: f64) -> f64 {
    let n = 128;
    let mut sum = 0.0;
    for iy in 0..n {
        for ix in 0..n {
            let px = x + ((ix as f64 + 0.5) / n as f64 - 0.5) * width;
            let py = ((iy as f64 + 0.5) / n as f64 - 0.5) * height;
            let r2 = px * px + py * py + z * z;
            sum += z * z / (r2 * r2);
        }
    }
    sum * width * height * radiance / (n * n) as f64
}

#[test]
#[ignore = "needs GPU"]
fn area_mis_matches_independent_quadrature_and_terminal_nee() {
    let r = renderer();
    let mut s = scene(NodeKind::Plane {
        normal: Vec3::Z,
        offset: 0.0,
    });
    // Unequal lights exercise the light-selection PMF in both MIS branches.
    s.lights.push(area(
        Vec3::new(-1.0, 0.0, 4.0),
        Vec3::NEG_Z,
        [1.0, 2.0],
        2.0,
    ));
    s.lights
        .push(area(Vec3::new(1.0, 0.0, 4.0), Vec3::NEG_Z, [2.0, 3.0], 5.0));
    let expected = (rectangle_irradiance(-1.0, 4.0, 1.0, 2.0, 2.0)
        + rectangle_irradiance(1.0, 4.0, 2.0, 3.0, 5.0))
        * 0.5
        / std::f64::consts::PI;
    let terminal = centre(&render(&r, &s, 512, 0));
    let mis = centre(&render(&r, &s, 512, 1));
    eprintln!("rectangle quadrature {expected}, terminal {terminal}, MIS {mis}");
    for actual in [terminal, mis] {
        assert!((f64::from(actual) - expected).abs() < expected * 0.04);
    }
    assert!((terminal - mis).abs() < terminal * 0.04);
}

#[test]
#[ignore = "needs GPU"]
fn area_coincident_emitters_preserve_camera_and_mis_radiance() {
    let r = renderer();
    let mut s = mirror_scene();
    let one = centre(&render(&r, &s, 128, 1));
    let mut lamp = s.lights[0].clone();
    if let Light::Area { intensity, .. } = &mut lamp {
        *intensity *= 0.5;
    }
    s.lights = vec![lamp.clone(), lamp];
    let pair = centre(&render(&r, &s, 128, 1));
    assert!((one - pair).abs() < one * 0.02, "one {one}, pair {pair}");
    s.camera.mode = CameraMode::Free {
        position: Vec3::new(0.0, 0.0, 3.0),
        target: Vec3::new(0.0, 0.0, 5.0),
        roll: 0.0,
    };
    assert!((centre(&render(&r, &s, 4, 0)) - 3.0).abs() < 1e-5);
}

#[test]
#[ignore = "needs GPU"]
fn area_reflection_is_attenuated_on_both_legs_in_absorbing_fog() {
    let r = renderer();
    let mut s = mirror_scene();
    s.atmosphere.fog_density = 0.15;
    s.atmosphere.fog_falloff = 0.0;
    s.atmosphere.fog_color = [0.0; 3];
    // Camera -> mirror: 3 units; mirror -> softbox: 5 units.
    let expected = 2.4 * (-0.15f32 * 8.0).exp();
    let measured = centre(&render(&r, &s, 512, 1));
    eprintln!("fog reflection {measured}, Beer-Lambert {expected}");
    assert!((measured - expected).abs() < expected * 0.07);
}

#[test]
#[ignore = "needs GPU"]
fn area_volume_mis_does_not_double_count_phase_sampled_emission() {
    let r = renderer();
    let mut s = scene(NodeKind::Plane {
        normal: Vec3::X,
        offset: 100.0,
    });
    s.camera.mode = CameraMode::Free {
        position: Vec3::ZERO,
        target: Vec3::Z,
        roll: 0.0,
    };
    s.lights
        .push(area(Vec3::new(3.0, 0.0, 3.0), Vec3::NEG_X, [20.0; 2], 5.0));
    s.atmosphere.fog_density = 0.001;
    s.atmosphere.fog_falloff = 0.0;
    // A broad lateral emitter makes phase sampling a significant strategy.
    // In optically thin fog one extra scatter contributes only a small tail.
    let terminal = centre(&render(&r, &s, 2048, 0));
    let mis = centre(&render(&r, &s, 2048, 1));
    eprintln!("thin fog area NEE {terminal}, phase MIS {mis}");
    assert!(terminal > 0.01);
    assert!(
        (0.90..1.15).contains(&(mis / terminal)),
        "terminal {terminal}, MIS {mis}"
    );
    s.lights.pop();
    for _ in 0..4 {
        s.lights
            .push(area(Vec3::new(3.0, 0.0, 3.0), Vec3::NEG_X, [20.0; 2], 1.25));
    }
    let split = centre(&render(&r, &s, 2048, 1));
    assert!(
        (0.85..1.15).contains(&(split / mis)),
        "volume light selection changed energy: single {mis}, split {split}"
    );
}

#[test]
#[ignore = "needs GPU"]
fn volume_rgb_lights_mix_in_empty_air_and_vanish_without_scattering() {
    let r = renderer();
    let mut s = scene(NodeKind::Plane {
        normal: Vec3::X,
        offset: 100.0,
    });
    s.camera.mode = CameraMode::Free {
        position: Vec3::ZERO,
        target: Vec3::Z,
        roll: 0.0,
    };
    for (x, color) in [(3.0, [1.0, 0.0, 0.0]), (-3.0, [0.0, 0.0, 1.0])] {
        s.lights.push(Light::Area {
            position: Vec3::new(x, 0.0, 3.0),
            normal: Vec3::new(-x.signum(), 0.0, 0.0),
            size: [10.0; 2],
            color: Color::Linear(color),
            intensity: 4.0,
        });
    }
    let clear = render(&r, &s, 4, 0);
    assert!(clear.color.data.iter().all(|p| p[..3] == [0.0; 3]));
    s.atmosphere.fog_density = 0.035;
    s.atmosphere.fog_falloff = 0.0;
    s.atmosphere.fog_color = [0.9; 3];
    s.atmosphere.anisotropy = 0.15;
    let mixed = render(&r, &s, 128, 0);
    assert!(mixed.color.data.iter().flatten().all(|v| v.is_finite()));
    let red: f32 = mixed.color.data.iter().map(|p| p[0]).sum();
    let blue: f32 = mixed.color.data.iter().map(|p| p[2]).sum();
    assert!(
        (0.9..1.1).contains(&(red / blue)),
        "symmetric fog energy: red {red}, blue {blue}"
    );
    assert!(mixed
        .color
        .data
        .iter()
        .all(|p| p[0] > 0.001 && p[2] > 0.001 && p[1] == 0.0));
    s.atmosphere.fog_color = [0.0; 3];
    let absorbing = render(&r, &s, 4, 0);
    assert!(absorbing.color.data.iter().all(|p| p[..3] == [0.0; 3]));
}

#[test]
#[ignore = "needs GPU"]
fn checkpoint_cannot_mix_samples_from_different_transport_shaders() {
    let r = renderer();
    let s = mirror_scene();
    let mut session = RenderSession::new(r.clone(), &s, &RenderConfig::new(64, 32, 4)).unwrap();
    session.step(std::time::Duration::ZERO).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "syfrack-lighting-checkpoint-{}",
        std::process::id()
    ));
    session.save(&dir).unwrap();
    let path = dir.join("state.ron");
    let original = std::fs::read_to_string(&path).unwrap();
    let mut state: SessionState = ron::from_str(&original).unwrap();
    assert!(state.tiles.iter().any(|tile| tile.done > 0));
    let fingerprint = state.shader_fingerprint.unwrap();
    let bytes = std::fs::read(dir.join("accum.bin")).unwrap();
    let legacy = original
        .lines()
        .filter(|line| !line.trim_start().starts_with("shader_fingerprint:"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(ron::from_str::<SessionState>(&legacy)
        .unwrap()
        .shader_fingerprint
        .is_none());
    std::fs::write(&path, legacy).unwrap();
    assert!(RenderSession::resume(r.clone(), &dir).is_err());
    for incompatible in [None, Some(fingerprint ^ 1)] {
        state.shader_fingerprint = incompatible;
        std::fs::write(&path, ron::to_string(&state).unwrap()).unwrap();
        let error = RenderSession::resume(r.clone(), &dir)
            .err()
            .expect("must reject");
        assert!(error
            .to_string()
            .contains("different or legacy lighting engine"));
        assert_eq!(std::fs::read(dir.join("accum.bin")).unwrap(), bytes);
    }
    state.shader_fingerprint = Some(fingerprint);
    // The untouched right half has four equal-progress siblings. Store it as
    // one tile, leaving the left half at startup size with nonuniform progress.
    state.tiles.retain(|tile| tile.x < 32);
    let mut right = state.tiles[0];
    right.x = 32;
    right.y = 0;
    right.w = 32;
    right.h = 32;
    right.done = 0;
    state.tiles.push(right);
    std::fs::write(&path, ron::to_string(&state).unwrap()).unwrap();
    let mut resumed = RenderSession::resume(r.clone(), &dir).unwrap();
    while !resumed.is_done() {
        resumed.step(std::time::Duration::from_millis(20)).unwrap();
    }
    let fresh = r
        .render(&s, &RenderConfig::new(64, 32, 4), &mut |_| {})
        .unwrap();
    assert_eq!(resumed.readback().unwrap().color.data, fresh.color.data);
    std::fs::remove_dir_all(&dir).unwrap();
}
