//! Native portrait artwork studies, with a persistent GPU for final curation.
//! fresco_lab OUTPUT; stdin: SCENE OUTPUT_STEM WIDTH HEIGHT SPP BOUNCES STEPS
use anyhow::{ensure, Context, Result};
use glam::Vec3;
use std::{
    io::BufRead,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use syfrack_color::{develop_and_encode, write_exr, write_png};
use syfrack_core::*;
use syfrack_render::{
    ComputeIntegrator, ExportMode, ExportSession, GpuContext, RenderConfig, Renderer,
};

fn node(g: &mut Graph, kind: NodeKind, name: &str) -> u32 {
    let id = g.nodes.len() as u32;
    g.nodes.push(Node {
        id,
        kind,
        name: name.into(),
        enabled: true,
        pos: [220.0 * (id % 7) as f32, 145.0 * (id / 7) as f32],
    });
    id
}
fn link(g: &mut Graph, a: u32, b: u32, port: u32) {
    g.links.push(Link {
        from: (a, 0),
        to: (b, port),
    });
}
fn chain(g: &mut Graph, a: u32, kind: NodeKind, name: &str) -> u32 {
    let b = node(g, kind, name);
    link(g, a, b, 0);
    b
}
fn combine(g: &mut Graph, a: u32, b: u32, kind: NodeKind, name: &str) -> u32 {
    let c = node(g, kind, name);
    link(g, a, c, 0);
    link(g, b, c, 1);
    c
}
fn area(position: Vec3, target: Vec3, size: [f32; 2], rgb: [f32; 3], intensity: f32) -> Light {
    Light::Area {
        position,
        normal: (target - position).normalize(),
        size,
        color: Color::Linear(rgb),
        intensity,
    }
}

fn artwork(variant: usize) -> Scene {
    let mut g = Graph::new_minimal();
    g.nodes.clear();
    g.links.clear();
    let input = node(&mut g, NodeKind::Input, "Espace de la mémoire");
    let axial = chain(
        &mut g,
        input,
        NodeKind::Rotate {
            angles: Vec3::new(90.0, 0.0, 0.0),
        },
        "Axe des générations",
    );
    let crown = chain(
        &mut g,
        axial,
        NodeKind::Kaleido {
            planes: 7,
            phase: 8.0,
        },
        "Sept nefs autour du vide",
    );
    let local = chain(
        &mut g,
        crown,
        NodeKind::Translate {
            offset: Vec3::new(-1.28, 0.0, 0.0),
        },
        "Décentrer les voûtes",
    );
    let fold = node(
        &mut g,
        NodeKind::MengerFold {
            scale: 2.78,
            offset: Vec3::ONE,
        },
        "Architecture de Menger",
    );
    let sphere = node(
        &mut g,
        NodeKind::SphereFold {
            min_radius: 0.68,
            fixed_radius: 1.0,
        },
        "Voûtes inversées",
    );
    let rotate = node(
        &mut g,
        NodeKind::Rotate {
            angles: Vec3::new(0.0, 11.0, 7.0),
        },
        "Mémoire oblique",
    );
    let constant = node(
        &mut g,
        NodeKind::AddConstant {
            multiplier: Vec3::ZERO,
            julia: Vec3::new(0.055, 0.018, -0.075),
        },
        "Dérive entre les échelles",
    );
    let trap = node(
        &mut g,
        NodeKind::Trap {
            slot: 0,
            measure: Measure::Norm,
            scale: 1.8,
            mode: TrapMode::Minimum,
        },
        "Strates orbitaires",
    );
    let angle = node(
        &mut g,
        NodeKind::Trap {
            slot: 1,
            measure: Measure::AxisZ,
            scale: 1.0,
            mode: TrapMode::Minimum,
        },
        "Patine des galeries",
    );
    let iter = chain(
        &mut g,
        local,
        NodeKind::Iterate {
            iterations: 12,
            bailout: 100.0,
            body: vec![fold, sphere, rotate, constant, trap, angle],
        },
        "Douze générations de pierre et d'or",
    );
    let surface = chain(
        &mut g,
        iter,
        NodeKind::Box { half: Vec3::ONE },
        "Surface de la mémoire",
    );
    let shaft = chain(
        &mut g,
        axial,
        NodeKind::Cylinder {
            radius: 0.47,
            half_height: 7.0,
        },
        "Ouverture des époques",
    );
    let perforated = combine(
        &mut g,
        surface,
        shaft,
        NodeKind::Subtraction,
        "Conserver un chemin de lumière",
    );
    let ball = chain(
        &mut g,
        input,
        NodeKind::Sphere { radius: 4.6 },
        "Domaine de la nef",
    );
    let bounded = combine(
        &mut g,
        perforated,
        ball,
        NodeKind::Intersection,
        "Corolle récursive",
    );
    let copies = node(
        &mut g,
        NodeKind::Instances {
            placements: vec![
                Placement {
                    position: Vec3::ZERO,
                    angles: Vec3::ZERO,
                    scale: Vec3::ONE,
                },
                Placement {
                    position: Vec3::new(0.0, 0.0, -2.9),
                    angles: Vec3::new(0.0, 0.0, 19.0),
                    scale: Vec3::splat(0.38),
                },
                Placement {
                    position: Vec3::new(0.0, 0.0, -4.15),
                    angles: Vec3::new(0.0, 0.0, -11.0),
                    scale: Vec3::splat(0.14),
                },
            ],
            half: Vec3::splat(4.65),
            margin: 0.12,
        },
        "Les trois âges de la cathédrale",
    );
    link(&mut g, bounded, copies, 0);
    // The two fine circles mark the final aperture in actual 3D geometry.
    let iris_origin = chain(
        &mut g,
        input,
        NodeKind::Translate {
            offset: Vec3::new(0.0, 0.0, 4.75),
        },
        "L'horizon de la mémoire",
    );
    let iris_point = chain(
        &mut g,
        iris_origin,
        NodeKind::Rotate {
            angles: Vec3::new(90.0, 0.0, 0.0),
        },
        "Plan de l'iris",
    );
    let iris = chain(
        &mut g,
        iris_point,
        NodeKind::Torus {
            radius: 0.29,
            tube: 0.012,
        },
        "Premier cercle de lumière",
    );
    let iris2 = chain(
        &mut g,
        iris_point,
        NodeKind::Torus {
            radius: 0.34,
            tube: 0.01,
        },
        "Second cercle de lumière",
    );
    let circles = combine(&mut g, iris, iris2, NodeKind::Union, "Iris double");
    let luminous = chain(
        &mut g,
        circles,
        NodeKind::MaterialTag { index: 1 },
        "Or incandescent",
    );
    let total = combine(
        &mut g,
        copies,
        luminous,
        NodeKind::Union,
        "Architecture et lumière",
    );
    chain(
        &mut g,
        total,
        NodeKind::Output,
        "La mémoire des cathédrales",
    );
    g.safety = 0.6;
    g.bounding_radius_hint = 8.5;

    let mut s = Scene::example();
    s.seed = 1910202607;
    s.fractal = Fractal::Graph(g);
    let (position, target, roll, focal) = match variant {
        0 => (
            Vec3::new(0.45, -0.85, 5.2),
            Vec3::new(-0.1, -0.35, -2.0),
            -8.0,
            40.0,
        ),
        1 => (
            Vec3::new(0.75, -1.4, 4.6),
            Vec3::new(-0.15, -0.4, -2.5),
            -14.0,
            38.0,
        ),
        _ => (
            Vec3::new(-0.4, -0.65, 4.3),
            Vec3::new(0.15, -0.45, -2.3),
            9.0,
            44.0,
        ),
    };
    s.camera.mode = CameraMode::Free {
        position,
        target,
        roll,
    };
    s.camera.focal_mm = focal;
    s.camera.f_number = 0.0;
    s.camera.focus_distance = 6.0;
    s.material.albedo = [0.82, 0.77, 0.68];
    s.material.roughness = 0.33;
    s.material.metallic = 0.62;
    s.material.specular = 0.04;
    s.material.texture = None;
    s.material.coloring = Coloring::Orbit {
        channels: OrbitColoring {
            traps: [0.85, 0.15, 0.0, 0.0],
            ..Default::default()
        },
        ramp: Ramp {
            stops: vec![
                [0.018, 0.028, 0.048],
                [0.05, 0.16, 0.20],
                [0.24, 0.39, 0.38],
                [0.60, 0.44, 0.20],
                [0.91, 0.73, 0.40],
                [0.25, 0.10, 0.035],
            ],
            cyclic: true,
            frequency: 2.3,
            phase: 0.08,
            interpolation: PaletteInterpolation::Smooth,
            ..Default::default()
        },
    };
    s.material.surface = Some(SurfaceVariation {
        slot: 0,
        roughness: [0.25, 0.5],
        metallic: [0.85, 0.25],
    });
    s.material.finishes = vec![Finish {
        name: "Or incandescent de l'iris".into(),
        albedo: [0.8, 0.42, 0.1],
        roughness: 0.25,
        metallic: 0.7,
        emission: [7.0, 2.4, 0.4],
        ..Default::default()
    }];
    s.lights = vec![
        Light::Environment {
            zenith: Color::Linear([0.015, 0.04, 0.09]),
            horizon: Color::Linear([0.04, 0.055, 0.075]),
            ground: Color::Linear([0.006, 0.012, 0.025]),
            intensity: 0.4,
            rotation: 0.0,
        },
        area(
            Vec3::new(-3.5, 4.0, 3.0),
            Vec3::ZERO,
            [3.5, 4.0],
            [1.0, 0.66, 0.3],
            16.0,
        ),
        area(
            Vec3::new(3.5, -0.8, 2.5),
            Vec3::ZERO,
            [2.5, 5.0],
            [0.1, 0.48, 0.8],
            12.0,
        ),
        area(
            Vec3::new(0.0, 1.0, -2.4),
            Vec3::new(0.0, 0.0, 1.0),
            [0.7, 0.9],
            [1.0, 0.55, 0.18],
            22.0,
        ),
        Light::Tube {
            start: Vec3::new(0.0, 0.0, -5.3),
            end: Vec3::new(0.0, 0.02, -5.3),
            radius: 0.17,
            color: Color::Linear([1.0, 0.68, 0.3]),
            intensity: 14.0,
        },
    ];
    s.atmosphere = Atmosphere::none();
    s.atmosphere.fog_density = 0.005;
    s.atmosphere.fog_color = [0.10, 0.17, 0.24];
    s.atmosphere.fog_height = 0.0;
    s.atmosphere.fog_falloff = 0.06;
    s.atmosphere.anisotropy = 0.15;
    s.atmosphere.cavity_darkness = 0.18;
    s.look = Look::neutral();
    s.look.auto_exposure = None;
    s.look.exposure_ev = 1.0;
    s.look.tonemap = Tonemap::Agx {
        slope: 1.0,
        power: 1.18,
        offset: 0.0,
    };
    s.look.saturation = 1.07;
    s.look.vibrance = 0.12;
    s.look.vignette = Some(Vignette {
        radius: 0.8,
        softness: 0.8,
        intensity: 0.2,
        elliptical: true,
    });
    s
}

fn render(r: &Renderer, s: &Scene, stem: &Path, cfg: &RenderConfig) -> Result<()> {
    ensure!(
        !stem.with_extension("png").exists(),
        "output already exists: {}",
        stem.display()
    );
    s.validate()?;
    let started = Instant::now();
    let mode = if s.dimension == Dimension::TwoD {
        ExportMode::Auto
    } else {
        ExportMode::Wavefront
    };
    let mut session = ExportSession::new(r.clone(), s, cfg, mode)?;
    let mut report = Instant::now();
    while !session.step(Duration::from_millis(100))? {
        if report.elapsed() >= Duration::from_secs(20) {
            let p = session.progress();
            println!(
                "{} {:.1}% {}..{} spp {:.1}s",
                stem.display(),
                100.0 * p.fraction_complete(),
                p.samples_min,
                p.samples_max,
                started.elapsed().as_secs_f64()
            );
            report = Instant::now();
        }
    }
    session.stop_workers();
    let image = session.readback()?;
    ensure!(
        image.samples_min == cfg.samples_per_pixel && image.samples_max == cfg.samples_per_pixel,
        "incomplete render"
    );
    ensure!(
        image.color.data.iter().flatten().all(|v| v.is_finite()),
        "non-finite radiance"
    );
    let filtered = if cfg.denoise {
        syfrack_render::denoise::session_output(&session, &image)
    } else {
        image.color.clone()
    };
    let ron = s.to_ron();
    write_png(
        stem.with_extension("png"),
        &develop_and_encode(&filtered, &s.look, 16),
        s.look.output,
        Some(&ron),
    )?;
    write_exr(stem.with_extension("exr"), &image.color)?;
    std::fs::write(stem.with_extension("ron"), ron)?;
    std::fs::write(
        stem.with_extension("config.ron"),
        ron::ser::to_string_pretty(cfg, Default::default())?,
    )?;
    println!(
        "DONE {}: {}x{} {} spp in {:.2}s",
        stem.display(),
        cfg.width,
        cfg.height,
        cfg.samples_per_pixel,
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let dir = Path::new(args.get(1).context("OUTPUT directory required")?);
    std::fs::create_dir_all(dir)?;
    let r = Renderer::new(Arc::new(GpuContext::new()?))?;
    println!(
        "GPU: {} / {:?}",
        r.context().info.name,
        r.context().info.backend
    );
    let mut cfg = RenderConfig::new(270, 480, 4);
    cfg.integrator = ComputeIntegrator::Wavefront;
    cfg.max_bounces = 1;
    cfg.max_steps = 384;
    cfg.pixel_tolerance = 0.3;
    cfg.tile_size = 128;
    cfg.samples_per_dispatch = 4;
    cfg.dispatch_budget_ms = 150;
    for i in 0..if args.get(2).is_some_and(|s| s == "--interactive") {
        0
    } else {
        3
    } {
        let s = artwork(i);
        render(&r, &s, &dir.join(format!("etude-{:02}", i + 1)), &cfg)?;
    }
    println!("READY: SCENE OUTPUT_STEM WIDTH HEIGHT SPP BOUNCES STEPS (or quit)");
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        if line.trim() == "quit" {
            break;
        }
        let values: Vec<_> = line.split_whitespace().collect();
        ensure!(
            values.len() == 7,
            "expected scene, stem, width, height, spp, bounces, steps"
        );
        let scene = Scene::load(values[0])?;
        let mut final_cfg = cfg.clone();
        if scene.dimension == Dimension::TwoD {
            final_cfg.integrator = ComputeIntegrator::Megakernel;
        }
        final_cfg.width = values[2].parse()?;
        final_cfg.height = values[3].parse()?;
        final_cfg.samples_per_pixel = values[4].parse()?;
        final_cfg.max_bounces = values[5].parse()?;
        final_cfg.max_steps = values[6].parse()?;
        final_cfg.samples_per_dispatch = 8;
        final_cfg.tile_size = 128;
        final_cfg.pixel_tolerance = 0.2;
        render(&r, &scene, Path::new(values[1]), &final_cfg)?;
        println!("READY");
    }
    Ok(())
}
