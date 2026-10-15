//! Two original native directions, with reproducible studies and full HDR exports.
use anyhow::{bail, Result};
use glam::Vec3;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use syfrack_color::{develop_and_encode, write_exr, write_png};
use syfrack_core::*;
use syfrack_render::{flame::FlameAccumulator, GpuContext, RenderConfig, RenderSession, Renderer};

fn rotation(scale: f32, angle: f32, x: f32, y: f32) -> [f32; 6] {
    let (sn, cs) = angle.to_radians().sin_cos();
    [cs * scale, -sn * scale, x, sn * scale, cs * scale, y]
}
fn term(amount: f32, variation: FlameVariation) -> FlameTerm {
    FlameTerm { amount, variation }
}
fn flame_study(i: usize) -> Scene {
    let mut s = Scene::example_2d();
    s.material.texture = None;
    s.seed = 7417;
    let copper = Ramp {
        stops: vec![
            [0.001, 0.002, 0.006],
            [0.018, 0.004, 0.002],
            [0.11, 0.013, 0.001],
            [0.70, 0.10, 0.008],
            [1.0, 0.44, 0.085],
            [0.95, 0.80, 0.44],
            [0.08, 0.24, 0.29],
            [0.003, 0.006, 0.012],
        ],
        interpolation: PaletteInterpolation::Smooth,
        ..Ramp::default()
    };
    let curl = [0.14, 0.28, -0.16, 0.40, 0.08, -0.30, 0.20, 0.34][i];
    let spin = [24.0, 37.0, 58.0, -31.0, 14.0, -48.0, 72.0, -12.0][i];
    let mut transforms = vec![
        FlameTransform {
            weight: 0.85,
            affine: rotation(0.73, spin, -0.22, 0.12),
            terms: vec![
                term(0.76, FlameVariation::Linear),
                term(0.24, FlameVariation::Swirl),
            ],
            color: 0.05,
            color_speed: 0.62,
            ..Default::default()
        },
        FlameTransform {
            weight: 0.48,
            affine: rotation(0.82, -spin * 1.3, 0.68, -0.24),
            terms: vec![
                term(
                    0.82,
                    FlameVariation::Curl {
                        c1: curl,
                        c2: 0.085,
                    },
                ),
                term(0.18, FlameVariation::Spherical),
            ],
            color: 0.56,
            color_speed: 0.48,
            ..Default::default()
        },
        FlameTransform {
            weight: 0.34,
            affine: rotation(0.63, spin + 110.0, -0.62, -0.15),
            terms: vec![
                term(
                    0.92,
                    FlameVariation::Julian {
                        power: 2,
                        distance: 1.0,
                    },
                ),
                term(0.08, FlameVariation::Swirl),
            ],
            post: rotation(1.0, 23.0, -0.35, 0.18),
            color: 0.70,
            color_speed: 0.25,
            ..Default::default()
        },
        FlameTransform {
            weight: 0.18,
            affine: rotation(0.54, spin - 70.0, 0.12, 0.70),
            terms: vec![
                term(0.90, FlameVariation::Linear),
                term(0.10, FlameVariation::Disc),
            ],
            color: 0.98,
            color_speed: 0.75,
            ..Default::default()
        },
    ];
    if i >= 4 {
        transforms[0].terms = vec![
            term(
                0.96,
                FlameVariation::Curl {
                    c1: curl,
                    c2: -0.12,
                },
            ),
            term(0.04, FlameVariation::Swirl),
        ];
        transforms[1].weight = 0.65;
    }
    s.canvas.flame = Some(Flame {
        transforms,
        palette: copper,
        brightness: 1.8,
        density_log: 14.0,
        filter_radius: 1.4,
        background: [0.0025, 0.0011, 0.0018],
        ..Default::default()
    });
    s.canvas.zoom = 2.05;
    s.canvas.center = [-0.1, 0.05];
    s.canvas.rotation = -12.0;
    s.look = Look::neutral();
    s.look.exposure_ev = 0.0;
    s.look.tonemap = Tonemap::Agx {
        slope: 1.08,
        power: 1.1,
        offset: 0.0,
    };
    s.look.saturation = 1.13;
    s.look.vignette = Some(Vignette {
        radius: 0.85,
        softness: 0.8,
        intensity: 0.32,
        elliptical: true,
    });
    s
}

