use glam::Vec3;
use std::sync::Arc;
use syfrack_core::{Color, Light, NodeKind, Scene};
use syfrack_render::{GpuContext, RenderConfig, RenderSession, Renderer, SessionState};

#[test]
#[ignore = "needs GPU"]
fn intrinsically_transparent_gas_adds_radiance_and_leaves_dust_absorbing() {
    check_additive_gas(0.0);
}

#[test]
#[ignore = "needs GPU"]
fn spectral_filaments_remain_additive_finite_repeatable_and_absorbed() {
    check_additive_gas(1.0);
}

fn check_additive_gas(spectral_detail: f32) {
    let renderer = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = Scene::load(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scenes/environments/nebuleuse-du-cygne.ron"
    ))
    .unwrap();
    scene.fractal.graph_mut().nodes[1].kind = NodeKind::Plane {
        normal: Vec3::Y,
        offset: -100.0,
    };
    scene.lights = vec![Light::Environment {
        zenith: Color::Linear([0.0; 3]),
        horizon: Color::Linear([0.0; 3]),
        ground: Color::Linear([0.0; 3]),
        intensity: 0.0,
        rotation: 0.0,
    }];
    let n = scene.atmosphere.nebula.as_mut().unwrap();
    n.half_size = Vec3::new(19.0, 12.0, 3.5);
    n.steps = 48;
    n.density = 0.4;
    n.emission = 10.0;
    n.dust = 0.0;
    n.scattering = 0.3;
    n.core_energy = 0.0;
    n.stars = 0.0;
    let p = n.particles.as_mut().unwrap();
    p.filaments = true;
    p.smoke = false;
    p.remnant = true;
    p.resolution = 80;
    p.time = 0.0;
    p.cluster_stars = 0.0;
    p.gas_opacity = 0.0;
    p.depth_sampling = 0.3;
    p.spectral_detail = spectral_detail;
    let mut cfg = RenderConfig::new(65, 37, 2);
    cfg.max_bounces = 0;
    let emitted = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert!(emitted.color.data.iter().any(|v| v[0] > 0.01));
    assert!(emitted
        .color
        .data
        .iter()
        .flatten()
        .all(|v| v.is_finite() && *v >= 0.0));
    cfg.tile_size = 64;
    assert_eq!(
        emitted.color,
        renderer.render(&scene, &cfg, &mut |_| {}).unwrap().color
    );
    scene.atmosphere.nebula.as_mut().unwrap().density = 0.8;
    let doubled = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    for (a, b) in emitted.color.data.iter().zip(&doubled.color.data) {
        for c in 0..3 {
            assert!((b[c] - 2.0 * a[c]).abs() < (a[c].abs() * 1e-5).max(1e-7));
        }
    }
    scene.lights = vec![Light::Environment {
        zenith: Color::Linear([1.0; 3]),
        horizon: Color::Linear([1.0; 3]),
        ground: Color::Linear([1.0; 3]),
        intensity: 1.0,
        rotation: 0.0,
    }];
    scene.atmosphere.nebula.as_mut().unwrap().emission = 0.0;
    let clear = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert!(clear.color.data.iter().all(|v| v[..3] == [1.0; 3]));
    scene.atmosphere.nebula.as_mut().unwrap().dust = 10.0;
    let dusty = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert!(dusty.color.data.iter().any(|v| v[0] < 0.95));
    for v in &dusty.color.data {
        for c in 0..3 {
            assert!(v[c] >= 0.0 && v[c] <= 1.0 + 1e-6);
        }
    }
}

