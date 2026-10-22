use glam::Vec3;
use std::sync::Arc;
use std::time::Duration;
use syfrack_core::{
    Atmosphere, Camera, CameraMode, Color, Coloring, Fractal, Graph, Light, Look, Material,
    NodeKind, Scene,
};
use syfrack_render::{CompileStatus, GpuContext, RenderConfig, RenderSession, Renderer};

/// A sphere of `radius` at the origin, optionally on a ground plane at
/// y = `ground` (the bundled sphere graph, resized; without its plane).
fn sphere(radius: f32, ground: Option<f32>) -> Fractal {
    let mut g = Graph::bundled("sphere").unwrap();
    for n in &mut g.nodes {
        match &mut n.kind {
            NodeKind::Sphere { radius: r } => *r = radius,
            NodeKind::Trap { scale, .. } => *scale = radius,
            NodeKind::Plane { offset, .. } => *offset = ground.unwrap_or(0.0),
            _ => {}
        }
    }
    if ground.is_none() {
        // Entrée -> Piège -> Sphère -> Sortie: drop the plane and the union.
        let sphere = g
            .nodes
            .iter()
            .find(|n| matches!(n.kind, NodeKind::Sphere { .. }))
            .unwrap()
            .id;
        let output = g
            .nodes
            .iter()
            .find(|n| matches!(n.kind, NodeKind::Output))
            .unwrap()
            .id;
        let keep: Vec<u32> = g
            .nodes
            .iter()
            .filter(|n| !matches!(n.kind, NodeKind::Plane { .. } | NodeKind::Union))
            .map(|n| n.id)
            .collect();
        g.nodes.retain(|n| keep.contains(&n.id));
        g.links
            .retain(|l| keep.contains(&l.from.0) && keep.contains(&l.to.0));
        g.links.push(syfrack_core::Link {
            from: (sphere, 0),
            to: (output, 0),
        });
        g.bounding_radius_hint = radius;
    }
    g.validate().unwrap();
    Fractal::Graph(g)
}

fn r_ctx_read(r: &Renderer, buf: &wgpu::Buffer, size: u64) -> Vec<u8> {
    r.context().read_buffer(buf, size).unwrap()
}

/// A grey sphere alone in a uniform white environment. Because the sphere is
/// convex, every bounce escapes: sphere pixels must equal the albedo (0.5)
/// and background pixels must equal the environment (1.0).
fn furnace_scene() -> Scene {
    Scene {
        creation: None,
        creation_draft: None,
        phenomena: None,
        dimension: syfrack_core::Dimension::ThreeD,
        canvas: syfrack_core::Canvas2D::default(),
        seed: 3,
        fractal: sphere(1.0, None),
        camera: Camera {
            mode: CameraMode::Orbit {
                point: Vec3::ZERO,
                distance: 3.0,
                azimuth: 0.0,
                elevation: 0.0,
                roll: 0.0,
            },
            focal_mm: 50.0,
            f_number: 0.0,
            focus_distance: 3.0,
        },
        lights: vec![Light::Environment {
            zenith: Color::Linear([1.0, 1.0, 1.0]),
            horizon: Color::Linear([1.0, 1.0, 1.0]),
            ground: Color::Linear([1.0, 1.0, 1.0]),
            intensity: 1.0,
            rotation: 0.0,
        }],
        material: Material {
            albedo: [0.5, 0.5, 0.5],
            roughness: 0.5,
            metallic: 0.0,
            specular: 0.0,
            coloring: Coloring::None,
            texture: None,
            surface: None,
            finishes: vec![],
            optics: None,
        },
        atmosphere: Atmosphere::none(),
        look: Look::neutral(),
    }
}

fn renderer() -> Renderer {
    let ctx = Arc::new(GpuContext::new().expect("gpu"));
    Renderer::new(ctx).expect("renderer")
}

#[test]
#[ignore = "needs GPU"]
fn pixel_tolerance_reduces_analytic_sphere_hit_error() {
    let r = renderer();
    let mut scene = furnace_scene();
    scene.camera.mode = CameraMode::Free {
        position: Vec3::new(0.0, 0.0, 3.0),
        target: Vec3::ZERO,
        roll: 0.0,
    };
    let mut cfg = RenderConfig::new(64, 64, 1);
    cfg.max_bounces = 0;
    cfg.max_steps = 512;
    let mean_error = |out: &syfrack_render::RenderOutput| {
        let mut error = 0.0;
        for y in 20..44 {
            for x in 20..44 {
                let i = y * 64 + x;
                let t = out.depth[i];
                let nz = out.normals[i][2];
                assert!(t > 0.0 && nz > 0.0);
                // p = radius * n, |p - camera| = t: recover the hit radius
                // independently of the ray jitter, then compare to the unit sphere.
                let radius = 3.0 * nz - (t * t - 9.0 * (1.0 - nz * nz)).max(0.0).sqrt();
                error += (radius - 1.0).abs();
            }
        }
        error / (24.0 * 24.0)
    };
    let coarse = mean_error(&r.render(&scene, &cfg, &mut |_| {}).unwrap());
    cfg.pixel_tolerance = 0.25;
    let fine = mean_error(&r.render(&scene, &cfg, &mut |_| {}).unwrap());
    eprintln!("sphere hit radius error: standard={coarse:e}, fine={fine:e}");
    assert!(coarse > 1e-5 && fine < coarse * 0.6, "{coarse} -> {fine}");
}

#[test]
#[ignore = "needs GPU"]
fn fractal_texture_changes_color_roughness_and_relief_without_changing_geometry() {
    use syfrack_core::FractalTexture;
    let r = renderer();
    let mut cfg = RenderConfig::new(48, 48, 16);
    cfg.max_bounces = 1;
    let mut scene = Scene::example();
    scene.material.metallic = 0.5;
    let base = r.render(&scene, &cfg, &mut |_| {}).unwrap();
    let difference = |out: &syfrack_render::RenderOutput| {
        base.color
            .data
            .iter()
            .zip(&out.color.data)
            .map(|(a, b)| (a[0] - b[0]).abs() + (a[1] - b[1]).abs() + (a[2] - b[2]).abs())
            .sum::<f32>()
            / base.color.data.len() as f32
    };
    scene.material.texture = Some(FractalTexture {
        strength: 0.0,
        roughness: 0.0,
        bump: 0.0,
        ..Default::default()
    });
    let neutral = r.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert!(
        difference(&neutral) < 1e-6,
        "zero strengths preserve the rendering"
    );
    for effect in ["strength", "roughness", "bump"] {
        scene
            .set_param(&format!("material.texture.{effect}"), 0.8)
            .unwrap();
        let out = r.render(&scene, &cfg, &mut |_| {}).unwrap();
        assert_eq!(
            out.depth, base.depth,
            "{effect}: silhouette and depth are unchanged"
        );
        assert_eq!(out.normals, base.normals, "aux stores the geometric normal");
        assert!(out
            .color
            .data
            .iter()
            .flatten()
            .all(|v| v.is_finite() && *v >= 0.0));
        assert!(
            difference(&out) > 0.002,
            "{effect} must affect the rendered material"
        );
        scene
            .set_param(&format!("material.texture.{effect}"), 0.0)
            .unwrap();
    }
}

