//! A native artwork built entirely in the canonical creation graph.
//! study [directory], final [style 0..3] [directory], or render SCENE OUTPUT W H SPP.
use anyhow::{ensure, Result};
use std::{path::PathBuf, sync::Arc, time::Instant};
use syfrack_core::{scene::phenomena::*, *};
use syfrack_render::{GpuContext, RenderConfig, Renderer};

fn add(g: &mut CreationGraph, kind: CreationKind, name: &str, x: f32, y: f32) -> u32 {
    let id = g.add(kind, [x, y]);
    g.nodes.iter_mut().find(|n| n.id == id).unwrap().name = name.into();
    id
}
fn field(g: &mut CreationGraph, op: FieldOp, name: &str, x: f32, y: f32) -> u32 {
    add(g, CreationKind::Field(op), name, x, y)
}
fn wire(g: &mut CreationGraph, a: (u32, u32), b: (u32, u32)) {
    g.connect(a, b).unwrap();
}
fn scalar(g: &mut CreationGraph, v: f32, name: &str, x: f32, y: f32) -> u32 {
    field(g, FieldOp::Scalar { value: v }, name, x, y)
}
fn palette(stops: Vec<[f32; 3]>) -> Ramp {
    Ramp {
        stops,
        cyclic: true,
        interpolation: PaletteInterpolation::Smooth,
        ..Default::default()
    }
}

