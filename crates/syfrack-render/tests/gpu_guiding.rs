//! Guided transport energy, determinism, persistence and invalidation.
use glam::Vec3;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use syfrack_core::{
    Atmosphere, CameraMode, Color, Coloring, Fractal, Graph, HdrImage, Light, NodeKind, Scene,
};
use syfrack_render::{GpuContext, RenderConfig, RenderOutput, RenderSession, Renderer};

struct Tmp(PathBuf);
impl Tmp {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("syfrack-guiding-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn renderer() -> Renderer {
    Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap()
}
fn plane() -> Scene {
    let mut s = Scene::example();
    let mut graph = Graph::new_minimal();
    graph.node_mut(1).unwrap().kind = NodeKind::Plane {
        normal: Vec3::Z,
        offset: 0.0,
    };
    s.fractal = Fractal::Graph(graph);
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
    s.material.coloring = Coloring::None;
    s.material.texture = None;
    s.material.surface = None;
    s.atmosphere = Atmosphere::none();
    s.lights = vec![Light::Environment {
        zenith: Color::Linear([1.0; 3]),
        horizon: Color::Linear([1.0; 3]),
        ground: Color::Linear([1.0; 3]),
        intensity: 2.0,
        rotation: 0.0,
    }];
    s
}
fn cfg(spp: u32) -> RenderConfig {
    let mut cfg = RenderConfig::new(32, 32, spp);
    cfg.max_bounces = 2;
    cfg.guiding = true;
    cfg
}
fn finish(session: &mut RenderSession) -> RenderOutput {
    while !session.is_done() {
        session.step(Duration::from_millis(250)).unwrap();
    }
    session.readback().unwrap()
}
fn centre(out: &RenderOutput) -> f64 {
    (12..20)
        .flat_map(|y| (12..20).map(move |x| out.color.get(x, y)[0] as f64))
        .sum::<f64>()
        / 64.0
}

#[test]
#[ignore = "needs GPU"]
fn guided_diffuse_furnace_conserves_energy_and_caches_are_bounded() {
    let r = renderer();
    let s = plane();
    let mut session = RenderSession::new(r, &s, &cfg(1024)).unwrap();
    let out = finish(&mut session);
    let measured = centre(&out);
    eprintln!("guided BSDF furnace {measured}, expected 1");
    assert!((measured - 1.0).abs() < 0.02);
    let stats = session.guiding_stats();
    assert_eq!(stats.updates, 5);
    assert!(stats.active_cells > 0);
    assert_eq!(stats.memory_bytes, syfrack_render::guiding::BYTES);
    assert!(out.normals.iter().all(|n| n[2] > 0.99 && n[3] >= 1.0));
    assert!(out
        .color
        .data
        .iter()
        .all(|p| p[..3].iter().all(|v| v.is_finite())));
}

#[test]
#[ignore = "needs GPU"]
fn finishing_at_a_training_boundary_commits_the_proposal_with_a_zero_budget() {
    let r = renderer();
    let s = plane();
    let mut session = RenderSession::new(r, &s, &cfg(16)).unwrap();
    while !session.is_done() {
        session.step(Duration::ZERO).unwrap();
    }
    assert_eq!(session.guiding_stats().updates, 1);
    assert!(session.guiding_stats().active_cells > 0);
}

#[test]
#[ignore = "needs GPU"]
fn untrusted_guiding_variants_keep_terminal_surface_samples_bit_identical() {
    let r = renderer();
    let s = plane();
    let mut config = cfg(512);
    config.max_bounces = 0;
    let mut guided = RenderSession::new(r.clone(), &s, &config).unwrap();
    let out = finish(&mut guided);
    assert_eq!(guided.guiding_stats().updates, 5);
    assert_eq!(guided.guiding_stats().trusted_cells, 0);
    let profile = guided.guiding_profile();
    assert!(profile.stages[5].dispatches > 0);
    assert_eq!(
        profile.stages.iter().map(|s| s.pixel_samples).sum::<u64>(),
        32 * 32 * 512
    );
    assert!(profile
        .updates
        .iter()
        .all(|u| u.readback_bytes > 0 && u.upload_bytes > 0));
    config.guiding = false;
    let mut plain = RenderSession::new(r, &s, &config).unwrap();
    assert_eq!(out.color.data, finish(&mut plain).color.data);
    guided.reset(&s).unwrap();
    assert_eq!(
        guided
            .guiding_profile()
            .stages
            .iter()
            .map(|s| s.dispatches)
            .sum::<u32>(),
        0
    );
}

#[test]
#[ignore = "needs GPU"]
fn guided_hdri_and_sun_mis_preserve_their_integrals() {
    let tmp = Tmp::new();
    let file = tmp.0.join("furnace.exr");
    let mut image = HdrImage::new(16, 8);
    image.data.fill([2.0; 4]);
    syfrack_color::write_exr(&file, &image).unwrap();
    let r = renderer();
    let mut s = plane();
    for (light, expected) in [
        (
            Light::Hdri {
                path: file,
                intensity: 1.0,
                rotation: 45.0,
            },
            1.0,
        ),
        (
            Light::Sun {
                direction: Vec3::Z,
                color: Color::Linear([1.0; 3]),
                intensity: 4.0,
                angular_size: 60.0,
            },
            2.0 / std::f64::consts::PI * (1.0 + 30f64.to_radians().cos()) / 2.0,
        ),
    ] {
        s.lights = vec![light];
        let mut session = RenderSession::new(r.clone(), &s, &cfg(2048)).unwrap();
        let out = finish(&mut session);
        let measured = centre(&out);
        eprintln!("guided light MIS {measured}, expected {expected}");
        assert!(session.guiding_stats().active_cells > 0);
        assert!((measured - expected).abs() < 0.02 * expected);
    }
}

fn rectangle_irradiance(x: f64, z: f64, w: f64, h: f64, radiance: f64) -> f64 {
    let n = 128;
    let mut sum = 0.0;
    for iy in 0..n {
        for ix in 0..n {
            let px = x + ((ix as f64 + 0.5) / n as f64 - 0.5) * w;
            let py = ((iy as f64 + 0.5) / n as f64 - 0.5) * h;
            let r2 = px * px + py * py + z * z;
            sum += z * z / (r2 * r2);
        }
    }
    sum * w * h * radiance / (n * n) as f64
}

#[test]
#[ignore = "needs GPU"]
fn guided_area_mis_matches_independent_quadrature() {
    let r = renderer();
    let mut s = plane();
    s.lights = vec![Light::Area {
        position: Vec3::new(1.0, 0.0, 4.0),
        normal: Vec3::NEG_Z,
        size: [2.0, 3.0],
        color: Color::Linear([1.0; 3]),
        intensity: 5.0,
    }];
    let expected = rectangle_irradiance(1.0, 4.0, 2.0, 3.0, 5.0) * 0.5 / std::f64::consts::PI;
    let mut session = RenderSession::new(r, &s, &cfg(2048)).unwrap();
    let out = finish(&mut session);
    let measured = centre(&out);
    eprintln!("guided rectangle {measured}, quadrature {expected}");
    assert!(session.guiding_stats().active_cells > 0);
    assert!(
        session.guiding_stats().trusted_cells > 0,
        "MIS must exercise accepted guide proposals"
    );
    assert!((measured - expected).abs() < 0.02 * expected);
}

#[test]
#[ignore = "needs GPU"]
fn cache_and_image_are_independent_of_tile_size_and_dispatch_chunking() {
    let r = renderer();
    let s = plane();
    let mut cfg = cfg(96);
    cfg.width = 128;
    cfg.height = 80; // exceeds the pool size: exercise strided spatial sampling
    cfg.tile_size = 32;
    cfg.samples_per_dispatch = 1;
    let mut a = RenderSession::new(r.clone(), &s, &cfg).unwrap();
    let out_a = finish(&mut a);
    cfg.tile_size = 128;
    cfg.samples_per_dispatch = 8;
    let mut b = RenderSession::new(r, &s, &cfg).unwrap();
    let out_b = finish(&mut b);
    assert!(a.guiding_stats().active_cells > 0);
    assert_eq!(out_a.color.data, out_b.color.data);
    let tmp = Tmp::new();
    a.save(&tmp.0.join("a")).unwrap();
    b.save(&tmp.0.join("b")).unwrap();
    assert_eq!(
        std::fs::read(tmp.0.join("a/guiding.bin")).unwrap(),
        std::fs::read(tmp.0.join("b/guiding.bin")).unwrap()
    );
}

#[test]
#[ignore = "needs GPU"]
fn resume_preserves_training_and_frozen_proposal_and_rejects_corruption() {
    let r = renderer();
    let s = plane();
    let tmp = Tmp::new();
    let dir = tmp.0.join("state");
    let mut session = RenderSession::new(r.clone(), &s, &cfg(96)).unwrap();
    while session.progress().samples_min < 23 {
        session.step(Duration::ZERO).unwrap();
    }
    assert!(session.guiding_stats().active_cells > 0);
    session.save(&dir).unwrap();
    let state_path = dir.join("state.ron");
    let saved = std::fs::read_to_string(&state_path).unwrap();
    let mut state: syfrack_render::SessionState = ron::from_str(&saved).unwrap();
    state.guiding_version += 1;
    std::fs::write(&state_path, ron::to_string(&state).unwrap()).unwrap();
    let error = RenderSession::resume(r.clone(), &dir)
        .err()
        .expect("incompatible model rejected");
    assert!(error.to_string().contains("guiding algorithm version"));
    std::fs::write(&state_path, saved).unwrap();
    let mut resumed = RenderSession::resume(r.clone(), &dir).unwrap();
    let uninterrupted = finish(&mut session);
    let restarted = finish(&mut resumed);
    assert_eq!(uninterrupted.color.data, restarted.color.data);
    let mut bytes = std::fs::read(dir.join("guiding.bin")).unwrap();
    bytes[100] ^= 1;
    std::fs::write(dir.join("guiding.bin"), &bytes).unwrap();
    let error = RenderSession::resume(r, &dir)
        .err()
        .expect("damaged guide rejected");
    assert!(error.to_string().contains("corrupted"));
    assert_eq!(std::fs::read(dir.join("guiding.bin")).unwrap(), bytes);
}

#[test]
#[ignore = "needs GPU"]
fn resetting_lighting_clears_the_learned_guide_and_dark_regions_stay_finite() {
    let r = renderer();
    let mut s = plane();
    let mut session = RenderSession::new(r, &s, &cfg(96)).unwrap();
    finish(&mut session);
    assert!(session.guiding_stats().active_cells > 0);
    if let Light::Environment { intensity, .. } = &mut s.lights[0] {
        *intensity = 0.0;
    }
    session.reset(&s).unwrap();
    assert_eq!(session.guiding_stats().updates, 0);
    assert_eq!(session.guiding_stats().active_cells, 0);
    let dark = finish(&mut session);
    assert_eq!(session.guiding_stats().active_cells, 0);
    assert!(dark.color.data.iter().all(|p| p[..3] == [0.0; 3]));
}

#[test]
#[ignore = "needs GPU"]
fn guided_surface_paths_retain_absorbing_fog_on_both_segments() {
    let r = renderer();
    let mut s = plane();
    s.atmosphere.fog_density = 0.001;
    s.atmosphere.fog_falloff = 0.0;
    s.atmosphere.fog_color = [0.0; 3];
    let mut session = RenderSession::new(r, &s, &cfg(4096)).unwrap();
    let out = finish(&mut session);
    // Three camera units, then the renderer's 100-unit environment segment.
    let expected = (-0.103f64).exp();
    let measured = centre(&out);
    eprintln!("guided absorbing fog {measured}, expected {expected}");
    assert!(session.guiding_stats().active_cells > 0);
    assert!((measured - expected).abs() < 0.015);
}

#[test]
#[ignore = "needs GPU"]
fn guided_rough_metal_matches_independent_ggx_disk_quadrature() {
    let r = renderer();
    let mut s = plane();
    s.material.albedo = [0.8; 3];
    s.material.metallic = 1.0;
    s.material.roughness = 0.5;
    s.lights = vec![Light::Sun {
        direction: Vec3::Z,
        color: Color::Linear([1.0; 3]),
        intensity: 4.0,
        angular_size: 60.0,
    }];
    // At normal incidence the azimuth integral cancels. For irradiance 4,
    // Le * BRDF * cosine / sun_pdf = F * D * G over the uniform disk.
    let n = 4096;
    let mu0 = 30f64.to_radians().cos();
    let a2 = 0.5f64.powi(4);
    let mut expected = 0.0;
    for i in 0..n {
        let mu = mu0 + (1.0 - mu0) * (i as f64 + 0.5) / n as f64;
        let nh2 = (1.0 + mu) / 2.0;
        let f = 0.8 + 0.2 * (1.0 - nh2.sqrt()).powi(5);
        let d = a2 / (std::f64::consts::PI * (nh2 * (a2 - 1.0) + 1.0).powi(2));
        let g = 2.0 * mu / (mu + (a2 + (1.0 - a2) * mu * mu).sqrt());
        expected += f * d * g / n as f64;
    }
    let mut session = RenderSession::new(r, &s, &cfg(2048)).unwrap();
    let out = finish(&mut session);
    let measured = centre(&out);
    eprintln!("guided rough metal {measured}, quadrature {expected}");
    assert!(session.guiding_stats().active_cells > 0);
    assert!((measured - expected).abs() < 0.02 * expected);
}