#[test]
#[ignore = "needs GPU"]
fn texture_parameters_reset_without_recompiling_and_checkpoint_resumes_exactly() {
    use syfrack_core::FractalTexture;
    let r = renderer();
    let mut scene = Scene::example();
    scene.material.texture = Some(FractalTexture::default());
    let cfg = RenderConfig::new(32, 24, 4);
    let mut session = RenderSession::new(r.clone(), &scene, &cfg).unwrap();
    let fingerprint = session.fingerprint();
    session.step(Duration::ZERO).unwrap();
    scene
        .set_param("material.texture.graph.nodes[5].factor", 2.5)
        .unwrap();
    assert!(session.set_scene_params(&scene).unwrap());
    assert_eq!(session.fingerprint(), fingerprint);
    assert_eq!(r.cached_pipelines(), 1);
    let fresh = r.render(&scene, &cfg, &mut |_| {}).unwrap();
    while !session.is_done() {
        session.step(Duration::from_millis(20)).unwrap();
    }
    assert_eq!(
        session.readback().unwrap().color.data,
        fresh.color.data,
        "parameter buffer was refreshed"
    );
    session.reset(&scene).unwrap();
    session.step(Duration::ZERO).unwrap();
    let dir = std::env::temp_dir().join(format!("syfrack-texture-resume-{}", std::process::id()));
    session.save(&dir).unwrap();
    let mut resumed = RenderSession::resume(r.clone(), &dir).unwrap();
    assert_eq!(resumed.scene(), &scene);
    while !resumed.is_done() {
        resumed.step(Duration::from_millis(20)).unwrap();
    }
    assert_eq!(resumed.readback().unwrap().color.data, fresh.color.data);
    std::fs::remove_dir_all(&dir).unwrap();
    scene.material.texture.as_mut().unwrap().graph = Graph::bundled("menger").unwrap();
    assert!(!session.set_scene_params(&scene).unwrap());
    assert_ne!(session.fingerprint(), fingerprint);
    scene.material.texture = None;
    assert!(!session.set_scene_params(&scene).unwrap());
    assert_eq!(
        session.fingerprint(),
        syfrack_render::graph_wgsl::scene_fingerprint(&scene)
    );
}

#[test]
#[ignore = "needs GPU"]
fn parallax_offsets_material_sampling_and_keeps_geometric_buffers() {
    let r = renderer();
    let mut scene = Scene::example();
    scene.material.texture = Some(syfrack_core::FractalTexture {
        scale: 2.0,
        roughness: 0.0,
        bump: 0.15,
        ..Default::default()
    });
    let cfg = RenderConfig::new(48, 32, 16);
    let flat = r.render(&scene, &cfg, &mut |_| {}).unwrap();
    scene.set_param("material.texture.parallax", 0.5).unwrap();
    scene
        .set_param("material.texture.parallax_steps", 16.0)
        .unwrap();
    let deep = r.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert_eq!(flat.depth, deep.depth);
    assert_eq!(flat.normals, deep.normals);
    assert_ne!(flat.color.data, deep.color.data);
    assert!(deep
        .color
        .data
        .iter()
        .flatten()
        .all(|v| v.is_finite() && *v >= 0.0));
}

#[test]
#[ignore = "needs GPU"]
fn surface_variation_endpoints_match_uniform_materials() {
    let r = renderer();
    let mut scene = furnace_scene();
    scene.material.specular = 0.04;
    let cfg = RenderConfig::new(32, 32, 32);
    for (slot, roughness, metallic) in [(0, 0.25, 0.0), (5, 0.35, 1.0)] {
        scene.material.roughness = roughness;
        scene.material.metallic = metallic;
        scene.material.surface = None;
        let uniform = r.render(&scene, &cfg, &mut |_| {}).unwrap();
        scene.material.surface = Some(syfrack_core::SurfaceVariation {
            slot,
            roughness: [roughness; 2],
            metallic: [metallic; 2],
        });
        let mapped = r.render(&scene, &cfg, &mut |_| {}).unwrap();
        assert_eq!(uniform.color.data, mapped.color.data);
        scene.material.surface.as_mut().unwrap().metallic = [0.1, 0.9];
        let varied = r.render(&scene, &cfg, &mut |_| {}).unwrap();
        assert_ne!(mapped.color.data, varied.color.data);
        assert!(varied.color.data.iter().flatten().all(|v| v.is_finite()));
    }
}

#[test]
#[ignore = "needs GPU"]
fn fan_texture_mask_selects_the_geometry_trap_without_changing_the_form() {
    use syfrack_core::{FanMapping, FractalTexture, TextureMask};
    let r = renderer();
    let mut scene = furnace_scene();
    let cfg = RenderConfig::new(40, 32, 8);
    let base = r.render(&scene, &cfg, &mut |_| {}).unwrap();
    scene.material.texture = Some(FractalTexture {
        fan: Some(FanMapping::default()),
        mask: Some(TextureMask::default()), // Trap 1 is unwritten on this sphere.
        ..Default::default()
    });
    let masked = r.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert_eq!(base.color.data, masked.color.data);
    scene
        .material
        .texture
        .as_mut()
        .unwrap()
        .mask
        .as_mut()
        .unwrap()
        .value = 1.0;
    let textured = r.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert_ne!(base.color.data, textured.color.data);
    assert_eq!(base.depth, textured.depth);
    assert_eq!(base.normals, textured.normals);
    assert!(textured
        .color
        .data
        .iter()
        .flatten()
        .all(|v| v.is_finite() && *v >= 0.0));
}

#[test]
#[ignore = "needs GPU"]
fn procedural_sea_has_a_finite_horizon_and_responds_to_sun_and_waves() {
    let r = renderer();
    let mut scene = Scene::example();
    scene.fractal = sphere(0.1, None);
    scene.camera.mode = CameraMode::Free {
        position: Vec3::new(0.0, 3.0, 6.0),
        target: Vec3::new(0.0, 3.0, -10.0),
        roll: 0.0,
    };
    scene.camera.focal_mm = 24.0;
    scene.lights[0] = Light::Sun {
        direction: Vec3::new(0.08, 0.045, -1.0),
        color: Color::Linear([1.0, 0.32, 0.085]),
        intensity: 1.4,
        angular_size: 2.0,
    };
    let mut cfg = RenderConfig::new(64, 36, 4);
    cfg.max_bounces = 1;
    let plain = r.render(&scene, &cfg, &mut |_| {}).unwrap();
    scene.atmosphere.seascape = Some(Default::default());
    let coast = r.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert_ne!(plain.color.data, coast.color.data);
    assert!(coast
        .color
        .data
        .iter()
        .flatten()
        .all(|v| v.is_finite() && *v >= 0.0));
    let top = coast.color.data[64 * 8 + 32];
    let bottom = coast.color.data[64 * 28 + 32];
    assert!(
        top[..3]
            .iter()
            .zip(&bottom[..3])
            .map(|(a, b)| (a - b).abs())
            .sum::<f32>()
            > 0.1
    );
    scene
        .set_param("atmosphere.seascape.wave_strength", 0.0)
        .unwrap();
    let calm = r.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert_ne!(calm.color.data, coast.color.data);
    if let Light::Sun { direction, .. } = &mut scene.lights[0] {
        *direction = Vec3::new(-0.3, 0.1, -1.0);
    }
    let moved_sun = r.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert_ne!(calm.color.data, moved_sun.color.data);
    assert_eq!(coast.depth, moved_sun.depth);
}