fn scene(style: u32, final_quality: bool) -> Result<Scene> {
    let reference =
        Scene::load("scenes/studies/duet-20261014/flame-refinements/incandescence-06.ron")?;
    let mut flame = reference.canvas.flame.clone().expect("native orbit source");
    flame.palette = palette(vec![
        [0.008, 0.018, 0.055],
        [0.035, 0.19, 0.26],
        [0.08, 0.58, 0.64],
        [0.67, 0.95, 0.87],
        [1.0, 0.73, 0.31],
        [0.93, 0.25, 0.055],
        [0.055, 0.035, 0.12],
    ]);
    flame.palette.cyclic = false;
    flame.background = [0.0; 3];
    flame.brightness = 1.15;
    flame.density_log = 4.0;
    flame.filter_radius = 0.85;
    if style == 1 || style == 3 {
        flame.transforms[2].terms[0].variation = FlameVariation::Julian {
            power: 3,
            distance: 1.0,
        };
        flame.transforms[2].weight = 0.48;
        if let Some(t) = &mut flame.final_transform {
            t.terms[0].variation = FlameVariation::Curl {
                c1: 0.15,
                c2: 0.018,
            };
        }
    }
    if style >= 2 {
        flame.transforms[1].terms[1].variation = FlameVariation::Bubble;
        flame.palette.stops.rotate_left(2);
        flame.transforms[1].terms[0].variation = FlameVariation::Curl {
            c1: 0.31,
            c2: 0.055,
        };
    }
    let mut s = Scene::example();
    s.dimension = Dimension::TwoD;
    s.seed = 20261020;
    s.canvas.center = [-0.1, 0.05];
    s.canvas.zoom = if style == 3 { 1.32 } else { 1.15 };
    s.canvas.rotation = if style == 2 { 18.0 } else { -8.0 };
    s.look.tonemap = Tonemap::Agx {
        slope: 1.0,
        power: 1.12,
        offset: 0.0,
    };
    s.look.exposure_ev = -0.55;
    s.look.auto_exposure = None;
    s.look.saturation = 1.18;
    s.look.vibrance = 0.08;
    s.look.neon_softness = 0.18;
    s.look.grain = None;
    s.look.vignette = Some(Vignette {
        radius: 0.78,
        softness: 0.8,
        intensity: 0.19,
        elliptical: true,
    });
    let mut g = CreationGraph::from_scene(&s);
    let out = g
        .nodes
        .iter()
        .find(|n| matches!(n.kind, CreationKind::Output))
        .unwrap()
        .id;
    {
        let n = g.nodes.iter_mut().find(|n| n.id == out).unwrap();
        n.name = "L’Océan des possibles".into();
        n.pos = [2740.0, 380.0];
    }
    g.nodes
        .retain(|n| !matches!(n.kind, CreationKind::Field(_)));
    let ids: Vec<_> = g.nodes.iter().map(|n| n.id).collect();
    g.links
        .retain(|l| ids.contains(&l.from.0) && ids.contains(&l.to.0));
    g.settings.domain = FieldDomain {
        center: [0.0; 3].into(),
        half: [2.8, 2.8, 1.0].into(),
        resolution: if final_quality {
            [192, 192, 1]
        } else {
            [96, 96, 1]
        },
        memory_budget_mib: 512,
    };
    let p = field(
        &mut g,
        FieldOp::Position,
        "Le même espace pour trois dynamiques",
        0.0,
        320.0,
    );
    let f = add(
        &mut g,
        CreationKind::Flame(FlameField {
            flame,
            center: [0.0; 2],
            half: [3.0; 2],
            angle: 0.0,
            resolution: if final_quality { 1536 } else { 1024 },
            points: 16_777_216,
            seed: 0xc0515617,
            ..Default::default()
        }),
        "Orbites : courants stellaires",
        300.0,
        300.0,
    );
    let mut tex = FractalTexture::default();
    tex.planar = Some(PlanarFractal {
        family: PlanarFamily::Julia,
        julia: if style >= 2 {
            [-0.701, 0.252]
        } else {
            [-0.74543, 0.11301]
        },
        zoom: 2.0,
        angle: 24.0,
        iterations: 224,
        stripes: 17.0,
        ..Default::default()
    });
    tex.scale = 0.85;
    tex.octaves = 2;
    tex.contrast = 1.35;
    tex.rotation = [0.0; 3].into();
    tex.offset = [0.0; 3].into();
    tex.channel = TextureChannel::Orbit {
        channels: OrbitColoring {
            traps: [0.36, 0.17, 0.22, 0.0],
            iterations: 0.16,
            normal: 0.0,
            logarithmic: 3.0,
        },
    };
    tex.ramp = palette(vec![
        [0.22, 0.32, 0.48],
        [0.92, 0.50, 0.14],
        [0.04, 0.28, 0.34],
        [1.0, 0.92, 0.68],
        [0.06, 0.045, 0.16],
    ]);
    tex.ramp.frequency = 2.6;
    let ornament = tex.ramp.clone();
    let j = add(
        &mut g,
        CreationKind::Texture(Box::new(tex)),
        "Julia : gravure des échelles",
        300.0,
        660.0,
    );
    let jp = add(
        &mut g,
        CreationKind::Palette(ornament),
        "Encres : or, nacre et pétrole",
        620.0,
        660.0,
    );
    wire(&mut g, (p, 0), (f, 0));
    wire(&mut g, (p, 0), (j, 0));
    wire(&mut g, (j, 0), (jp, 0));

    // Both fractal processes sculpt the Hamiltonian of the wave evolution.
    let potential = field(
        &mut g,
        FieldOp::Multiply,
        "Potentiel : Julia × densité d’orbites",
        620.0,
        1020.0,
    );
    let bounded = field(
        &mut g,
        FieldOp::Clamp { min: 0.0, max: 2.0 },
        "Potentiel borné : stabilité physique",
        920.0,
        1020.0,
    );
    wire(&mut g, (j, 0), (potential, 0));
    wire(&mut g, (f, 1), (potential, 1));
    wire(&mut g, (potential, 0), (bounded, 0));
    let sim = add(
        &mut g,
        CreationKind::Simulation(Simulation {
            name: "Deux vortex en contre-rotation".into(),
            model: SimulationModel::GrossPitaevskii {
                dispersion: 0.009,
                interaction: 0.7,
                damping: 0.0,
            },
            steps: 96,
            dt: 0.003,
            seed: 7351,
            seeds: vec![
                SeedBlob {
                    center: [-0.62, 0.1, 0.0].into(),
                    radius: 1.0,
                    amplitude: 1.0,
                    phase: 0.0,
                    winding: 1,
                },
                SeedBlob {
                    center: [0.62, -0.18, 0.0].into(),
                    radius: 0.88,
                    amplitude: 0.82,
                    phase: 1.9,
                    winding: -2,
                },
            ],
            ..Default::default()
        }),
        "Condensat : émergence et interférence",
        1220.0,
        1020.0,
    );
    wire(&mut g, (p, 0), (sim, 0));
    wire(&mut g, (bounded, 0), (sim, 1));
    // Reconstruct phase from interpolated complex amplitudes, avoiding an
    // incorrect linear interpolation across the scalar phase branch cut.
    let complex = field(
        &mut g,
        FieldOp::ComplexFromParts,
        "ψ = Re(ψ) + i Im(ψ)",
        1520.0,
        1020.0,
    );
    let phase = field(
        &mut g,
        FieldOp::Phase,
        "Phase : le temps devient couleur",
        1820.0,
        1020.0,
    );
    let intensity = field(
        &mut g,
        FieldOp::Intensity,
        "Probabilité |ψ|²",
        1520.0,
        1320.0,
    );
    let envelope = field(
        &mut g,
        FieldOp::Power { exponent: 0.5 },
        "Amplitude cohérente |ψ|",
        1820.0,
        1320.0,
    );
    let qp = add(
        &mut g,
        CreationKind::Palette(palette(vec![
            [0.035, 0.12, 0.3],
            [0.12, 0.8, 0.83],
            [0.85, 0.96, 0.84],
            [1.0, 0.58, 0.16],
            [0.25, 0.055, 0.22],
            [0.12, 0.25, 0.55],
        ])),
        "Chromatisme des phases",
        2120.0,
        1020.0,
    );
    for (a, b) in [
        ((sim, 0), (complex, 0)),
        ((sim, 1), (complex, 1)),
        ((complex, 0), (phase, 0)),
        ((complex, 0), (intensity, 0)),
        ((intensity, 0), (envelope, 0)),
        ((phase, 0), (qp, 0)),
    ] {
        wire(&mut g, a, b);
    }

    let emboss = field(&mut g, FieldOp::Scale, "Ciselure de Julia", 920.0, 660.0);
    let emboss_strength = scalar(&mut g, 0.7, "Force de la gravure", 620.0, 840.0);
    let base = field(
        &mut g,
        FieldOp::Vector {
            value: [0.60; 3].into(),
        },
        "Continuité du courant",
        920.0,
        480.0,
    );
    let tint = field(
        &mut g,
        FieldOp::Add,
        "Transmission des encres",
        1220.0,
        480.0,
    );
    let floor = scalar(&mut g, 0.12, "Le seuil du silence", 300.0, 140.0);
    let subtract = field(
        &mut g,
        FieldOp::Subtract,
        "Isoler les courants",
        620.0,
        140.0,
    );
    let positive = field(
        &mut g,
        FieldOp::Clamp { min: 0.0, max: 8.0 },
        "Écarter la poussière diffuse",
        920.0,
        140.0,
    );
    let mask = field(
        &mut g,
        FieldOp::Power { exponent: 0.85 },
        "Hiérarchie des densités",
        1220.0,
        140.0,
    );
    let filaments = field(
        &mut g,
        FieldOp::Scale,
        "Le trait naît de la densité",
        620.0,
        320.0,
    );
    for (a, b) in [
        ((f, 1), (subtract, 0)),
        ((floor, 0), (subtract, 1)),
        ((subtract, 0), (positive, 0)),
        ((positive, 0), (mask, 0)),
        ((f, 0), (filaments, 0)),
        ((mask, 0), (filaments, 1)),
    ] {
        wire(&mut g, a, b);
    }
    let etched = field(
        &mut g,
        FieldOp::Multiply,
        "Le macrocosme porte le détail",
        1520.0,
        340.0,
    );
    for (a, b) in [
        ((jp, 0), (emboss, 0)),
        ((emboss_strength, 0), (emboss, 1)),
        ((base, 0), (tint, 0)),
        ((emboss, 0), (tint, 1)),
        ((filaments, 0), (etched, 0)),
        ((tint, 0), (etched, 1)),
    ] {
        wire(&mut g, a, b);
    }
    let quantum = field(
        &mut g,
        FieldOp::Multiply,
        "Phase dans les filaments",
        2120.0,
        680.0,
    );
    let coherent = field(
        &mut g,
        FieldOp::Scale,
        "Lueur quantique cohérente",
        2420.0,
        680.0,
    );
    let light = field(
        &mut g,
        FieldOp::Add,
        "Un seul champ de lumière",
        2420.0,
        340.0,
    );
    let night = field(
        &mut g,
        FieldOp::Vector {
            value: [0.0005, 0.0011, 0.0038].into(),
        },
        "Le silence entre les mondes",
        2120.0,
        140.0,
    );
    let final_light = field(
        &mut g,
        FieldOp::Add,
        "De la probabilité au cosmos",
        2740.0,
        140.0,
    );
    for (a, b) in [
        ((etched, 0), (quantum, 0)),
        ((qp, 0), (quantum, 1)),
        ((quantum, 0), (coherent, 0)),
        ((envelope, 0), (coherent, 1)),
        ((etched, 0), (light, 0)),
        ((coherent, 0), (light, 1)),
        ((light, 0), (final_light, 0)),
        ((night, 0), (final_light, 1)),
        ((final_light, 0), (out, 1)),
    ] {
        wire(&mut g, a, b);
    }
    s.creation = Some(g);
    s.unify()?;
    Ok(s)
}