#[test]
#[ignore = "needs GPU"]
fn remnant_local_excitation_is_emissive_absorbed_and_occluded() {
    let renderer = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = Scene::load(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scenes/environments/nebuleuse-du-cygne.ron"
    ))
    .unwrap();
    scene.fractal.graph_mut().nodes[1].kind = NodeKind::Plane {
        normal: Vec3::Y,
        offset: -100.0,
    };
    let n = scene.atmosphere.nebula.as_mut().unwrap();
    n.half_size = Vec3::new(19.0, 12.0, 3.5);
    n.steps = 48;
    n.emission = 20.0;
    n.core_energy = 0.0;
    n.stars = 0.0;
    n.scattering = 0.0;
    n.dust = 0.0;
    let p = n.particles.as_mut().unwrap();
    p.resolution = 80;
    p.time = 0.0;
    p.filaments = true;
    p.smoke = false;
    p.remnant = true;
    p.convergence = 0.0;
    p.shock = 0.0;
    p.cluster_stars = 0.0;
    let mut cfg = RenderConfig::new(97, 55, 2);
    cfg.max_bounces = 0;
    cfg.tile_size = 32;
    let unexcited = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    scene
        .atmosphere
        .nebula
        .as_mut()
        .unwrap()
        .particles
        .as_mut()
        .unwrap()
        .convergence = 5.0;
    let encounters = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert!(unexcited
        .color
        .data
        .iter()
        .zip(&encounters.color.data)
        .any(|(a, b)| b[0] - a[0] > 0.01));
    for (a, b) in unexcited.color.data.iter().zip(&encounters.color.data) {
        for c in 0..3 {
            assert!(b[c].is_finite() && b[c] >= a[c] - 1e-5);
        }
    }
    scene
        .atmosphere
        .nebula
        .as_mut()
        .unwrap()
        .particles
        .as_mut()
        .unwrap()
        .shock = 2.0;
    let shocks = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert!(encounters
        .color
        .data
        .iter()
        .zip(&shocks.color.data)
        .any(|(a, b)| b[0] - a[0] > 0.01));
    cfg.tile_size = 64;
    assert_eq!(
        shocks.color,
        renderer.render(&scene, &cfg, &mut |_| {}).unwrap().color
    );
    scene.atmosphere.nebula.as_mut().unwrap().dust = 12.0;
    let dusty = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert!(shocks
        .color
        .data
        .iter()
        .zip(&dusty.color.data)
        .any(|(a, b)| a[0] - b[0] > 0.01));
    for (a, b) in shocks.color.data.iter().zip(&dusty.color.data) {
        for c in 0..3 {
            assert!(b[c].is_finite() && b[c] >= 0.0 && b[c] <= a[c] + 1e-5);
        }
    }
    scene.fractal.graph_mut().nodes[1].kind = NodeKind::Plane {
        normal: Vec3::Z,
        offset: 5.0,
    };
    let covered = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert!(covered.color.data.iter().all(|v| v[..3] == [0.0; 3]));
}

#[test]
#[ignore = "needs GPU"]
fn branching_storm_obeys_beer_absorption_and_tile_invariance() {
    let renderer = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = Scene::load(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scenes/environments/nebuleuse-du-cygne.ron"
    ))
    .unwrap();
    let n = scene.atmosphere.nebula.as_mut().unwrap();
    n.steps = 32;
    n.emission = 18.0;
    n.core_energy = 0.0;
    n.stars = 0.0;
    n.scattering = 0.0;
    n.dust = 0.0;
    let p = n.particles.as_mut().unwrap();
    p.resolution = 64;
    p.time = 0.125;
    p.storm = true;
    p.spider = false;
    p.cluster_stars = 0.0;
    let mut cfg = RenderConfig::new(97, 55, 2);
    cfg.max_bounces = 0;
    cfg.tile_size = 32;
    let clear = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert!(clear
        .color
        .data
        .iter()
        .flatten()
        .all(|v| v.is_finite() && *v >= 0.0));
    assert!(clear.color.data.iter().any(|v| v[2] > 0.02));
    cfg.tile_size = 64;
    assert_eq!(
        clear.color,
        renderer.render(&scene, &cfg, &mut |_| {}).unwrap().color
    );
    scene.atmosphere.nebula.as_mut().unwrap().dust = 12.0;
    let dusty = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    let mut absorbed = false;
    for (a, b) in clear.color.data.iter().zip(&dusty.color.data) {
        for c in 0..3 {
            assert!(
                b[c].is_finite() && b[c] >= 0.0 && b[c] <= a[c] + 1e-5,
                "channel {c}: clear={a:?}, dusty={b:?}"
            );
            absorbed |= a[c] - b[c] > 0.01;
        }
    }
    assert!(
        absorbed,
        "dust must occlude emitted gas in the branching streams"
    );
    // Near-zero optical depths must retain their emission. This also checks
    // the optically thin limit without relying on a particular colour/pixel.
    let n = scene.atmosphere.nebula.as_mut().unwrap();
    n.dust = 0.0;
    n.density = 1e-6;
    let thin = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert!(thin
        .color
        .data
        .iter()
        .any(|p| p[..3].iter().any(|v| *v > 1e-9)));
    scene.atmosphere.nebula.as_mut().unwrap().density = 2e-6;
    let doubled = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    for (a, b) in thin.color.data.iter().zip(&doubled.color.data) {
        for c in 0..3 {
            assert!(
                (b[c] - 2.0 * a[c]).abs() <= (a[c] * 1e-4).max(1e-10),
                "thin-limit channel {c}: {a:?} -> {b:?}"
            );
        }
    }
}