#[test]
#[ignore = "needs GPU"]
fn weighted_area_selection_preserves_the_sum_of_identical_lights() {
    let r = renderer();
    let mut scene = furnace_scene();
    let lamp = Light::Area {
        position: Vec3::new(0.0, 3.0, 3.0),
        normal: Vec3::new(0.0, -1.0, -1.0),
        size: [3.0, 3.0],
        color: Color::Linear([1.0; 3]),
        intensity: 4.0,
    };
    scene.lights = vec![lamp.clone(), lamp];
    let mut cfg = RenderConfig::new(32, 32, 32);
    cfg.max_bounces = 0;
    let pair = r.render(&scene, &cfg, &mut |_| {}).unwrap();
    scene.lights.truncate(1);
    if let Light::Area { intensity, .. } = &mut scene.lights[0] {
        *intensity = 8.0;
    }
    let one = r.render(&scene, &cfg, &mut |_| {}).unwrap();
    for (a, b) in pair.color.data.iter().zip(&one.color.data) {
        for c in 0..3 {
            assert!((a[c] - b[c]).abs() < 1e-5);
        }
    }
}

fn mean_rgb(out: &syfrack_render::RenderOutput, x0: u32, y0: u32, n: u32) -> f32 {
    let mut s = 0.0;
    for y in y0..y0 + n {
        for x in x0..x0 + n {
            let p = out.color.get(x, y);
            s += (p[0] + p[1] + p[2]) / 3.0;
        }
    }
    s / (n * n) as f32
}

/// The pixel with the largest hit depth whose 3x3 neighbourhood is entirely
/// sphere (no background mixed in): the most grazing fully covered pixel.
fn rim_pixel(out: &syfrack_render::RenderOutput, w: u32) -> (u32, u32) {
    let h = out.depth.len() as u32 / w;
    let mut best = (w / 2, h / 2);
    let mut best_d = -1.0f32;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let all_hit = (y - 1..=y + 1)
                .all(|yy| (x - 1..=x + 1).all(|xx| out.depth[(yy * w + xx) as usize] >= 0.0));
            let d = out.depth[(y * w + x) as usize];
            if all_hit && d > best_d {
                best_d = d;
                best = (x, y);
            }
        }
    }
    best
}

#[test]
#[ignore = "needs GPU"]
fn furnace_sphere_matches_albedo_and_background_matches_environment() {
    let r = renderer();
    let mut cfg = RenderConfig::new(48, 48, 64);
    cfg.tile_size = 32;
    let out = r
        .render(&furnace_scene(), &cfg, &mut |_| {})
        .expect("render");
    let centre = mean_rgb(&out, 22, 22, 4);
    let corner = mean_rgb(&out, 0, 0, 3);
    assert!(
        (centre - 0.5).abs() < 0.03,
        "sphere centre {centre}, expected 0.5"
    );
    assert!(
        (corner - 1.0).abs() < 1e-3,
        "background {corner}, expected 1.0"
    );
    assert_eq!(out.samples_min, 64);
    assert_eq!(out.samples_max, 64);
}

#[test]
#[ignore = "needs GPU"]
fn depth_of_centre_pixel_is_distance_minus_radius() {
    let r = renderer();
    let out = r
        .render(&furnace_scene(), &RenderConfig::new(33, 33, 1), &mut |_| {})
        .expect("render");
    let d = out.depth[(16 * 33 + 16) as usize];
    assert!((d - 2.0).abs() < 0.05, "depth {d}");
    assert!(
        out.depth[0] < 0.0,
        "background depth must be negative (miss)"
    );
}

#[test]
#[ignore = "needs GPU"]
fn rendering_twice_is_bit_identical() {
    let r = renderer();
    let mut cfg = RenderConfig::new(64, 40, 4);
    cfg.tile_size = 32;
    let a = r
        .render(&Scene::example(), &cfg, &mut |_| {})
        .expect("render a");
    let b = r
        .render(&Scene::example(), &cfg, &mut |_| {})
        .expect("render b");
    assert_eq!(a.color.data, b.color.data);
    assert_eq!(a.depth, b.depth);
}

#[test]
#[ignore = "needs GPU"]
fn sun_lit_example_has_a_shadow_side() {
    let r = renderer();
    let out = r
        .render(
            &Scene::example(),
            &RenderConfig::new(64, 64, 16),
            &mut |_| {},
        )
        .expect("render");
    let bright = out
        .color
        .data
        .iter()
        .map(|p| p[0] + p[1] + p[2])
        .fold(0.0f32, f32::max);
    let dark = out
        .color
        .data
        .iter()
        .map(|p| p[0] + p[1] + p[2])
        .fold(f32::MAX, f32::min);
    assert!(
        bright > 0.5 && dark >= 0.0 && bright > 2.0 * dark,
        "bright {bright} dark {dark}"
    );
    assert!(
        out.color
            .data
            .iter()
            .all(|p| p.iter().all(|c| c.is_finite())),
        "no NaN/inf"
    );
}

#[test]
#[ignore = "needs GPU"]
fn time_budget_stops_early_and_reports_partial_samples() {
    let r = renderer();
    let mut cfg = RenderConfig::new(256, 256, 100_000);
    cfg.time_budget = Some(std::time::Duration::from_millis(300));
    let mut calls = 0;
    let out = r
        .render(&Scene::example(), &cfg, &mut |_| calls += 1)
        .expect("render");
    assert!(calls > 0);
    assert!(out.samples_min >= 1 && out.samples_max < 100_000);

    // Nothing rendered: every pixel must still report the miss depth.
    let mut none = RenderConfig::new(64, 64, 8);
    none.time_budget = Some(std::time::Duration::ZERO);
    let out = r
        .render(&Scene::example(), &none, &mut |_| {})
        .expect("render");
    assert_eq!(out.samples_max, 0);
    assert!(
        out.depth.iter().all(|&d| d == -1.0),
        "unrendered pixels must have depth -1"
    );
}

#[test]
#[ignore = "needs GPU"]
fn furnace_rim_pixel_does_not_gain_energy() {
    let r = renderer();
    let cfg = RenderConfig::new(48, 48, 256);
    let out = r
        .render(&furnace_scene(), &cfg, &mut |_| {})
        .expect("render");
    let (rx, ry) = rim_pixel(&out, 48);
    let m = mean_rgb(&out, rx - 1, ry - 1, 3);
    assert!((0.3..=1.0).contains(&m), "rim mean {m} (at {rx},{ry})");
}

#[test]
#[ignore = "needs GPU"]
fn metallic_furnace_does_not_exceed_unit_energy() {
    let r = renderer();
    let mut scene = furnace_scene();
    scene.material = Material {
        albedo: [1.0, 1.0, 1.0],
        roughness: 0.5,
        metallic: 1.0,
        specular: 0.04,
        coloring: Coloring::None,
        texture: None,
        surface: None,
        finishes: vec![],
        optics: None,
    };
    let cfg = RenderConfig::new(48, 48, 256);
    let out = r.render(&scene, &cfg, &mut |_| {}).expect("render");
    let (rx, ry) = rim_pixel(&out, 48);
    let centre = mean_rgb(&out, 22, 22, 4);
    let rim = mean_rgb(&out, rx - 1, ry - 1, 3);
    assert!(centre <= 1.02, "centre {centre}");
    assert!(rim <= 1.02, "rim {rim}");
}