fn architecture_study(i: usize) -> Scene {
    let mut s = Scene::example_for("dentelle-inversee").unwrap();
    let g = s.fractal.graph_mut();
    for node in &mut g.nodes {
        match &mut node.kind {
            NodeKind::Iterate { iterations, .. } => *iterations = 14,
            NodeKind::SphereInversion { radius, min_radius } => {
                *radius = [1.15, 1.13, 1.18, 1.10, 1.15, 1.13, 1.18, 1.10][i];
                *min_radius = 0.12;
            }
            NodeKind::Rotate { angles } => {
                *angles = Vec3::new(
                    [0.0, 2.5, 0.0, 1.0, 0.0, 2.5, 0.0, 1.0][i],
                    0.0,
                    [0.0, 0.0, 2.0, -1.5, 0.0, 0.0, 2.0, -1.5][i],
                )
            }
            NodeKind::Trap { scale, .. } => *scale = 2.8,
            _ => {}
        }
    }
    s.camera.mode = CameraMode::Free {
        position: Vec3::new(
            [2.7, 2.3, 1.8, 3.2, 0.55, 0.72, 1.0, 0.4][i],
            [1.35, 1.2, 0.9, 1.4, 0.3, 0.38, 0.48, 0.22][i],
            [3.6, 3.1, 2.8, 3.9, 1.0, 1.2, 1.5, 0.85][i],
        ),
        target: Vec3::new(0.0, 0.12, 0.0),
        roll: -18.0,
    };
    s.camera.focal_mm = if i < 4 { 32.0 } else { 24.0 };
    s.camera.f_number = 0.0;
    s.material.albedo = [0.88, 0.92, 0.96];
    s.material.metallic = 0.82;
    s.material.roughness = 0.24;
    s.material.coloring = Coloring::Orbit {
        channels: OrbitColoring {
            traps: [0.84, 0.0, 0.0, 0.0],
            logarithmic: 1.2,
            ..Default::default()
        },
        ramp: Ramp {
            stops: vec![
                [0.018, 0.028, 0.065],
                [0.055, 0.16, 0.28],
                [0.32, 0.63, 0.66],
                [0.83, 0.91, 0.96],
                [0.58, 0.45, 0.32],
            ],
            positions: vec![0.0, 0.24, 0.48, 0.70, 1.0],
            interpolation: PaletteInterpolation::Smooth,
            ..Default::default()
        },
    };
    s.lights = vec![
        Light::Environment {
            zenith: Color::Linear([0.12, 0.22, 0.38]),
            horizon: Color::Linear([0.50, 0.56, 0.64]),
            ground: Color::Linear([0.012, 0.022, 0.036]),
            intensity: 0.6,
            rotation: 0.0,
        },
        Light::Area {
            position: Vec3::new(-3.5, 6.0, 5.0),
            normal: Vec3::new(0.4, -0.65, -0.6).normalize(),
            size: [5.0, 3.0],
            color: Color::Linear([0.74, 0.88, 1.0]),
            intensity: 95.0,
        },
        Light::Area {
            position: Vec3::new(4.0, 1.0, -2.0),
            normal: Vec3::new(-1.0, 0.0, 0.5).normalize(),
            size: [2.4, 4.0],
            color: Color::Linear([0.07, 0.44, 1.0]),
            intensity: 48.0,
        },
        Light::Sun {
            direction: Vec3::new(-0.45, 0.8, -0.2).normalize(),
            color: Color::Linear([1.0, 0.53, 0.25]),
            intensity: 1.5,
            angular_size: 5.0,
        },
    ];
    s.atmosphere = Atmosphere::none();
    s.look = Look::neutral();
    s.look.exposure_ev = 0.55;
    s.look.tonemap = Tonemap::Agx {
        slope: 1.06,
        power: 1.10,
        offset: 0.0,
    };
    s.look.saturation = 1.03;
    s
}

fn export(scene: &Scene, image: &HdrImage, stem: &Path) -> Result<()> {
    let ron = scene.to_ron();
    write_png(
        &stem.with_extension("png"),
        &develop_and_encode(image, &scene.look, 16),
        scene.look.output,
        Some(&ron),
    )?;
    write_exr(&stem.with_extension("exr"), image)?;
    scene.save(stem.with_extension("ron"))?;
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() < 3 {
        bail!("duet_lab compose FOLDER | flame FOLDER OUTPUT [width height passes] | architecture FOLDER OUTPUT [width height samples]");
    }
    let folder = PathBuf::from(&args[2]);
    if args[1] == "compose" {
        for kind in ["flame", "architecture"] {
            let path = folder.join(kind);
            std::fs::create_dir_all(&path)?;
            for i in 0..8 {
                let scene = if kind == "flame" {
                    flame_study(i)
                } else {
                    architecture_study(i)
                };
                scene.validate()?;
                scene.save(path.join(format!("{kind}-{:02}.ron", i + 1)))?;
            }
        }
        return Ok(());
    }
    let output = PathBuf::from(
        args.get(3)
            .ok_or_else(|| anyhow::anyhow!("missing output"))?,
    );
    std::fs::create_dir_all(&output)?;
    let parse = |i: usize, default: u32| args.get(i).map_or(Ok(default), |s| s.parse::<u32>());
    let (w, h, spp) = (parse(4, 480)?, parse(5, 300)?, parse(6, 8)?);
    let mut files: Vec<_> = std::fs::read_dir(&folder)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|s| s == "ron"))
        .collect();
    files.sort();
    let renderer = if args[1] == "architecture" {
        Some(Renderer::new(Arc::new(GpuContext::new()?))?)
    } else {
        None
    };
    for file in files {
        let scene = Scene::load(&file)?;
        scene.validate()?;
        let stem = output.join(file.file_stem().unwrap());
        if stem.with_extension("png").exists() {
            bail!("output exists: {}", stem.display());
        }
        let start = Instant::now();
        if let Some(renderer) = &renderer {
            let mut cfg = RenderConfig::new(w, h, spp);
            cfg.max_steps = 512;
            cfg.max_bounces = 2;
            cfg.pixel_tolerance = 0.25;
            cfg.tile_size = 32;
            let mut session = RenderSession::new(renderer.clone(), &scene, &cfg)?;
            while !session.step(Duration::from_millis(250))? {}
            let raw = session.readback()?;
            export(&scene, &raw.color, &stem)?;
        } else {
            let mut flame = FlameAccumulator::new(&scene.canvas, scene.seed, w, h)?;
            flame.advance(spp);
            let image = flame.reconstruct();
            export(&scene, &image, &stem)?;
        }
        println!(
            "{} {}x{} {} samples {:.2}s",
            stem.display(),
            w,
            h,
            spp,
            start.elapsed().as_secs_f64()
        );
    }
    Ok(())
}
