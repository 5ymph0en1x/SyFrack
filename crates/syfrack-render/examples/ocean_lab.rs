//! Small, reproducible native scene to inspect the ocean before city renders.
//! cargo run -p syfrack-render --example ocean_lab --offline -- renders/studies/ocean 32 800
use anyhow::Result;
use glam::Vec3;
use std::{path::PathBuf, sync::Arc};
use syfrack_color::{develop_and_encode, write_exr, write_png};
use syfrack_core::{
    Atmosphere, CameraMode, Color, Coloring, Finish, FinishDetail, FinishPattern, Fractal, Graph,
    Light, Link, Node, NodeKind, OceanSurface, Placement, Scene, Seascape,
};
use syfrack_render::{GpuContext, RenderConfig, Renderer};

fn add(g: &mut Graph, kind: NodeKind, inputs: &[u32]) -> u32 {
    let id = g.next_id();
    g.nodes.push(Node {
        id,
        name: kind.label().into(),
        kind,
        pos: [id as f32 * 70.0, 0.0],
        enabled: true,
    });
    for (port, &from) in inputs.iter().enumerate() {
        g.links.push(Link {
            from: (from, 0),
            to: (id, port as u32),
        });
    }
    id
}

fn solid(g: &mut Graph, position: Vec3, rotation: Vec3, kind: NodeKind, material: u32) -> u32 {
    let bound = match &kind {
        NodeKind::Rock { half, erosion, .. } => Some(*half * (1.0 + 0.5 * erosion)),
        _ => None,
    };
    let p = add(g, NodeKind::Translate { offset: -position }, &[0]);
    let p = add(g, NodeKind::Rotate { angles: rotation }, &[p]);
    let d = add(g, kind, &[p]);
    let d = add(g, NodeKind::MaterialTag { index: material }, &[d]);
    if let Some(half) = bound {
        add(g, NodeKind::Bounded { half, margin: 0.15 }, &[p, d])
    } else {
        d
    }
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let dir = args
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| "renders/studies/ocean".into());
    let samples: u32 = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(32);
    let width: u32 = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(800);
    std::fs::create_dir_all(&dir)?;
    let mut s = Scene::example();
    let mut g = Graph::new_minimal();
    g.nodes.truncate(1);
    g.links.clear();
    g.bounding_radius_hint = 40.0;
    let mut parts = Vec::new();
    for (position, half, angles) in [
        ((-3.7, -0.6, -1.8), (2.3, 1.5, 1.5), (12.0, 24.0, -7.0)),
        ((-2.6, 0.5, -2.4), (1.6, 0.8, 1.0), (-14.0, -7.0, 9.0)),
        ((3.1, -0.65, -5.0), (2.0, 1.4, 1.35), (4.0, -31.0, 12.0)),
        ((0.4, -0.9, 2.7), (1.05, 1.1, 1.5), (9.0, 12.0, -11.0)),
        ((-5.2, -1.2, 3.3), (0.85, 1.0, 0.9), (-8.0, 39.0, 6.0)),
    ] {
        parts.push(solid(
            &mut g,
            Vec3::from(position),
            Vec3::from(angles),
            NodeKind::Rock {
                half: Vec3::from(half),
                erosion: 0.48,
                seed: (parts.len() as u32) * 317 + 73,
            },
            1,
        ));
    }
    // Angular, independently eroded shore rubble replaces the artificial markers.
    let mut random = 421073u32;
    let mut unit = || {
        random = random.wrapping_mul(1664525).wrapping_add(1013904223);
        (random >> 8) as f32 / 16777216.0
    };
    let mut rubble = Vec::new();
    for _ in 0..24 {
        let angle = unit() * std::f32::consts::TAU;
        let radius = 1.2 + unit() * 3.0;
        let size = 0.2 + unit() * 0.55;
        let position = Vec3::new(
            -3.2 + angle.cos() * radius,
            -0.3 - unit() * 0.6,
            -2.0 + angle.sin() * radius,
        );
        rubble.push(Placement {
            position,
            angles: Vec3::new(unit() * 30.0, unit() * 180.0, unit() * 25.0),
            scale: Vec3::splat(size),
        });
    }
    let block = add(
        &mut g,
        NodeKind::Rock {
            half: Vec3::new(1.0, 0.8, 1.3),
            erosion: 0.5,
            seed: 1473,
        },
        &[0],
    );
    let block = add(&mut g, NodeKind::MaterialTag { index: 1 }, &[block]);
    parts.push(add(
        &mut g,
        NodeKind::Instances {
            placements: rubble,
            half: Vec3::new(1.26, 1.01, 1.64),
            margin: 0.15,
        },
        &[block],
    ));
    // A warm beacon produces a real reflected light, not a painted highlight.
    parts.push(solid(
        &mut g,
        Vec3::new(-3.5, 1.25, -2.2),
        Vec3::ZERO,
        NodeKind::Cylinder {
            radius: 0.12,
            half_height: 1.0,
        },
        4,
    ));
    parts.push(solid(
        &mut g,
        Vec3::new(-3.5, 2.3, -2.2),
        Vec3::ZERO,
        NodeKind::Sphere { radius: 0.21 },
        5,
    ));
    let mut root = parts[0];
    for part in &parts[1..] {
        root = add(&mut g, NodeKind::Union, &[root, *part]);
    }
    add(&mut g, NodeKind::Output, &[root]);
    s.fractal = Fractal::Graph(g);
    s.camera.mode = CameraMode::Free {
        position: Vec3::new(12.0, 9.0, 18.0),
        target: Vec3::new(-0.5, 0.0, -1.0),
        roll: 0.0,
    };
    s.camera.focal_mm = 43.0;
    s.camera.f_number = 0.0;
    s.material.coloring = Coloring::None;
    s.material.finishes = vec![
        Finish {
            name: "Wet basalt".into(),
            albedo: [0.26, 0.28, 0.25],
            roughness: 0.68,
            detail: Some(FinishDetail {
                pattern: FinishPattern::CoastalRock,
                seed: 71,
                scale: 6.0,
                strength: 0.55,
                bump: 0.015,
                roughness_delta: 0.18,
                tint: [0.09, 0.10, 0.085],
            }),
            ..Default::default()
        },
        Finish {
            name: "Submerged limestone".into(),
            albedo: [0.75, 0.6, 0.36],
            roughness: 0.6,
            ..Default::default()
        },
        Finish {
            name: "Submerged copper".into(),
            albedo: [0.3, 0.09, 0.025],
            roughness: 0.4,
            metallic: 0.65,
            ..Default::default()
        },
        Finish {
            name: "Beacon bronze".into(),
            albedo: [0.35, 0.2, 0.08],
            roughness: 0.24,
            metallic: 0.85,
            ..Default::default()
        },
        Finish {
            name: "Amber lantern".into(),
            albedo: [0.6, 0.2, 0.04],
            emission: [12.0, 4.0, 0.65],
            ..Default::default()
        },
    ];
    s.atmosphere = Atmosphere::none();
    s.atmosphere.seascape = Some(Seascape {
        sea_level: 0.0,
        sky_zenith: [0.065, 0.14, 0.30],
        sky_horizon: [0.48, 0.33, 0.24],
        water_color: [0.045, 0.16, 0.19],
        intensity: 1.0,
        clouds: 0.28,
        surface: Some(OceanSurface {
            wave_height: 0.6,
            wavelength: 6.0,
            wind_direction: -28.0,
            crest_sharpness: 0.65,
            time: 8.0,
            foam_amount: 0.6,
            shore_width: 0.15,
            ripple_strength: 0.32,
            roughness: 0.10,
            depth: 6.0,
            absorption: [0.18, 0.045, 0.025],
            scattering: 0.04,
            ..Default::default()
        }),
        ..Default::default()
    });
    s.lights = vec![
        Light::Sun {
            direction: Vec3::new(-0.5, 0.22, -0.7).normalize(),
            color: Color::Kelvin(4200.0),
            intensity: 0.85,
            angular_size: 3.0,
        },
        Light::Area {
            position: Vec3::new(-20.0, 40.0, 30.0),
            normal: Vec3::new(0.3, -1.0, -0.3).normalize(),
            size: [25.0; 2],
            color: Color::Kelvin(9500.0),
            intensity: 0.65,
        },
        Light::Area {
            position: Vec3::new(-3.5, 2.2, -2.2),
            normal: Vec3::NEG_Y,
            size: [0.3; 2],
            color: Color::Kelvin(2400.0),
            intensity: 55.0,
        },
    ];
    s.look.exposure_ev = 1.2;
    s.validate()?;
    s.save(dir.join("ocean.ron"))?;
    if width == 0 {
        return Ok(());
    } // scene-only export for CLI / portable presets
    let context = Arc::new(GpuContext::new()?);
    println!("GPU: {}", context.info.name);
    let r = Renderer::new(context)?;
    let mut cfg = RenderConfig::new(width, width * 9 / 16, samples);
    cfg.max_bounces = 3;
    cfg.tile_size = 8;
    cfg.samples_per_dispatch = 1;
    cfg.pixel_tolerance = 0.5;
    let mut reported = 0;
    let out = r.render(&s, &cfg, &mut |p| {
        let seconds = p.elapsed.as_secs();
        if seconds >= reported + 15 {
            println!(
                "{} dispatches, {}..{} / {} spp, {}s, max {}ms",
                p.dispatches, p.samples_min, p.samples_max, p.target, seconds, p.max_dispatch_ms
            );
            reported = seconds;
        }
    })?;
    write_exr(dir.join("ocean.exr"), &out.color)?;
    let image = syfrack_render::denoise::denoise(&out.color, &out.depth, &out.normals);
    let encoded = develop_and_encode(&image, &s.look, 16);
    write_png(
        dir.join("ocean.png"),
        &encoded,
        s.look.output,
        Some(&s.to_ron()),
    )?;
    println!(
        "Ocean: {width}x{}, {samples} spp, {:.2}s",
        cfg.height,
        out.elapsed.as_secs_f64()
    );
    Ok(())
}
