//! Native art direction: coherent quantum waves, a fractal crown and cosmic dust.
//! cargo run -p syfrack-render --example forge_des_mondes -- study|scene INDEX
use anyhow::{ensure, Result};
use glam::Vec3;
use std::{path::PathBuf, sync::Arc, time::Instant};
use syfrack_color::{develop_and_encode, write_exr, write_png};
use syfrack_core::{scene::phenomena::*, *};
use syfrack_render::{ComputeIntegrator, GpuContext, RenderConfig, Renderer};

const OUTPUT: &str = "renders/la-forge-des-mondes-20261020";

fn node(g: &mut Graph, kind: NodeKind, name: &str) -> u32 {
    let id = g.nodes.len() as u32;
    g.nodes.push(Node {
        id,
        kind,
        name: name.into(),
        pos: [(id % 6) as f32 * 240.0, (id / 6) as f32 * 160.0],
        enabled: true,
    });
    id
}
fn link(g: &mut Graph, from: u32, to: u32, port: u32) {
    g.links.push(Link {
        from: (from, 0),
        to: (to, port),
    });
}
fn chain(g: &mut Graph, from: u32, kind: NodeKind, name: &str) -> u32 {
    let to = node(g, kind, name);
    link(g, from, to, 0);
    to
}
fn crown() -> Graph {
    let mut g = Graph::new_minimal();
    g.nodes.clear();
    g.links.clear();
    g.safety = 0.72;
    g.bounding_radius_hint = 4.3;
    let input = node(&mut g, NodeKind::Input, "Espace de la Forge");
    let rotated = chain(
        &mut g,
        input,
        NodeKind::Rotate {
            angles: Vec3::new(0.0, 17.0, 0.0),
        },
        "Orientation de la dentelle",
    );
    let twisted = chain(
        &mut g,
        rotated,
        NodeKind::Twist { rate: 0.12 },
        "Torsion des chambres",
    );
    let scaled = chain(
        &mut g,
        twisted,
        NodeKind::Scale { factor: 1.0 },
        "Échelle des chambres",
    );
    let tiled = chain(
        &mut g,
        scaled,
        NodeKind::Tile {
            period: Vec3::splat(2.0),
        },
        "Réseau des chambres imbriquées",
    );
    let fold = node(
        &mut g,
        NodeKind::MengerFold {
            scale: 3.0,
            offset: Vec3::ONE,
        },
        "Repli du récif",
    );
    let trap = node(
        &mut g,
        NodeKind::Trap {
            slot: 0,
            measure: Measure::Norm,
            scale: 3.0,
            mode: TrapMode::Minimum,
        },
        "Mémoire des chambres imbriquées",
    );
    let it = chain(
        &mut g,
        tiled,
        NodeKind::Iterate {
            iterations: 5,
            bailout: 1.0e6,
            body: vec![fold, trap],
        },
        "Cinq générations de cavités",
    );
    let sponge = chain(
        &mut g,
        it,
        NodeKind::Box { half: Vec3::ONE },
        "Architecture récursive",
    );
    let torus = chain(
        &mut g,
        input,
        NodeKind::Torus {
            radius: 2.50,
            tube: 0.90,
        },
        "Couronne cosmique",
    );
    let cut = node(&mut g, NodeKind::Intersection, "Dentelle dans la couronne");
    link(&mut g, sponge, cut, 0);
    link(&mut g, torus, cut, 1);
    let mut assembly = cut;
    for (angle, radius, tube) in [
        (Vec3::new(59.0, 13.0, 38.0), 1.96, 0.026),
        (Vec3::new(-46.0, -12.0, -32.0), 1.54, 0.032),
    ] {
        let oriented = chain(
            &mut g,
            input,
            NodeKind::Rotate { angles: angle },
            "Plan orbital",
        );
        let orbit = chain(
            &mut g,
            oriented,
            NodeKind::Torus { radius, tube },
            "Orbite de l'armillaire",
        );
        let gold = chain(
            &mut g,
            orbit,
            NodeKind::MaterialTag { index: 1 },
            "Fil d'or incandescent",
        );
        let union = node(&mut g, NodeKind::Union, "Assemblage des échelles");
        link(&mut g, assembly, union, 0);
        link(&mut g, gold, union, 1);
        assembly = union;
    }
    chain(&mut g, assembly, NodeKind::Output, "La Forge des Mondes");
    g
}
fn field(id: u32, op: FieldOp, inputs: &[u32], name: &str) -> FieldNode {
    FieldNode {
        id,
        name: name.into(),
        op,
        inputs: inputs.into(),
        pos: [(id % 5) as f32 * 230.0, (id / 5) as f32 * 150.0],
    }
}
fn phenomena() -> Phenomena {
    let mut p = Phenomena::default();
    p.domain.half = Vec3::splat(4.0);
    p.domain.resolution = [48, 48, 48];
    p.domain.memory_budget_mib = 128;
    p.time = 0.37;
    p.simulations.push(Simulation {
        name: "Germe du monde — vortex de Gross–Pitaevskii".into(),
        model: SimulationModel::GrossPitaevskii {
            dispersion: 0.03,
            interaction: 0.5,
            damping: 0.0,
        },
        steps: 48,
        dt: 0.01,
        seed: 20261020,
        seeds: vec![SeedBlob {
            radius: 0.95,
            winding: 1,
            ..Default::default()
        }],
        ..Default::default()
    });
    p.fields.nodes = vec![
        field(0, FieldOp::Position, &[], "Position"),
        field(
            1,
            FieldOp::GaussianWave {
                center: Vec3::new(-0.22, 0.0, 0.0),
                width: 0.88,
                wavevector: Vec3::new(11.0, 3.0, -2.5),
                frequency: 1.0,
            },
            &[0],
            "Onde primordiale",
        ),
        field(
            2,
            FieldOp::GaussianWave {
                center: Vec3::new(0.22, 0.0, 0.0),
                width: 0.88,
                wavevector: Vec3::new(-8.0, 5.0, 3.0),
                frequency: 1.35,
            },
            &[0],
            "Onde conjuguée",
        ),
        field(3, FieldOp::Add, &[1, 2], "Interférence cohérente"),
        field(4, FieldOp::Intensity, &[3], "Densité des franges"),
        field(5, FieldOp::Phase, &[3], "Phase de la lumière"),
        field(
            6,
            FieldOp::Snapshot {
                simulation: 0,
                channel: 2,
            },
            &[0],
            "Densité du vortex",
        ),
        field(7, FieldOp::Scalar { value: 4.0 }, &[], "Énergie du germe"),
        field(8, FieldOp::Scale, &[6, 7], "Vortex lumineux"),
        field(9, FieldOp::Add, &[4, 8], "Matière et interférence"),
        field(
            10,
            FieldOp::GeometryTrap { slot: 0 },
            &[0],
            "Orbites de la couronne",
        ),
        field(
            11,
            FieldOp::Scalar { value: 0.35 },
            &[],
            "Amplitude chromatique",
        ),
        field(12, FieldOp::Scale, &[5, 11], "Phase chromatique"),
        field(
            13,
            FieldOp::Add,
            &[10, 12],
            "Empreinte du quantique dans la matière",
        ),
    ];
    p.bindings.density = Some(9);
    p.bindings.color = Some(13);
    p.bindings.density_gain = 0.12;
    p.bindings.emission_gain = 0.19;
    p.bindings.film_thickness = Some(13);
    p.bindings.palette = Ramp {
        stops: vec![
            [0.007, 0.025, 0.055],
            [0.014, 0.18, 0.22],
            [0.12, 0.53, 0.47],
            [0.86, 0.67, 0.30],
            [0.93, 0.28, 0.045],
            [0.19, 0.035, 0.10],
            [0.025, 0.06, 0.13],
        ],
        cyclic: true,
        frequency: 1.8,
        phase: 0.16,
        interpolation: PaletteInterpolation::Smooth,
        ..Default::default()
    };
    p.film = Some(ThinFilm {
        thickness_nm: 340.0,
        thickness_range_nm: 420.0,
        ior: 1.38,
        substrate_ior: 0.8,
        substrate_extinction: 2.4,
        strength: 0.45,
        dispersion_b: 0.003,
    });
    p
}
fn scene(index: usize) -> Result<Scene> {
    let mut s = Scene::example();
    s.seed = 20261020;
    s.fractal = Fractal::Graph(crown());
    s.camera.mode = CameraMode::Free {
        position: [
            Vec3::new(5.0, 7.2, 7.2),
            Vec3::new(3.4, 8.7, 6.2),
            Vec3::new(5.8, 5.7, 7.0),
        ][index % 3],
        target: Vec3::new(0.40, 0.0, 0.0),
        roll: [-18.0, -25.0, -12.0][index % 3],
    };
    s.camera.focal_mm = [44.0, 43.0, 40.0][index % 3];
    s.camera.f_number = 0.0;
    s.camera.focus_distance = 10.0;
    s.material.albedo = [0.72, 0.46, 0.19];
    s.material.roughness = 0.27;
    s.material.metallic = 0.74;
    s.material.specular = 0.05;
    s.material.coloring = Coloring::None;
    s.material.texture = None;
    s.material.surface = None;
    s.material.finishes = vec![Finish {
        name: "Or des orbites".into(),
        albedo: [0.9, 0.55, 0.16],
        roughness: 0.20,
        metallic: 0.86,
        specular: 0.05,
        emission: [0.55, 0.19, 0.018],
        detail: None,
        optics: None,
    }];
    s.material.optics = None;
    s.lights = vec![
        Light::Sun {
            direction: Vec3::new(-0.4, 0.9, 0.6),
            color: Color::Kelvin(5200.0),
            intensity: 4.3,
            angular_size: 3.5,
        },
        Light::Sun {
            direction: Vec3::new(0.8, 0.2, -0.5),
            color: Color::Linear([0.07, 0.38, 1.0]),
            intensity: 3.0,
            angular_size: 7.0,
        },
        Light::Tube {
            start: Vec3::new(0.0, -0.018, 0.0),
            end: Vec3::new(0.0, 0.018, 0.0),
            radius: 0.115,
            color: Color::Kelvin(6300.0),
            intensity: 28.0,
        },
        Light::Environment {
            zenith: Color::Linear([0.00002, 0.00005, 0.00015]),
            horizon: Color::Linear([0.0001, 0.00015, 0.0003]),
            ground: Color::Linear([0.00003, 0.00002, 0.00004]),
            intensity: 0.2,
            rotation: 0.0,
        },
    ];
    s.atmosphere = Atmosphere::none();
    s.atmosphere.nebula = Some(Nebula {
        particles: None,
        center: Vec3::new(-5.0, -9.0, -12.0),
        half_size: Vec3::new(26.0, 6.2, 3.5),
        angle: 25.0,
        density: 0.065,
        emission: 0.60,
        dust: 2.5,
        scattering: 0.15,
        anisotropy: 0.35,
        turbulence: 1.4,
        detail: 2.3,
        cool_color: [0.012, 0.08, 0.20],
        warm_color: [0.65, 0.15, 0.025],
        core_position: Vec3::new(-7.0, -0.4, 0.0),
        core_radius: 1.3,
        core_energy: 3.5,
        stars: 0.16,
        seed: 20260,
        steps: 24,
    });
    s.phenomena = Some(phenomena());
    s.look = Look::neutral();
    s.look.exposure_ev = 0.35;
    s.look.tonemap = Tonemap::Agx {
        slope: 1.0,
        power: 1.10,
        offset: 0.0,
    };
    s.look.saturation = 1.16;
    s.look.vibrance = 0.12;
    s.look.neon_softness = 0.12;
    s.look.vignette = Some(Vignette {
        radius: 0.90,
        softness: 0.65,
        intensity: 0.23,
        elliptical: true,
    });
    s.validate()?;
    Ok(s)
}
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("study");
    let out = PathBuf::from(OUTPUT);
    std::fs::create_dir_all(&out)?;
    if mode == "scene" {
        let index: usize = args.get(2).map(String::as_str).unwrap_or("0").parse()?;
        let s = scene(index)?;
        s.save("scenes/showcase/la-forge-des-mondes.ron")?;
        s.save(out.join("la-forge-des-mondes.ron"))?;
        return Ok(());
    }
    let renderer = Renderer::new(Arc::new(GpuContext::new()?))?;
    println!("GPU: {:?}", renderer.context().info);
    for index in 0..1 {
        let s = scene(index)?;
        s.save(out.join(format!("study-c{index}.ron")))?;
        let start = Instant::now();
        let mut cfg = RenderConfig::new(640, 360, 4);
        cfg.integrator = ComputeIntegrator::Wavefront;
        cfg.max_bounces = 2;
        cfg.max_steps = 256;
        cfg.tile_size = 64;
        let mut previous = Instant::now();
        println!("Study {index}: preparing native render");
        let image = renderer.render(&s, &cfg, &mut |progress| {
            if previous.elapsed().as_secs() >= 15 {
                println!(
                    "Study {index}: {} dispatches, max {}ms",
                    progress.dispatches, progress.max_dispatch_ms
                );
                previous = Instant::now();
            }
        })?;
        ensure!(image.samples_min == 4, "incomplete study");
        ensure!(
            image.color.data.iter().flatten().all(|v| v.is_finite()),
            "nonfinite artwork"
        );
        let encoded = develop_and_encode(&image.color, &s.look, 8);
        write_png(
            out.join(format!("study-c{index}.png")),
            &encoded,
            OutputSpace::Srgb,
            Some(&s.to_ron()),
        )?;
        write_exr(out.join(format!("study-c{index}.exr")), &image.color)?;
        println!(
            "Study {index} completed in {:.1}s",
            start.elapsed().as_secs_f32()
        );
    }
    Ok(())
}