fn render(
    r: &Renderer,
    s: &Scene,
    path: &std::path::Path,
    w: u32,
    h: u32,
    spp: u32,
    exr: bool,
) -> Result<()> {
    let start = Instant::now();
    s.save(path.with_extension("ron"))?;
    eprintln!("{}: prepare GPU sources", path.display());
    let prepared = r.prepare_phenomena(s)?.unwrap();
    for d in &prepared.diagnostics {
        eprintln!("wave mass {} -> {}", d.mass_before, d.mass_after);
    }
    let mut cfg = RenderConfig::new(w, h, spp);
    cfg.max_bounces = 1;
    cfg.max_steps = 64;
    cfg.tile_size = 64;
    cfg.samples_per_dispatch = 1;
    cfg.denoise = false;
    std::fs::write(
        path.with_extension("render-config.ron"),
        ron::ser::to_string_pretty(&cfg, ron::ser::PrettyConfig::new())?,
    )?;
    eprintln!("native raw render: denoise=false; develop_and_encode only");
    let mut last = 0;
    let image = r.render(s, &cfg, &mut |p| {
        if p.samples_min != last {
            last = p.samples_min;
            eprintln!("{} {last}/{spp}", path.display());
        }
    })?;
    ensure!(image.samples_min == spp, "unfinished artwork");
    ensure!(
        image.color.data.iter().flatten().all(|v| v.is_finite()),
        "nonfinite artwork"
    );
    let text = s.to_ron();
    let encoded =
        syfrack_color::develop_and_encode(&image.color, &s.look, if exr { 16 } else { 8 });
    syfrack_color::write_png(
        path.with_extension("png"),
        &encoded,
        OutputSpace::Srgb,
        Some(&text),
    )?;
    if exr {
        syfrack_color::write_exr(path.with_extension("exr"), &image.color)?;
    }
    eprintln!(
        "{} complete: {:.2}s",
        path.display(),
        start.elapsed().as_secs_f64()
    );
    Ok(())
}