#[test]
#[ignore = "needs GPU"]
fn nebula_is_finite_repeatable_absorbing_and_occluded_by_geometry() {
    let renderer = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = Scene::load(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scenes/environments/nebuleuse-du-cygne.ron"
    ))
    .unwrap();
    scene.atmosphere.nebula.as_mut().unwrap().steps = 32;
    let particles = scene
        .atmosphere
        .nebula
        .as_mut()
        .unwrap()
        .particles
        .as_mut()
        .unwrap();
    particles.count = 4096;
    particles.filaments = true;
    particles.resolution = 64;
    particles.time = 0.125;
    let mut cfg = RenderConfig::new(65, 37, 2);
    cfg.max_bounces = 0;
    cfg.tile_size = 32;
    let a = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert!(a
        .color
        .data
        .iter()
        .flatten()
        .all(|v| v.is_finite() && *v >= 0.0));
    assert!(a.color.data.iter().any(|p| p[0] > 0.1));
    cfg.tile_size = 64;
    let b = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert_eq!(a.color, b.color);
    assert_eq!(a.depth, b.depth);
    assert_eq!(a.normals, b.normals);
    // A resumed snapshot must use the same generated medium. Changes to CPU
    // particle transport invalidate accumulated pixels just like shader edits.
    let dir = std::env::temp_dir().join(format!("syfrack-particle-state-{}", std::process::id()));
    let mut session = RenderSession::new(renderer.clone(), &scene, &cfg).unwrap();
    assert!(!session.step(std::time::Duration::from_millis(1)).unwrap());
    session.save(&dir).unwrap();
    drop(session);
    let mut resumed = RenderSession::resume(renderer.clone(), &dir).unwrap();
    while !resumed.step(std::time::Duration::from_millis(5)).unwrap() {}
    assert_eq!(resumed.readback().unwrap().color, a.color);
    drop(resumed);
    let state_path = dir.join("state.ron");
    let mut state: SessionState =
        ron::from_str(&std::fs::read_to_string(&state_path).unwrap()).unwrap();
    assert_ne!(state.particle_fingerprint, 0);
    state.particle_fingerprint ^= 1;
    std::fs::write(&state_path, ron::to_string(&state).unwrap()).unwrap();
    assert!(RenderSession::resume(renderer.clone(), &dir).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
    scene.atmosphere.nebula.as_mut().unwrap().seed += 1;
    assert_ne!(
        a.color,
        renderer.render(&scene, &cfg, &mut |_| {}).unwrap().color
    );

    // The opaque plane is in front of the entire cloud: no glow or distant
    // stars may be composited through it.
    scene.fractal.graph_mut().nodes[1].kind = NodeKind::Plane {
        normal: Vec3::Z,
        offset: 5.0,
    };
    let covered = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert!(covered.color.data.iter().all(|p| p[..3] == [0.0; 3]));

    scene.fractal.graph_mut().nodes[1].kind = NodeKind::Plane {
        normal: Vec3::Y,
        offset: -100.0,
    };
    scene.lights = vec![Light::Environment {
        zenith: Color::Linear([1.0; 3]),
        horizon: Color::Linear([1.0; 3]),
        ground: Color::Linear([1.0; 3]),
        intensity: 1.0,
        rotation: 0.0,
    }];
    let n = scene.atmosphere.nebula.as_mut().unwrap();
    n.emission = 0.0;
    n.core_energy = 0.0;
    n.stars = 0.0;
    let absorbed = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert!(absorbed.color.data.iter().any(|p| p[0] < 0.2));
    // Non-emissive particles must be visible through external illumination,
    // even when the internal core is disabled. Intensity edits reuse the
    // view-independent visibility cache and scale linear radiance.
    scene.lights = vec![Light::Sun {
        direction: Vec3::Z,
        color: Color::Linear([1.0; 3]),
        intensity: 1.0,
        angular_size: 1.0,
    }];
    let lit = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert!(lit.color.data.iter().any(|p| p[0] > 1e-4));
    if let Light::Sun { intensity, .. } = &mut scene.lights[0] {
        *intensity = 2.0;
    }
    let doubled = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    for (a, b) in lit.color.data.iter().zip(&doubled.color.data) {
        for c in 0..3 {
            assert!((a[c] * 2.0 - b[c]).abs() < 1e-6);
        }
    }
    scene.lights = vec![Light::Environment {
        zenith: Color::Linear([1.0; 3]),
        horizon: Color::Linear([1.0; 3]),
        ground: Color::Linear([1.0; 3]),
        intensity: 1.0,
        rotation: 0.0,
    }];
    scene.atmosphere.nebula = None;
    let clear = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert!(clear.color.data.iter().all(|p| p[..3] == [1.0; 3]));
}

