//! Panorama energy, seam/rotation, MIS and checkpoint regressions.
use glam::Vec3;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use syfrack_core::{
    Atmosphere, CameraMode, Coloring, Fractal, Graph, HdrImage, Light, NodeKind, Scene,
};
use syfrack_render::{GpuContext, RenderConfig, RenderOutput, RenderSession, Renderer};

struct Tmp(PathBuf);
impl Tmp {
    fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "syfrack-hdri-{}-{nonce}-{name}",
            std::process::id()
        ));
        std::fs::create_dir(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn map(dir: &Path, name: &str, w: u32, h: u32, pixel: impl Fn(u32, u32) -> [f32; 3]) -> PathBuf {
    let mut image = HdrImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let p = pixel(x, y);
            image.set(x, y, [p[0], p[1], p[2], 1.0]);
        }
    }
    let path = dir.join(name);
    syfrack_color::write_exr(&path, &image).unwrap();
    path
}
fn renderer() -> Renderer {
    Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap()
}
fn scene(path: PathBuf, kind: NodeKind) -> Scene {
    let mut s = Scene::example();
    let mut graph = Graph::new_minimal();
    graph.node_mut(1).unwrap().kind = kind;
    s.fractal = Fractal::Graph(graph);
    s.camera.mode = CameraMode::Free {
        position: Vec3::new(0.0, 0.0, 3.0),
        target: Vec3::ZERO,
        roll: 0.0,
    };
    s.camera.focal_mm = 500.0;
    s.camera.f_number = 0.0;
    s.material.albedo = [0.5; 3];
    s.material.metallic = 0.0;
    s.material.specular = 0.0;
    s.material.coloring = Coloring::None;
    s.material.texture = None;
    s.material.surface = None;
    s.atmosphere = Atmosphere::none();
    s.lights = vec![Light::Hdri {
        path,
        intensity: 1.0,
        rotation: 0.0,
    }];
    s
}
fn forward(s: &mut Scene, direction: Vec3) {
    let position = Vec3::new(0.0, 0.0, 3.0);
    s.camera.mode = CameraMode::Free {
        position,
        target: position + direction,
        roll: 0.0,
    };
}
fn render(r: &Renderer, s: &Scene, spp: u32, bounces: u32) -> RenderOutput {
    let mut cfg = RenderConfig::new(32, 32, spp);
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
fn native_hdr_and_exr_decode_as_linear_radiance() {
    let tmp = Tmp::new("decode");
    let hdr = tmp.0.join("native.hdr");
    image::codecs::hdr::HdrEncoder::new(std::fs::File::create(&hdr).unwrap())
        .encode(&vec![image::Rgb([4.0, 0.5, 2.0]); 32], 8, 4)
        .unwrap();
    let exr = map(&tmp.0, "native.exr", 8, 4, |_, _| [4.0, 0.5, 2.0]);
    for file in [hdr, exr] {
        let env = syfrack_render::environment::EnvironmentMap::load(&file).unwrap();
        assert_eq!((env.width, env.height), (8, 4));
        for p in env.texels {
            assert!((p[0] - 4.0).abs() < 0.04 && (p[1] - 0.5).abs() < 0.01);
        }
    }
    assert!(syfrack_render::environment::EnvironmentMap::load(&tmp.0.join("missing.exr")).is_err());
}

#[test]
#[ignore = "needs GPU"]
fn camera_preserves_hdr_values_and_intensity() {
    let tmp = Tmp::new("camera");
    let file = map(&tmp.0, "constant.exr", 16, 8, |_, _| [2.0, 0.25, 4.0]);
    let r = renderer();
    let mut s = scene(file, NodeKind::Sphere { radius: 0.01 });
    forward(&mut s, Vec3::Z);
    if let Light::Hdri { intensity, .. } = &mut s.lights[0] {
        *intensity = 2.0;
    }
    let out = render(&r, &s, 4, 0);
    for p in out.color.data {
        for (actual, expected) in p[..3].iter().zip([4.0, 0.5, 8.0]) {
            assert!((actual - expected).abs() < 1e-5, "{p:?}");
        }
    }
}

#[test]
#[ignore = "needs GPU"]
fn uniform_panorama_obeys_the_diffuse_furnace_with_and_without_mis() {
    let tmp = Tmp::new("furnace");
    let file = map(&tmp.0, "constant.exr", 32, 16, |_, _| [4.0; 3]);
    let r = renderer();
    let s = scene(
        file,
        NodeKind::Plane {
            normal: Vec3::Z,
            offset: 0.0,
        },
    );
    for bounces in [0, 1] {
        let measured = centre(&render(&r, &s, 256, bounces));
        eprintln!("HDRI diffuse furnace bounces={bounces}: {measured}, expected 2");
        assert!((measured - 2.0).abs() < 0.06);
    }
    let mut split = s.clone();
    if let Light::Hdri { intensity, .. } = &mut split.lights[0] {
        *intensity = 0.5;
    }
    split.lights.push(split.lights[0].clone());
    assert!((centre(&render(&r, &split, 256, 1)) - 2.0).abs() < 0.06);
}

// Independent bilinear lookup and spherical quadrature of the test panorama.
fn lookup(pixels: &[[f32; 3]], w: u32, h: u32, u: f64, v: f64) -> f64 {
    let x = u * f64::from(w) - 0.5;
    let y = v * f64::from(h) - 0.5;
    let sample = |ix: i32, iy: i32| {
        f64::from(
            pixels
                [(iy.clamp(0, h as i32 - 1) as u32 * w + ix.rem_euclid(w as i32) as u32) as usize]
                [0],
        )
    };
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    (sample(ix, iy) * (1.0 - fx) + sample(ix + 1, iy) * fx) * (1.0 - fy)
        + (sample(ix, iy + 1) * (1.0 - fx) + sample(ix + 1, iy + 1) * fx) * fy
}
fn plane_reference(pixels: &[[f32; 3]], w: u32, h: u32) -> f64 {
    let mut irradiance = 0.0;
    let pi = std::f64::consts::PI;
    for y in 0..512 {
        let theta = pi * (y as f64 + 0.5) / 512.0;
        let row_area = (pi * y as f64 / 512.0).cos() - (pi * (y + 1) as f64 / 512.0).cos();
        for x in 0..1024 {
            let phi = 2.0 * pi * (x as f64 + 0.5) / 1024.0;
            irradiance += lookup(pixels, w, h, phi / (2.0 * pi), theta / pi)
                * (theta.sin() * phi.sin()).max(0.0)
                * row_area
                * 2.0
                * pi
                / 1024.0;
        }
    }
    irradiance * 0.5 / pi
}

#[test]
#[ignore = "needs GPU"]
fn bright_small_source_matches_quadrature_without_double_counting() {
    let tmp = Tmp::new("bright-patch");
    let (w, h) = (64u32, 32u32);
    let mut pixels = vec![[0.01; 3]; (w * h) as usize];
    for y in 13..17 {
        for x in 15..17 {
            pixels[(y * w + x) as usize] = [100.0; 3];
        }
    }
    let file = map(&tmp.0, "patch.exr", w, h, |x, y| {
        pixels[(y * w + x) as usize]
    });
    let expected = plane_reference(&pixels, w, h);
    let r = renderer();
    let s = scene(
        file,
        NodeKind::Plane {
            normal: Vec3::Z,
            offset: 0.0,
        },
    );
    for bounces in [0, 1] {
        let measured = centre(&render(&r, &s, 512, bounces));
        eprintln!("HDRI patch bounces={bounces}: {measured}, quadrature {expected}");
        assert!((f64::from(measured) - expected).abs() < expected * 0.035);
    }
    // Rotating the receiver and panorama together must preserve irradiance.
    let mut rotated = s.clone();
    rotated.fractal.graph_mut().node_mut(1).unwrap().kind = NodeKind::Plane {
        normal: Vec3::NEG_X,
        offset: 0.0,
    };
    rotated.camera.mode = CameraMode::Free {
        position: Vec3::new(-3.0, 0.0, 0.0),
        target: Vec3::ZERO,
        roll: 0.0,
    };
    if let Light::Hdri { rotation, .. } = &mut rotated.lights[0] {
        *rotation = 90.0;
    }
    let measured = centre(&render(&r, &rotated, 512, 1));
    assert!((f64::from(measured) - expected).abs() < expected * 0.035);
}

#[test]
#[ignore = "needs GPU"]
fn rotation_is_consistent_and_the_longitude_seam_is_continuous() {
    let tmp = Tmp::new("rotation");
    let file = map(&tmp.0, "harmonic.exr", 64, 32, |x, y| {
        let phi = 2.0 * std::f32::consts::PI * (x as f32 + 0.5) / 64.0;
        let theta = std::f32::consts::PI * (y as f32 + 0.5) / 32.0;
        [1.0 + 0.75 * phi.cos() * theta.sin(); 3]
    });
    let r = renderer();
    let mut s = scene(file, NodeKind::Sphere { radius: 0.01 });
    forward(&mut s, Vec3::X);
    let at_x = centre(&render(&r, &s, 4, 0));
    forward(&mut s, Vec3::Z);
    let at_z = centre(&render(&r, &s, 4, 0));
    assert!(at_x > 1.7 && (at_z - 1.0).abs() < 0.01);
    if let Light::Hdri { rotation, .. } = &mut s.lights[0] {
        *rotation = 90.0;
    }
    assert!((centre(&render(&r, &s, 4, 0)) - at_x).abs() < 0.002);
    if let Light::Hdri { rotation, .. } = &mut s.lights[0] {
        *rotation = 0.0;
    }
    forward(&mut s, Vec3::new(1.0, 0.0, -0.0001));
    let left = centre(&render(&r, &s, 4, 0));
    forward(&mut s, Vec3::new(1.0, 0.0, 0.0001));
    let right = centre(&render(&r, &s, 4, 0));
    assert!((left - right).abs() < 0.001);
}

#[test]
#[ignore = "needs GPU"]
fn glossy_reflection_restores_panorama_radiance_and_fog_attenuation() {
    let tmp = Tmp::new("mirror");
    let file = map(&tmp.0, "constant.exr", 32, 16, |_, _| [3.0; 3]);
    let r = renderer();
    let mut s = scene(
        file,
        NodeKind::Plane {
            normal: Vec3::Z,
            offset: 0.0,
        },
    );
    s.material.metallic = 1.0;
    s.material.albedo = [0.8; 3];
    s.material.roughness = 0.02;
    let clean = centre(&render(&r, &s, 256, 1));
    assert!((clean - 2.4).abs() < 0.1, "{clean}");
    s.atmosphere.fog_density = 0.01;
    s.atmosphere.fog_falloff = 0.0;
    s.atmosphere.fog_color = [0.0; 3];
    let expected = 2.4 * (-0.01f32 * (3.0 + 100.0)).exp();
    let measured = centre(&render(&r, &s, 1024, 1));
    eprintln!("HDRI fog mirror {measured}, Beer-Lambert {expected}");
    assert!((measured - expected).abs() < expected * 0.07);
}

#[test]
#[ignore = "needs GPU"]
fn phase_mis_in_thin_fog_preserves_white_environment_energy() {
    let tmp = Tmp::new("phase");
    let file = map(&tmp.0, "constant.exr", 32, 16, |_, _| [1.0; 3]);
    let r = renderer();
    let mut s = scene(file, NodeKind::Sphere { radius: 0.01 });
    forward(&mut s, Vec3::Z);
    s.atmosphere.fog_density = 0.001;
    s.atmosphere.fog_falloff = 0.0;
    s.atmosphere.anisotropy = 0.3;
    for bounces in [0, 1, 4] {
        let measured = centre(&render(&r, &s, 1024, bounces));
        eprintln!("HDRI scattering furnace bounces={bounces}: {measured}");
        assert!((measured - 1.0).abs() < 0.025);
    }
}

#[test]
#[ignore = "needs GPU"]
fn asset_edits_reset_on_the_same_pipeline_and_changed_files_refuse_resume() {
    let tmp = Tmp::new("resume");
    let file = map(&tmp.0, "constant.exr", 16, 8, |_, _| [2.0; 3]);
    let r = renderer();
    let mut s = scene(
        file,
        NodeKind::Plane {
            normal: Vec3::Z,
            offset: 0.0,
        },
    );
    let cfg = RenderConfig::new(32, 32, 8);
    let mut session = RenderSession::new(r.clone(), &s, &cfg).unwrap();
    let pipeline = session.fingerprint();
    if let Light::Hdri {
        intensity,
        rotation,
        ..
    } = &mut s.lights[0]
    {
        *intensity = 0.5;
        *rotation = 90.0;
    }
    assert!(session.set_scene_params(&s).unwrap());
    assert_eq!(pipeline, session.fingerprint());
    let mut missing = s.clone();
    if let Light::Hdri { path, .. } = &mut missing.lights[0] {
        *path = tmp.0.join("missing.exr");
    }
    assert!(session.set_scene_params(&missing).is_err());
    assert_eq!(session.scene(), &s);
    assert_eq!(session.fingerprint(), pipeline);
    session.step(Duration::ZERO).unwrap();
    let checkpoint = tmp.0.join("checkpoint");
    session.save(&checkpoint).unwrap();
    let mut resumed = RenderSession::resume(r.clone(), &checkpoint).unwrap();
    while !resumed.is_done() {
        resumed.step(Duration::from_millis(10)).unwrap();
    }
    let fresh = r.render(&s, &cfg, &mut |_| {}).unwrap();
    assert_eq!(resumed.readback().unwrap().color.data, fresh.color.data);
    let original = std::fs::read(checkpoint.join("accum.bin")).unwrap();
    map(&tmp.0, "constant.exr", 16, 8, |_, _| [5.0; 3]);
    let error = RenderSession::resume(r.clone(), &checkpoint)
        .err()
        .expect("changed HDRI must be refused");
    assert!(error.to_string().contains("HDRI contents have changed"));
    assert_eq!(
        std::fs::read(checkpoint.join("accum.bin")).unwrap(),
        original
    );
}