// A native revision: no image-space sharpening, external raster edit, coordinate
// change or modification of an orbit law. A frozen original quantum state keeps
// its exact samples while its downstream appearance is art-directed.
fn lock_original_state(r: &Renderer, source: &Scene) -> Result<Scene> {
    let prepared = r.prepare_phenomena(source)?.expect("creation sources");
    let mut locked = source.clone();
    let g = locked.creation.as_mut().unwrap();
    let (simulations, _) = g.resource_nodes()?;
    ensure!(simulations.len() == prepared.snapshots.len());
    for (id, grid) in simulations.into_iter().zip(&prepared.snapshots) {
        if let CreationKind::Simulation(sim) =
            &mut g.nodes.iter_mut().find(|n| n.id == id).unwrap().kind
        {
            sim.frozen = Some(FrozenSnapshot {
                resolution: grid.domain.resolution,
                values: grid.data.iter().map(|v| v.to_array()).collect(),
            });
        }
    }
    locked.unify()?;
    Ok(locked)
}

fn revision(source: &Scene, style: u32) -> Result<Scene> {
    let mut s = source.clone();
    let g = s.creation.as_mut().unwrap();
    g.settings.domain.memory_budget_mib = 1024;
    for n in &mut g.nodes {
        match &mut n.kind {
            CreationKind::Flame(f) => {
                f.resolution = 2048;
                f.points = 50_331_648;
                f.flame.filter_radius = 0.58;
                f.frozen = None;
                if style > 0 {
                    f.flame.palette.stops = match style {
                        1 => vec![
                            [0.003, 0.009, 0.038],
                            [0.014, 0.075, 0.19],
                            [0.025, 0.27, 0.35],
                            [0.17, 0.61, 0.59],
                            [0.84, 0.65, 0.32],
                            [0.72, 0.19, 0.035],
                            [0.055, 0.012, 0.07],
                        ],
                        2 => vec![
                            [0.003, 0.006, 0.028],
                            [0.025, 0.07, 0.25],
                            [0.018, 0.37, 0.48],
                            [0.33, 0.79, 0.74],
                            [0.93, 0.78, 0.49],
                            [0.78, 0.28, 0.075],
                            [0.032, 0.009, 0.075],
                        ],
                        _ => vec![
                            [0.006, 0.004, 0.027],
                            [0.055, 0.025, 0.14],
                            [0.16, 0.035, 0.18],
                            [0.57, 0.16, 0.20],
                            [0.96, 0.57, 0.26],
                            [0.82, 0.76, 0.61],
                            [0.018, 0.025, 0.11],
                        ],
                    };
                }
            }
            CreationKind::Palette(p) if style > 0 => {
                if n.name == "Chromatisme des phases" {
                    // A glaze, rather than a second full-strength white layer.
                    p.stops = vec![
                        [0.01, 0.035, 0.10],
                        [0.025, 0.20, 0.24],
                        [0.24, 0.36, 0.31],
                        [0.42, 0.22, 0.055],
                        [0.10, 0.02, 0.09],
                        [0.02, 0.055, 0.16],
                    ];
                } else if n.name == "Encres : or, nacre et pétrole" {
                    p.stops = vec![
                        [0.10, 0.18, 0.30],
                        [0.70, 0.37, 0.11],
                        [0.025, 0.20, 0.24],
                        [0.72, 0.67, 0.42],
                        [0.035, 0.02, 0.095],
                    ];
                }
            }
            CreationKind::Field(FieldOp::Scalar { value })
                if style > 0 && n.name == "Force de la gravure" =>
            {
                *value = 0.28
            }
            CreationKind::Field(FieldOp::Vector { value })
                if style > 0 && n.name == "Continuité du courant" =>
            {
                *value = [0.72, 0.72, 0.72].into()
            }
            CreationKind::Field(FieldOp::Vector { value })
                if style > 0 && n.name == "Le feu avant l’océan" =>
            {
                *value = [1.4, 0.68, 0.30].into()
            }
            _ => {}
        }
    }
    if style > 0 {
        s.look.exposure_ev = match style {
            1 => -0.42,
            2 => -0.62,
            _ => -0.45,
        };
        s.look.tonemap = Tonemap::Agx {
            slope: 1.0,
            power: 1.19,
            offset: 0.0,
        };
        s.look.saturation = 1.18;
        s.look.vibrance = 0.10;
        s.look.neon_softness = 0.06;
    }
    s.unify()?;
    verify_revision(source, &s)?;
    Ok(s)
}