#[test]
#[ignore = "needs GPU"]
fn spider_combines_emissive_stars_and_an_absorbing_volume() {
    let renderer = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = Scene::load(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scenes/environments/nebuleuse-de-laraignee.ron"
    ))
    .unwrap();
    let n = scene.atmosphere.nebula.as_mut().unwrap();
    n.steps = 32;
    let p = n.particles.as_mut().unwrap();
    p.resolution = 64;
    p.time = 0.0;
    let mut cfg = RenderConfig::new(65, 37, 2);
    cfg.max_bounces = 0;
    let image = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert!(image
        .color
        .data
        .iter()
        .flatten()
        .all(|v| v.is_finite() && *v >= 0.0));
    scene.atmosphere.nebula = None;
    let stars = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert!(stars.color.data.iter().any(|p| p[2] > 1.0));
    assert_ne!(image.color, stars.color);
}

#[test]
#[ignore = "needs GPU"]
fn ionized_fronts_and_finite_cluster_obey_visibility_and_occlusion() {
    let renderer = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = Scene::load(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scenes/environments/nebuleuse-de-laraignee.ron"
    ))
    .unwrap();
    // Reuse the minimal Input -> Plane -> Output graph. The general example
    // contains a point-valued trap chain and cannot accept a distance node here.
    scene.fractal = Scene::load(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scenes/environments/nebuleuse-du-cygne.ron"
    ))
    .unwrap()
    .fractal;
    scene.fractal.graph_mut().nodes[1].kind = NodeKind::Plane {
        normal: Vec3::Y,
        offset: -100.0,
    };
    let n = scene.atmosphere.nebula.as_mut().unwrap();
    n.steps = 32;
    n.emission = 0.0;
    n.core_energy = 0.0;
    n.stars = 0.0;
    n.dust = 0.0;
    let p = n.particles.as_mut().unwrap();
    p.resolution = 64;
    p.time = 0.0;
    p.cluster_stars = 2.0;
    let mut cfg = RenderConfig::new(129, 73, 2);
    cfg.max_bounces = 0;
    cfg.tile_size = 32;
    let clear = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert!(clear.color.data.iter().any(|v| v[2] > 0.05));
    cfg.tile_size = 64;
    assert_eq!(
        clear.color,
        renderer.render(&scene, &cfg, &mut |_| {}).unwrap().color
    );
    scene.atmosphere.nebula.as_mut().unwrap().dust = 20.0;
    let dusty = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert!(clear
        .color
        .data
        .iter()
        .zip(&dusty.color.data)
        .any(|(a, b)| a[2] - b[2] > 1e-4));
    for (a, b) in clear.color.data.iter().zip(&dusty.color.data) {
        for c in 0..3 {
            assert!(b[c] >= 0.0 && b[c].is_finite() && b[c] <= a[c] + 1e-5);
        }
    }
    // Exposed fronts respond to the source; emission from the molecular
    // interior must not replace visibility or create a halo behind occluders.
    let n = scene.atmosphere.nebula.as_mut().unwrap();
    n.particles.as_mut().unwrap().cluster_stars = 0.0;
    n.emission = 8.0;
    let dark = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    scene.atmosphere.nebula.as_mut().unwrap().core_energy = 12.0;
    let lit = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert!(lit
        .color
        .data
        .iter()
        .zip(&dark.color.data)
        .any(|(a, b)| a[0] - b[0] > 0.05));
    for (a, b) in lit.color.data.iter().zip(&dark.color.data) {
        for c in 0..3 {
            assert!(a[c].is_finite() && a[c] >= b[c] - 1e-5);
        }
    }
    scene
        .atmosphere
        .nebula
        .as_mut()
        .unwrap()
        .particles
        .as_mut()
        .unwrap()
        .cluster_stars = 2.0;
    scene.fractal.graph_mut().nodes[1].kind = NodeKind::Plane {
        normal: Vec3::Z,
        offset: 5.0,
    };
    let covered = renderer.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert!(covered.color.data.iter().all(|v| v[..3] == [0.0; 3]));
}