#[test]
#[ignore = "needs GPU"]
fn output_is_identical_across_tile_size_and_samples_per_dispatch() {
    let r = renderer();
    let run = |tile: u32, spd: u32, dispatch_ms: u32| {
        // One-pixel edge tiles exercise the ramp's fixed dispatch overhead
        // and regrouping at pass boundaries without losing edge samples.
        let mut cfg = RenderConfig::new(65, 41, 16);
        cfg.tile_size = tile;
        cfg.samples_per_dispatch = spd;
        cfg.dispatch_budget_ms = dispatch_ms;
        r.render(&Scene::example(), &cfg, &mut |_| {})
            .expect("render")
    };
    let a = run(32, 8, 400);
    let b = run(64, 8, 800);
    let c = run(32, 1, 50);
    let d = run(128, 8, 800);
    assert_eq!(a.color.data, b.color.data);
    assert_eq!(a.color.data, c.color.data);
    assert_eq!(a.color.data, d.color.data);
    assert_eq!(a.depth, b.depth);
    assert_eq!(a.depth, c.depth);
    assert_eq!(a.depth, d.depth);
    assert_eq!(a.normals, b.normals);
    assert_eq!(a.normals, c.normals);
    assert_eq!(a.normals, d.normals);
}

/// A sun of irradiance E on a Lambertian plane of albedo rho reflects
/// radiance rho * E / pi toward any viewer.
#[test]
#[ignore = "needs GPU"]
fn sun_irradiance_on_lambertian_plane_matches_albedo_e_over_pi() {
    let scene = Scene {
        creation: None,
        creation_draft: None,
        phenomena: None,
        dimension: syfrack_core::Dimension::ThreeD,
        canvas: syfrack_core::Canvas2D::default(),
        seed: 5,
        fractal: sphere(0.01, Some(0.0)),
        camera: Camera {
            mode: CameraMode::Free {
                position: Vec3::new(0.0, 3.0, 0.5),
                target: Vec3::new(0.0, 0.0, 0.5),
                roll: 0.0,
            },
            focal_mm: 50.0,
            f_number: 0.0,
            focus_distance: 3.0,
        },
        lights: vec![Light::Sun {
            direction: Vec3::Y,
            color: Color::Linear([1.0, 1.0, 1.0]),
            intensity: 2.0,
            angular_size: 0.5,
        }],
        material: Material {
            albedo: [0.5, 0.5, 0.5],
            roughness: 0.5,
            metallic: 0.0,
            specular: 0.0,
            coloring: Coloring::None,
            texture: None,
            surface: None,
            finishes: vec![],
            optics: None,
        },
        atmosphere: Atmosphere::none(),
        look: Look::neutral(),
    };
    let mut cfg = RenderConfig::new(16, 16, 256);
    cfg.max_bounces = 1;
    let out = renderer()
        .render(&scene, &cfg, &mut |_| {})
        .expect("render");
    let centre = mean_rgb(&out, 6, 6, 4);
    let expected = 0.5 * 2.0 / std::f32::consts::PI;
    println!("NEE centre mean {centre}, expected {expected}");
    assert!(
        (centre - expected).abs() < 0.03 * expected,
        "centre {centre}, expected {expected}"
    );
}

#[test]
#[ignore = "needs GPU"]
fn each_fractal_renders_hits_with_unit_normals() {
    let r = renderer();
    for kind in ["mandelbulb", "mandelbox", "menger"] {
        let scene = Scene::example_for(kind).unwrap();
        let mut cfg = RenderConfig::new(64, 64, 4);
        cfg.tile_size = 32;
        let out = r.render(&scene, &cfg, &mut |_| {}).unwrap();
        let hits = out.depth.iter().filter(|d| **d > 0.0).count();
        assert!(
            hits * 10 >= out.depth.len() * 3,
            "{kind:?}: only {hits} hits of {}",
            out.depth.len()
        );
        for (i, n) in out.normals.iter().enumerate() {
            if out.depth[i] > 0.0 {
                let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                assert!((len - 1.0).abs() < 1e-3, "{kind:?} pixel {i} normal {n:?}");
                assert!(n[3] >= 1.0, "{kind:?} pixel {i} steps {}", n[3]);
            } else {
                assert_eq!(n[3], -1.0, "{kind:?} miss must carry steps -1");
            }
        }
        assert!(
            out.color
                .data
                .iter()
                .all(|p| p.iter().all(|c| c.is_finite())),
            "{kind:?} NaN"
        );
    }
}