fn verify_revision(source: &Scene, revised: &Scene) -> Result<()> {
    ensure!(source.fractal == revised.fractal, "solid geometry changed");
    ensure!(source.seed == revised.seed && source.dimension == revised.dimension);
    ensure!(source.canvas == revised.canvas, "camera or framing changed");
    let a = source.creation.as_ref().unwrap();
    let b = revised.creation.as_ref().unwrap();
    ensure!(
        a.links == b.links && a.nodes.len() == b.nodes.len(),
        "graph topology changed"
    );
    ensure!(
        a.settings.domain.center == b.settings.domain.center
            && a.settings.domain.half == b.settings.domain.half
            && a.settings.domain.resolution == b.settings.domain.resolution,
        "simulation domain changed"
    );
    for original in &a.nodes {
        let revised = b.node(original.id).unwrap();
        ensure!(original.enabled == revised.enabled);
        match (&original.kind, &revised.kind) {
            (CreationKind::Flame(a), CreationKind::Flame(b)) => {
                ensure!(
                    a.flame.transforms == b.flame.transforms
                        && a.flame.final_transform == b.flame.final_transform,
                    "orbit law changed"
                );
                ensure!(
                    a.seed == b.seed
                        && a.center == b.center
                        && a.half == b.half
                        && a.angle == b.angle,
                    "orbit seed or source coordinates changed"
                );
                ensure!(
                    a.flame.density_log == b.flame.density_log
                        && a.flame.brightness == b.flame.brightness
                );
            }
            (CreationKind::Palette(_), CreationKind::Palette(_)) => {}
            _ if [
                "Force de la gravure",
                "Continuité du courant",
                "Le feu avant l’océan",
            ]
            .contains(&original.name.as_str()) => {}
            _ => ensure!(
                original.kind == revised.kind,
                "non-appearance node {} changed",
                original.name
            ),
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let mode = args.first().map(String::as_str).unwrap_or("study");
    let r = Renderer::new(Arc::new(GpuContext::new()?))?;
    eprintln!("adapter {:?}", r.context().info);
    if mode == "native-study" {
        let source = Scene::load(&args[1])?;
        let dir = PathBuf::from(&args[2]);
        std::fs::create_dir_all(&dir)?;
        let locked = lock_original_state(&r, &source)?;
        locked.save(dir.join("etat-original-verrouille.ron"))?;
        for style in 1..=3 {
            render(
                &r,
                &revision(&locked, style)?,
                &dir.join(format!("direction-{style}")),
                1280,
                720,
                4,
                false,
            )?;
        }
        render(
            &r,
            &revision(&locked, 0)?,
            &dir.join("pique-natif-reference"),
            2560,
            1440,
            24,
            true,
        )?;
        return Ok(());
    }
    if mode == "native-final" {
        let source = Scene::load(&args[1])?;
        let s = Scene::load(&args[2])?;
        verify_revision(&source, &s)?;
        return render(&r, &s, &PathBuf::from(&args[3]), 2560, 1440, 32, true);
    }
    if mode == "render" {
        let s = Scene::load(&args[1])?;
        return render(
            &r,
            &s,
            &PathBuf::from(&args[2]),
            args[3].parse()?,
            args[4].parse()?,
            args[5].parse()?,
            true,
        );
    }
    if mode == "frames" {
        let source = Scene::load(&args[1])?;
        let dir = PathBuf::from(&args[2]);
        std::fs::create_dir_all(&dir)?;
        for (i, (angle, zoom, exposure)) in
            [(15.0, 1.30, -0.65), (38.0, 1.42, -1.0), (48.0, 1.35, -1.30)]
                .into_iter()
                .enumerate()
        {
            let mut s = source.clone();
            s.canvas.rotation = angle;
            s.canvas.zoom = zoom;
            s.canvas.center = [-0.1, 0.18];
            s.look.exposure_ev = exposure;
            render(
                &r,
                &s,
                &dir.join(format!("cadrage-{i}")),
                800,
                450,
                4,
                false,
            )?;
        }
        return Ok(());
    }
    if mode == "duet" {
        let mut s = Scene::load(&args[1])?;
        let g = s.creation.as_mut().unwrap();
        let p = g
            .nodes
            .iter()
            .find(|n| matches!(n.kind, CreationKind::Field(FieldOp::Position)))
            .unwrap()
            .id;
        let f = g
            .nodes
            .iter()
            .find(|n| matches!(n.kind, CreationKind::Flame(_)))
            .unwrap()
            .id;
        let cfg = if let CreationKind::Flame(f) = &g.node(f).unwrap().kind {
            f.clone()
        } else {
            unreachable!()
        };
        let shift = field(
            g,
            FieldOp::Vector {
                value: [-0.85, -0.1, 0.0].into(),
            },
            "Centre du macrocosme",
            0.0,
            1700.0,
        );
        let large_pos = field(g, FieldOp::Add, "Déployer le monde", 300.0, 1700.0);
        wire(g, (p, 0), (large_pos, 0));
        wire(g, (shift, 0), (large_pos, 1));
        wire(g, (large_pos, 0), (f, 0));
        let scale = field(
            g,
            FieldOp::Vector {
                value: [-1.85, 1.85, 1.85].into(),
            },
            "L’autre échelle · réflexion",
            0.0,
            2000.0,
        );
        let offset = field(
            g,
            FieldOp::Vector {
                value: [-3.2, -0.12, 0.0].into(),
            },
            "Centre du microcosme",
            300.0,
            2180.0,
        );
        let scaled = field(
            g,
            FieldOp::Multiply,
            "Une même loi à une autre échelle",
            300.0,
            2000.0,
        );
        let small_pos = field(g, FieldOp::Add, "Espace du germe", 620.0, 2000.0);
        let small = add(
            g,
            CreationKind::Flame(cfg),
            "Microcosme · les mêmes orbites",
            920.0,
            2000.0,
        );
        for (a, b) in [
            ((p, 0), (scaled, 0)),
            ((scale, 0), (scaled, 1)),
            ((scaled, 0), (small_pos, 0)),
            ((offset, 0), (small_pos, 1)),
            ((small_pos, 0), (small, 0)),
        ] {
            wire(g, a, b);
        }
        let sum = field(
            g,
            FieldOp::Add,
            "Les deux échelles sculptent le potentiel",
            620.0,
            1700.0,
        );
        let potential = g
            .nodes
            .iter()
            .find(|n| n.name == "Potentiel : Julia × densité d’orbites")
            .unwrap()
            .id;
        wire(g, (f, 1), (sum, 0));
        wire(g, (small, 1), (sum, 1));
        wire(g, (sum, 0), (potential, 1));
        let floor = scalar(g, 0.12, "Seuil du germe", 920.0, 2180.0);
        let sub = field(g, FieldOp::Subtract, "Contraste du germe", 1220.0, 2000.0);
        let mask = field(
            g,
            FieldOp::Clamp { min: 0.0, max: 8.0 },
            "Densité du germe",
            1520.0,
            2000.0,
        );
        let warm = field(
            g,
            FieldOp::Vector {
                value: [1.3, 0.62, 0.28].into(),
            },
            "Le feu avant l’océan",
            1220.0,
            2180.0,
        );
        let rgb = field(g, FieldOp::Multiply, "Or du microcosme", 1820.0, 2000.0);
        let light = field(g, FieldOp::Scale, "Émergence du germe", 2120.0, 2000.0);
        for (a, b) in [
            ((small, 1), (sub, 0)),
            ((floor, 0), (sub, 1)),
            ((sub, 0), (mask, 0)),
            ((small, 0), (rgb, 0)),
            ((warm, 0), (rgb, 1)),
            ((rgb, 0), (light, 0)),
            ((mask, 0), (light, 1)),
        ] {
            wire(g, a, b);
        }
        let out = g
            .nodes
            .iter()
            .find(|n| matches!(n.kind, CreationKind::Output))
            .unwrap()
            .id;
        let previous = g.input(out, 1).unwrap();
        let both = field(g, FieldOp::Add, "Le germe et le monde", 2440.0, 2000.0);
        wire(g, previous, (both, 0));
        wire(g, (light, 0), (both, 1));
        wire(g, (both, 0), (out, 1));
        for n in &mut g.nodes {
            if let CreationKind::Simulation(sim) = &mut n.kind {
                sim.seeds[0].center = [-1.72, 0.0, 0.0].into();
                sim.seeds[0].radius = 0.8;
                sim.seeds[1].center = [0.85, 0.1, 0.0].into();
                sim.seeds[1].radius = 1.2;
            }
        }
        g.settings.domain.half = [3.6, 2.8, 1.0].into();
        s.canvas.rotation = 0.0;
        s.canvas.center = [-0.1, 0.16];
        s.canvas.zoom = 1.15;
        s.look.exposure_ev = -0.8;
        s.unify()?;
        let path = PathBuf::from(&args[2]);
        render(&r, &s, &path, 1000, 562, 6, false)?;
        return Ok(());
    }
    if mode == "finish" {
        let mut s = Scene::load(&args[1])?;
        let g = s.creation.as_mut().unwrap();
        for n in &mut g.nodes {
            match &mut n.kind {
                CreationKind::Flame(f) => {
                    f.resolution = 1536;
                    f.points = 16_777_216;
                    f.frozen = None;
                }
                CreationKind::Simulation(sim) => sim.frozen = None,
                _ => {}
            }
        }
        g.settings.domain.resolution = [256, 192, 1];
        s.canvas.zoom = 1.05;
        s.canvas.center = [-0.08, 0.27];
        s.look.exposure_ev = -0.85;
        s.unify()?;
        let path = PathBuf::from(&args[2]);
        render(&r, &s, &path, 2560, 1440, 16, true)?;
        return Ok(());
    }
    let dir = PathBuf::from(
        if mode == "final" {
            args.get(2)
        } else {
            args.get(1)
        }
        .map(String::as_str)
        .unwrap_or("renders/l-ocean-des-possibles-20261020"),
    );
    std::fs::create_dir_all(&dir)?;
    if mode == "final" {
        let style = args.get(1).map_or(Ok(1), |s| s.parse::<u32>())?;
        render(
            &r,
            &scene(style, true)?,
            &dir.join("l-ocean-des-possibles"),
            2560,
            1440,
            16,
            true,
        )?;
    } else {
        for style in 0..4 {
            render(
                &r,
                &scene(style, false)?,
                &dir.join(format!("etude-{style}")),
                800,
                450,
                4,
                false,
            )?;
        }
    }
    Ok(())
}