#[test]
#[ignore = "needs GPU"]
fn coloring_changes_the_image_and_none_leaves_it_alone() {
    use syfrack_core::Ramp;
    let r = renderer();
    let base = Scene::example_for("mandelbox").unwrap();
    let mut plain = base.clone();
    plain.material.coloring = Coloring::None;
    let mut trap = base.clone();
    trap.material.coloring = Coloring::Trap {
        slot: 0,
        ramp: Ramp {
            stops: vec![[1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            ..Ramp::default()
        },
    };
    let mut ones = base.clone();
    ones.material.coloring = Coloring::Trap {
        slot: 4,
        ramp: Ramp {
            stops: vec![[1.0; 3], [1.0; 3]],
            ..Ramp::default()
        },
    };
    let cfg = RenderConfig::new(48, 48, 8);
    let a = r.render(&plain, &cfg, &mut |_| {}).unwrap();
    let b = r.render(&trap, &cfg, &mut |_| {}).unwrap();
    let c = r.render(&ones, &cfg, &mut |_| {}).unwrap();
    assert_ne!(
        a.color.data, b.color.data,
        "an orbit-trap ramp must change the colours"
    );
    assert_eq!(
        a.color.data, c.color.data,
        "an all-white ramp must equal no coloring, bit for bit"
    );
}

/// A smooth metal sphere under a small sun with no sky: the sun's mirror
/// image must appear as a bright highlight, which only BSDF sampling can find.
#[test]
#[ignore = "needs GPU"]
fn sun_is_visible_in_a_mirror_reflection() {
    let r = renderer();
    let mut scene = furnace_scene();
    scene.lights = vec![Light::Sun {
        direction: Vec3::new(0.3, 0.9, 0.3),
        color: Color::Linear([1.0, 1.0, 1.0]),
        intensity: 2.0,
        angular_size: 2.0,
    }];
    scene.material = Material {
        albedo: [0.9; 3],
        roughness: 0.05,
        metallic: 1.0,
        specular: 0.04,
        coloring: Coloring::None,
        texture: None,
        surface: None,
        finishes: vec![],
        optics: None,
    };
    let mut cfg = RenderConfig::new(64, 64, 64);
    cfg.max_bounces = 2;
    let out = r.render(&scene, &cfg, &mut |_| {}).unwrap();
    let mut lum: Vec<f32> = out.color.data.iter().map(|p| p[1]).collect();
    lum.sort_by(f32::total_cmp);
    let median = lum[lum.len() / 2];
    let max = *lum.last().unwrap();
    eprintln!("mirror sun: max {max} median {median}");
    assert!(max > 5.0, "no highlight: max {max}");
    assert!(
        max > 20.0 * median.max(1e-3),
        "highlight {max} vs median {median}"
    );
}

/// The sun disk seen directly by the camera has radiance E / solid angle.
#[test]
#[ignore = "needs GPU"]
fn camera_ray_into_the_sun_sees_its_radiance() {
    let r = renderer();
    let mut scene = furnace_scene();
    scene.fractal = sphere(0.01, None);
    scene.camera.mode = CameraMode::Free {
        position: Vec3::new(0.0, 0.0, 5.0),
        target: Vec3::new(0.0, 0.0, 6.0),
        roll: 0.0,
    };
    scene.camera.focal_mm = 200.0;
    let angular = 4.0f32;
    scene.lights = vec![Light::Sun {
        direction: Vec3::Z,
        color: Color::Linear([1.0; 3]),
        intensity: 1.0,
        angular_size: angular,
    }];
    let out = r
        .render(&scene, &RenderConfig::new(33, 33, 16), &mut |_| {})
        .unwrap();
    let centre = out.color.get(16, 16)[0];
    let solid_angle = 2.0 * std::f32::consts::PI * (1.0 - (angular * 0.5).to_radians().cos());
    let expected = 1.0 / solid_angle;
    eprintln!("sun centre {centre} expected {expected}");
    assert!(
        (centre - expected).abs() < 0.05 * expected,
        "centre {centre} expected {expected}"
    );
}

#[test]
#[ignore = "needs GPU"]
fn cavity_darkness_only_darkens_and_is_monotonic() {
    let r = renderer();
    let base = Scene::example_for("mandelbox").unwrap();
    let with = |cavity: f32| {
        let mut s = base.clone();
        s.atmosphere.cavity_darkness = cavity;
        s
    };
    let cfg = RenderConfig::new(48, 48, 8);
    let a = r.render(&with(0.0), &cfg, &mut |_| {}).unwrap();
    let mid = r.render(&with(0.4), &cfg, &mut |_| {}).unwrap();
    let b = r.render(&with(0.8), &cfg, &mut |_| {}).unwrap();
    let sum = |o: &syfrack_render::RenderOutput| o.color.data.iter().map(|p| p[1]).sum::<f32>();
    eprintln!("cavity sums {} {} {}", sum(&a), sum(&mid), sum(&b));
    assert!(
        sum(&b) < sum(&mid) && sum(&mid) < sum(&a),
        "cavity darkness must remove light monotonically"
    );
    assert!(b
        .color
        .data
        .iter()
        .zip(&a.color.data)
        .all(|(d, l)| d[1] <= l[1] + 1e-4));
}

/// White furnace with scattering fog: a non-absorbing medium inside a uniform
/// white environment leaves radiance at 1 everywhere (up to path truncation).
#[test]
#[ignore = "needs GPU"]
fn scattering_fog_in_a_white_furnace_conserves_energy() {
    let r = renderer();
    let mut scene = furnace_scene();
    scene.material.albedo = [1.0; 3];
    // Optical depth ~2 to the far cap. Fog now also participates after a
    // surface bounce, so 12 events truncate too much of this conservative
    // random walk. 32 events resolve the tail without loosening the tolerance.
    scene.atmosphere.fog_density = 0.02;
    scene.atmosphere.fog_color = [1.0; 3];
    scene.atmosphere.fog_height = 50.0;
    scene.atmosphere.fog_falloff = 0.1;
    scene.atmosphere.anisotropy = 0.3;
    let mut cfg = RenderConfig::new(32, 32, 256);
    cfg.max_bounces = 32;
    let out = r.render(&scene, &cfg, &mut |_| {}).unwrap();
    let mean = out.color.data.iter().map(|p| p[1]).sum::<f32>() / out.color.data.len() as f32;
    eprintln!("fog furnace mean {mean}");
    assert!((mean - 1.0).abs() < 0.06, "furnace mean {mean}");
    assert!(
        out.color.data.iter().all(|p| p[1] < 1.3),
        "no fireflies above 1.3"
    );
}

/// Absorbing fog (black albedo) in a thin layer above the ground: the camera
/// looks straight down, the sun shines straight down, both legs cross the
/// layer once, so radiance = rho*E/pi * exp(-2 * density / falloff).
#[test]
#[ignore = "needs GPU"]
fn absorbing_fog_attenuates_by_the_analytic_optical_depth() {
    let r = renderer();
    let mut scene = furnace_scene();
    scene.fractal = sphere(0.01, Some(0.0));
    scene.camera.mode = CameraMode::Free {
        position: Vec3::new(0.0, 3.0, 0.5),
        target: Vec3::new(0.0, 0.0, 0.5),
        roll: 0.0,
    };
    // Narrow field and fine pixels: the surface hit threshold and the shadow
    // origin offset both scale with the pixel cone, and the layer is only ~0.2
    // thick, so a coarse cone would skip part of it.
    scene.camera.focal_mm = 1000.0;
    scene.lights = vec![Light::Sun {
        direction: Vec3::Y,
        color: Color::Linear([1.0; 3]),
        intensity: 2.0,
        angular_size: 0.5,
    }];
    scene.material = Material {
        albedo: [0.5; 3],
        roughness: 0.5,
        metallic: 0.0,
        specular: 0.0,
        coloring: Coloring::None,
        texture: None,
        surface: None,
        finishes: vec![],
        optics: None,
    };
    scene.atmosphere.fog_density = 2.0;
    scene.atmosphere.fog_color = [0.0; 3];
    scene.atmosphere.fog_height = 0.0;
    scene.atmosphere.fog_falloff = 10.0;
    let mut cfg = RenderConfig::new(32, 32, 512);
    cfg.max_bounces = 1;
    let out = r.render(&scene, &cfg, &mut |_| {}).unwrap();
    let centre = mean_rgb(&out, 14, 14, 4);
    let expected = 0.5 * 2.0 / std::f32::consts::PI * (-2.0 * 2.0 / 10.0f32).exp();
    eprintln!("absorbing fog centre {centre} expected {expected}");
    assert!(
        (centre - expected).abs() < 0.05 * expected,
        "centre {centre} expected {expected}"
    );
}

#[test]
#[ignore = "needs GPU"]
fn zero_fog_density_is_bit_identical_to_no_atmosphere() {
    let r = renderer();
    let a = Scene::example();
    let mut b = Scene::example();
    b.atmosphere.fog_density = 0.0;
    b.atmosphere.fog_color = [0.3, 0.2, 0.1];
    b.atmosphere.anisotropy = 0.5;
    let cfg = RenderConfig::new(48, 32, 4);
    let x = r.render(&a, &cfg, &mut |_| {}).unwrap();
    let y = r.render(&b, &cfg, &mut |_| {}).unwrap();
    assert_eq!(x.color.data, y.color.data);
}

/// With no bounce left to continue, NEE must carry the full sun weight.
#[test]
#[ignore = "needs GPU"]
fn glossy_sun_highlight_survives_at_the_last_bounce() {
    let r = renderer();
    let mut scene = furnace_scene();
    scene.lights = vec![Light::Sun {
        direction: Vec3::new(0.3, 0.9, 0.3),
        color: Color::Linear([1.0, 1.0, 1.0]),
        intensity: 2.0,
        angular_size: 2.0,
    }];
    scene.material = Material {
        albedo: [0.9; 3],
        roughness: 0.05,
        metallic: 1.0,
        specular: 0.04,
        coloring: Coloring::None,
        texture: None,
        surface: None,
        finishes: vec![],
        optics: None,
    };
    let mut cfg = RenderConfig::new(64, 64, 64);
    cfg.max_bounces = 0;
    let out = r.render(&scene, &cfg, &mut |_| {}).unwrap();
    let max = out.color.data.iter().map(|p| p[1]).fold(0.0f32, f32::max);
    eprintln!("last bounce mirror sun: max {max}");
    assert!(max > 300.0, "highlight lost at the last bounce: max {max}");
}

/// Near-vertical rays at tiny pixel cones must not tunnel through the plane.
#[test]
#[ignore = "needs GPU"]
fn nee_plane_is_exact_at_small_pixel_cones() {
    let scene = Scene {
        creation: None,
        creation_draft: None,
        phenomena: None,
        dimension: syfrack_core::Dimension::ThreeD,
        canvas: syfrack_core::Canvas2D::default(),
        seed: 5,
        fractal: sphere(0.01, Some(0.0)),
        camera: Camera {
            mode: CameraMode::Free {
                position: Vec3::new(0.0, 3.0, 0.5),
                target: Vec3::new(0.0, 0.0, 0.5),
                roll: 0.0,
            },
            focal_mm: 1000.0,
            f_number: 0.0,
            focus_distance: 3.0,
        },
        lights: vec![Light::Sun {
            direction: Vec3::Y,
            color: Color::Linear([1.0, 1.0, 1.0]),
            intensity: 2.0,
            angular_size: 0.5,
        }],
        material: Material {
            albedo: [0.5, 0.5, 0.5],
            roughness: 0.5,
            metallic: 0.0,
            specular: 0.0,
            coloring: Coloring::None,
            texture: None,
            surface: None,
            finishes: vec![],
            optics: None,
        },
        atmosphere: Atmosphere::none(),
        look: Look::neutral(),
    };
    let mut cfg = RenderConfig::new(64, 64, 512);
    cfg.max_bounces = 1;
    let out = renderer()
        .render(&scene, &cfg, &mut |_| {})
        .expect("render");
    let centre = mean_rgb(&out, 30, 30, 4);
    let expected = 0.5 * 2.0 / std::f32::consts::PI;
    eprintln!("small cone centre {centre} expected {expected}");
    assert!(
        (centre - expected).abs() < 0.02 * expected,
        "centre {centre}, expected {expected}"
    );
}

/// Single scattering in thin uniform fog, no surfaces: a ray crosses 100 units of
/// fog (sigma 0.001) and the sun is at 90 degrees to the view, also seen through
/// fog. L ~ E/(4 pi) (1 - e^-tau_ray) e^-tau_sun (+ ~10% multiple scattering).
/// The sun is wide (60 degrees) so phase-sampled directions hit it often: the
/// escape must be MIS-weighted against the NEE, not added on top of it (a
/// double count roughly doubles the value).
#[test]
#[ignore = "needs GPU"]
fn thin_scattering_fog_single_scatter_matches_analytic() {
    let r = renderer();
    let mut scene = furnace_scene();
    scene.fractal = sphere(0.01, None);
    scene.camera.mode = CameraMode::Free {
        position: Vec3::new(0.0, 0.0, 5.0),
        target: Vec3::new(0.0, 0.0, 6.0),
        roll: 0.0,
    };
    scene.camera.focal_mm = 1000.0;
    scene.lights = vec![Light::Sun {
        direction: Vec3::Y,
        color: Color::Linear([1.0; 3]),
        intensity: 2.0,
        angular_size: 60.0,
    }];
    scene.atmosphere.fog_density = 0.001;
    scene.atmosphere.fog_color = [1.0; 3];
    scene.atmosphere.fog_height = 50.0;
    scene.atmosphere.fog_falloff = 0.1;
    scene.atmosphere.anisotropy = 0.0;
    let mut cfg = RenderConfig::new(16, 16, 4096);
    cfg.max_bounces = 2;
    let out = r.render(&scene, &cfg, &mut |_| {}).unwrap();
    let centre = mean_rgb(&out, 6, 6, 4);
    // Ray leg: sigma * 100 = 0.1. Sun leg from y = 0: 50 units of full density
    // plus the exponential tail above (density / falloff).
    let tau_ray = 0.1f32;
    let tau_sun = 0.001f32 * 50.0 + 0.001 / 0.1;
    let expected = 2.0 / (4.0 * std::f32::consts::PI) * (1.0 - (-tau_ray).exp()) * (-tau_sun).exp();
    eprintln!("thin fog single scatter centre {centre} expected {expected}");
    assert!(
        (centre - expected).abs() < 0.15 * expected,
        "centre {centre} expected {expected}"
    );
}

#[test]
#[ignore = "needs GPU"]
fn stepping_a_session_equals_one_shot_render_bit_for_bit() {
    let r = renderer();
    let scene = Scene::example();
    let mut cfg = RenderConfig::new(64, 40, 16);
    cfg.tile_size = 32;
    let one_shot = r.render(&scene, &cfg, &mut |_| {}).unwrap();
    let mut session = RenderSession::new(r.clone(), &scene, &cfg).unwrap();
    let mut steps = 0;
    while !session.step(std::time::Duration::from_millis(1)).unwrap() {
        steps += 1;
        assert!(steps < 10_000, "session never finishes");
    }
    assert!(
        steps >= 2,
        "a 1 ms budget must take several steps, took {steps}"
    );
    let stepped = session.readback().unwrap();
    assert_eq!(stepped.color.data, one_shot.color.data);
    assert_eq!(stepped.depth, one_shot.depth);
    assert_eq!(stepped.normals, one_shot.normals);
    assert!(session.is_done());
    assert_eq!(session.progress().samples_min, 16);
}

#[test]
#[ignore = "needs GPU"]
fn save_then_resume_equals_uninterrupted_render_bit_for_bit() {
    let r = renderer();
    let scene = Scene::example_for("mandelbulb").unwrap();
    let mut cfg = RenderConfig::new(48, 48, 12);
    cfg.pixel_tolerance = 0.25;
    cfg.max_steps = 768;
    cfg.tile_size = 32;
    cfg.samples_per_dispatch = 2;
    let reference = r.render(&scene, &cfg, &mut |_| {}).unwrap();

    let dir = std::env::temp_dir().join(format!("syfrack-session-{}", std::process::id()));
    let mut first = RenderSession::new(r.clone(), &scene, &cfg).unwrap();
    assert!(
        !first.step(std::time::Duration::from_millis(1)).unwrap(),
        "must not finish in one step"
    );
    let mid = first.progress();
    assert!(mid.samples_max > 0 && mid.samples_min < 12, "{mid:?}");
    first.save(&dir).unwrap();
    drop(first);

    let mut second = RenderSession::new(r.clone(), &scene, &cfg).unwrap(); // decoy: fresh session, not used
    let _ = second.step(std::time::Duration::from_millis(1));
    let mut resumed = RenderSession::resume(r.clone(), &dir).unwrap();
    assert_eq!(resumed.config().pixel_tolerance, 0.25);
    assert_eq!(resumed.progress().samples_max, mid.samples_max);
    assert_eq!(resumed.scene(), &scene);
    resumed.set_dispatch_budget_ms(800).unwrap();
    assert_eq!(resumed.progress().samples_max, mid.samples_max);
    resumed.save(&dir).unwrap();
    let mut resumed = RenderSession::resume(r.clone(), &dir).unwrap();
    assert_eq!(resumed.config().dispatch_budget_ms, 800);
    while !resumed.step(std::time::Duration::from_millis(5)).unwrap() {}
    let out = resumed.readback().unwrap();
    assert_eq!(out.color.data, reference.color.data);
    assert_eq!(out.depth, reference.depth);
    assert_eq!(out.normals, reference.normals);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
#[ignore = "needs GPU"]
fn reset_restarts_accumulation_with_a_new_scene() {
    let r = renderer();
    let cfg = RenderConfig::new(32, 32, 4);
    let a = Scene::example();
    let mut b = Scene::example();
    b.seed = 99;
    let fresh_b = r.render(&b, &cfg, &mut |_| {}).unwrap();
    let mut session = RenderSession::new(r.clone(), &a, &cfg).unwrap();
    while !session.step(std::time::Duration::from_millis(5)).unwrap() {}
    session.reset(&b).unwrap();
    assert!(!session.is_done());
    assert_eq!(session.progress().samples_max, 0);
    while !session.step(std::time::Duration::from_millis(5)).unwrap() {}
    assert_eq!(session.readback().unwrap().color.data, fresh_b.color.data);
}

#[test]
#[ignore = "needs GPU"]
fn resume_rejects_a_corrupt_state_dir() {
    let r = renderer();
    let dir = std::env::temp_dir().join(format!("syfrack-bad-state-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("state.ron"), "(not: valid)").unwrap();
    assert!(RenderSession::resume(r.clone(), &dir).is_err());

    // Syntactically valid state with a negative elapsed time and empty buffers.
    let cfg = RenderConfig::new(32, 32, 2);
    let session = RenderSession::new(r.clone(), &Scene::example(), &cfg).unwrap();
    session.save(&dir).unwrap();
    let text = std::fs::read_to_string(dir.join("state.ron")).unwrap();
    let state: syfrack_render::SessionState = ron::from_str(&text).unwrap();
    let ron_text = ron::ser::to_string(&state).unwrap();
    let patched = ron_text.replace(
        &format!("elapsed_secs:{:?}", state.elapsed_secs),
        "elapsed_secs:-1.0",
    );
    assert_ne!(patched, ron_text, "the elapsed_secs patch must apply");
    std::fs::write(dir.join("state.ron"), &patched).unwrap();
    // Intact buffers: the duration check must reject it.
    assert!(RenderSession::resume(r.clone(), &dir).is_err());
    for name in ["accum.bin", "depth.bin", "normals.bin"] {
        std::fs::write(dir.join(name), []).unwrap();
    }
    assert!(RenderSession::resume(r, &dir).is_err());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
#[ignore = "needs GPU"]
fn saving_twice_into_the_same_dir_resumes_from_the_latest_state() {
    let r = renderer();
    let scene = Scene::example_for("mandelbulb").unwrap();
    let mut cfg = RenderConfig::new(48, 48, 12);
    cfg.tile_size = 32;
    cfg.samples_per_dispatch = 2;
    let reference = r.render(&scene, &cfg, &mut |_| {}).unwrap();

    let dir = std::env::temp_dir().join(format!("syfrack-twice-{}", std::process::id()));
    let mut session = RenderSession::new(r.clone(), &scene, &cfg).unwrap();
    assert!(!session.step(std::time::Duration::from_millis(1)).unwrap());
    session.save(&dir).unwrap();
    assert!(!session.step(std::time::Duration::from_millis(1)).unwrap());
    let latest = session.progress();
    session.save(&dir).unwrap();
    drop(session);

    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        ["accum.bin", "depth.bin", "normals.bin", "state.ron"]
    );
    let leftovers = std::fs::read_dir(dir.parent().unwrap())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(&format!("syfrack-twice-{}.tmp-", std::process::id()))
        })
        .count();
    assert_eq!(leftovers, 0, "temp directory must not remain");

    let mut resumed = RenderSession::resume(r.clone(), &dir).unwrap();
    assert_eq!(resumed.progress().samples_max, latest.samples_max);
    assert_eq!(resumed.progress().dispatches, latest.dispatches);
    while !resumed.step(std::time::Duration::from_millis(5)).unwrap() {}
    let out = resumed.readback().unwrap();
    assert_eq!(out.color.data, reference.color.data);
    assert_eq!(out.depth, reference.depth);
    assert_eq!(out.normals, reference.normals);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
#[ignore = "needs GPU"]
fn large_image_starts_small_and_stays_under_the_tdr_margin() {
    let r = renderer();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenes/exhibition.ron");
    let scene = Scene::load(&path).unwrap();
    let mut cfg = RenderConfig::new(1920, 1080, 2);
    cfg.max_bounces = 4;
    let mut session = RenderSession::new(r, &scene, &cfg).unwrap();
    while !session.step(std::time::Duration::from_millis(500)).unwrap() {}
    let p = session.progress();
    eprintln!("max_dispatch_ms = {}", p.max_dispatch_ms);
    assert_eq!(p.samples_min, 2);
    assert!(
        p.max_dispatch_ms < 1000,
        "slowest dispatch {} ms",
        p.max_dispatch_ms
    );
}

#[test]
#[ignore = "needs GPU"]
fn object_on_sky_scene_stays_under_the_dispatch_limit_with_the_default_ramp() {
    let r = renderer();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenes/mandelbulb.ron");
    let scene = Scene::load(&path).unwrap();
    let cfg = RenderConfig::new(1280, 720, 16);
    let mut session = RenderSession::new(r, &scene, &cfg).unwrap();
    while !session.step(std::time::Duration::from_millis(500)).unwrap() {}
    let p = session.progress();
    eprintln!("max_dispatch_ms = {}", p.max_dispatch_ms);
    assert_eq!(p.samples_min, 16);
    assert!(
        p.max_dispatch_ms < 400,
        "slowest dispatch {} ms",
        p.max_dispatch_ms
    );
}

#[test]
#[ignore = "needs GPU"]
fn save_refuses_to_replace_a_directory_with_foreign_files() {
    let r = renderer();
    let scene = Scene::example_for("sphere").unwrap();
    let cfg = RenderConfig::new(32, 32, 2);
    let session = RenderSession::new(r, &scene, &cfg).unwrap();
    let dir = std::env::temp_dir().join(format!("syfrack-foreign-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("important.txt"), "keep me").unwrap();
    let err = session.save(&dir).unwrap_err();
    assert!(format!("{err:#}").contains("not a SyFrack state directory"));
    assert_eq!(
        std::fs::read_to_string(dir.join("important.txt")).unwrap(),
        "keep me"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
#[ignore = "needs GPU"]
fn save_leaves_no_aside_copy_and_resumes_a_finished_state() {
    let r = renderer();
    let scene = Scene::example_for("sphere").unwrap();
    let mut cfg = RenderConfig::new(64, 64, 8);
    cfg.tile_size = 32;
    let mut session = RenderSession::new(r.clone(), &scene, &cfg).unwrap();
    while !session.step(std::time::Duration::from_millis(500)).unwrap() {}
    let dir = std::env::temp_dir().join(format!("syfrack-aside-{}", std::process::id()));
    session.save(&dir).unwrap();
    session.save(&dir).unwrap();
    let aside = dir.with_file_name(format!(
        "{}.old-{}",
        dir.file_name().unwrap().to_string_lossy(),
        std::process::id()
    ));
    assert!(!aside.exists(), "aside copy must be deleted");
    let resumed = RenderSession::resume(r, &dir).unwrap();
    assert!(resumed.is_done());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
#[ignore = "needs GPU"]
fn session_exposes_its_accumulator_and_size() {
    let r = renderer();
    let cfg = RenderConfig::new(40, 24, 2);
    let mut s = RenderSession::new(r.clone(), &Scene::example(), &cfg).unwrap();
    assert_eq!(s.image_size(), (40, 24));
    assert_eq!(s.accumulation_buffer().size(), 40 * 24 * 16);
    while !s.step(std::time::Duration::from_millis(5)).unwrap() {}
    let raw = r_ctx_read(&r, s.accumulation_buffer(), 40 * 24 * 16);
    let sums: Vec<[f32; 4]> = bytemuck::pod_collect_to_vec(&raw);
    assert!(
        sums.iter().all(|p| (p[3] - 2.0).abs() < 1e-6),
        "alpha counts samples"
    );
}

#[test]
#[ignore = "needs GPU"]
fn set_scene_params_rewrites_values_in_place_and_recompiles_on_topology_change() {
    let r = renderer();
    let cfg = RenderConfig::new(32, 32, 2);
    let a = Scene::example_for("mandelbox").unwrap();
    let mut graph = a.fractal.to_graph();
    let mut b = a.clone();
    for n in &mut graph.nodes {
        if let NodeKind::Scale { factor } = &mut n.kind {
            *factor = 2.4;
        }
    }
    b.fractal = Fractal::Graph(graph.clone());
    let fresh_b = r.render(&b, &cfg, &mut |_| {}).unwrap();

    let mut session = RenderSession::new(r.clone(), &a, &cfg).unwrap();
    while !session.step(std::time::Duration::from_millis(5)).unwrap() {}
    let before = session.fingerprint();
    assert!(
        session.set_scene_params(&b).unwrap(),
        "a scalar change keeps the topology"
    );
    assert_eq!(session.fingerprint(), before);
    assert_eq!(session.progress().samples_max, 0, "the image restarts");
    while !session.step(std::time::Duration::from_millis(5)).unwrap() {}
    assert_eq!(
        session.readback().unwrap().color.data,
        fresh_b.color.data,
        "the new values reach the shader"
    );

    let mut c = b.clone();
    graph.nodes[2].enabled = false; // the box fold inside the loop
    c.fractal = Fractal::Graph(graph);
    assert!(!session.set_scene_params(&c).unwrap());
    assert_ne!(session.fingerprint(), before);
    while !session.step(std::time::Duration::from_millis(5)).unwrap() {}
    let fresh_c = r.render(&c, &cfg, &mut |_| {}).unwrap();
    assert_eq!(session.readback().unwrap().color.data, fresh_c.color.data);
    assert_ne!(fresh_c.color.data, fresh_b.color.data);
}

fn render_scene(scene: &Scene, w: u32, h: u32, spp: u32) -> syfrack_core::HdrImage {
    renderer()
        .render(scene, &RenderConfig::new(w, h, spp), &mut |_| {})
        .unwrap()
        .color
}

/// 8x8 block means of an 8-bit RGB encoding, per channel.
fn block_means(img: &syfrack_color::EncodedImage) -> Vec<f32> {
    let (w, h) = (img.width as usize, img.height as usize);
    let mut out = Vec::new();
    for by in (0..h).step_by(8) {
        for bx in (0..w).step_by(8) {
            for c in 0..3 {
                let (mut sum, mut n) = (0.0f32, 0.0f32);
                for y in by..(by + 8).min(h) {
                    for x in bx..(bx + 8).min(w) {
                        sum += img.data[(y * w + x) * 3 + c] as f32;
                        n += 1.0;
                    }
                }
                out.push(sum / n);
            }
        }
    }
    out
}

#[test]
#[ignore = "needs GPU"]
fn classic_scenes_keep_their_colours_with_visible_ggx_sampling() {
    // Per-pixel equality is out of reach: the path tracer amplifies float
    // rounding differences into noise. Compare 8x8 block means instead.
    let mut failures = Vec::new();
    for name in ["mandelbox", "menger", "mandelbulb"] {
        let reference =
            syfrack_color::read_exr(format!("tests/fixtures/parity/{name}.exr")).unwrap();
        let scene = Scene::load(format!("../../scenes/{name}.ron")).unwrap();
        assert!(
            matches!(scene.fractal, Fractal::Graph(_)),
            "{name} must be a graph scene"
        );
        let img = render_scene(&scene, 640, 360, 64);
        let a = syfrack_color::develop_and_encode(&reference, &scene.look, 8);
        let b = syfrack_color::develop_and_encode(&img, &scene.look, 8);
        let (ma, mb) = (block_means(&a), block_means(&b));
        let diffs: Vec<f32> = ma.iter().zip(&mb).map(|(x, y)| (x - y).abs()).collect();
        let max = diffs.iter().cloned().fold(0.0f32, f32::max);
        let within2 = diffs.iter().filter(|d| **d <= 2.0).count() as f32 / diffs.len() as f32;
        let mut bias = [0.0f64; 3];
        for (x, y) in a.data.chunks(3).zip(b.data.chunks(3)) {
            for c in 0..3 {
                bias[c] += x[c] as f64 - y[c] as f64;
            }
        }
        let n = (a.data.len() / 3) as f64;
        let bias = bias.map(|v| v / n);
        eprintln!(
            "parity {name}: block max {max:.2}, {:.2} % of blocks <= 2, bias {bias:?}",
            within2 * 100.0
        );
        // VNDF changes sampled paths relative to the old NDF fixture. Keep
        // the regional means and global colour bias tight, allowing rare
        // noisy blocks a six-code-value bound in the 8-bit image.
        if max > 6.0 || within2 < 0.95 || bias.iter().any(|b| b.abs() > 0.5) {
            failures.push(format!(
                "{name}: block max {max:.2}, within2 {within2:.3}, bias {bias:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}

#[test]
#[ignore = "needs GPU"]
fn async_compile_delivers_a_pipeline_while_the_session_keeps_rendering() {
    let r = renderer();
    let scene = Scene::example_for("mandelbox").unwrap();
    let mut session = RenderSession::new(r.clone(), &scene, &RenderConfig::new(64, 36, 4)).unwrap();
    let mut g = scene.fractal.to_graph();
    g.nodes[2].enabled = false;
    let cached_before = r.cached_pipelines();
    let ticket = r.request(&g);
    let mut steps = 0;
    loop {
        match Renderer::poll(&ticket) {
            CompileStatus::Pending => {
                // A finished session returns at once: do not spin through the budget.
                if session.step(Duration::from_millis(5)).unwrap() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                steps += 1;
            }
            CompileStatus::Ready(p) => {
                let mut s2 = scene.clone();
                s2.fractal = Fractal::Graph(g.clone());
                session.swap_pipeline(p, &s2).unwrap();
                // A delivered status stays delivered.
                assert!(matches!(Renderer::poll(&ticket), CompileStatus::Ready(_)));
                assert!(matches!(Renderer::poll(&ticket), CompileStatus::Ready(_)));
                break;
            }
            CompileStatus::Failed(e) => panic!("{e}"),
        }
        assert!(steps < 100_000, "compile never finished");
    }
    assert!(steps > 0, "the session kept rendering while Pending");
    assert_eq!(r.cached_pipelines(), cached_before + 1);
    assert_eq!(
        session.fingerprint(),
        syfrack_render::graph_wgsl::fingerprint(&g)
    );
    session.step(Duration::from_millis(50)).unwrap();

    // Failed path: an invalid graph yields Failed (repeatedly), no panic.
    let mut bad = scene.fractal.to_graph();
    bad.links.clear();
    let ticket = r.request(&bad);
    let status = loop {
        match Renderer::poll(&ticket) {
            CompileStatus::Pending => std::thread::sleep(Duration::from_millis(5)),
            other => break other,
        }
    };
    assert!(matches!(status, CompileStatus::Failed(_)));
    assert!(matches!(Renderer::poll(&ticket), CompileStatus::Failed(_)));
    assert_eq!(r.cached_pipelines(), cached_before + 1);
}
