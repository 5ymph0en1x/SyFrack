//! A native 2D orbit engraving and its genuine 3D invariant-plane lift.
//! dimension_duet study|final|curate INDEX OUTPUT [width height samples]
use anyhow::{ensure, Context, Result};
use glam::{Vec2, Vec3};
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use syfrack_color::{develop_and_encode, write_exr, write_png};
use syfrack_core::*;
use syfrack_render::{
    control::ControlSession, ExportMode, ExportSession, GpuContext, RenderConfig, Renderer,
};

fn node(g: &mut Graph, kind: NodeKind, name: &str) -> u32 {
    let id = g.nodes.len() as u32;
    g.nodes.push(Node {
        id,
        name: name.into(),
        kind,
        pos: [240.0 * (id % 6) as f32, 160.0 * (id / 6) as f32],
        enabled: true,
    });
    id
}
fn link(g: &mut Graph, a: u32, b: u32, port: u32) {
    g.links.push(Link {
        from: (a, 0),
        to: (b, port),
    });
}
fn chain(g: &mut Graph, a: u32, k: NodeKind, name: &str) -> u32 {
    let b = node(g, k, name);
    link(g, a, b, 0);
    b
}

#[derive(Clone, Copy)]
struct Formula {
    c: Vec2,
    iterations: u32,
}
fn formula(i: usize) -> Formula {
    let c = [
        [0.0, 1.0],
        [0.03, 0.98],
        [0.10, 0.90],
        [0.16, 0.87],
        [0.03, 1.03],
        [-0.06, 0.82],
    ][i % 6];
    Formula {
        c: Vec2::from_array(c),
        iterations: 32,
    }
}
fn graph(f: Formula) -> Graph {
    let mut g = Graph::new_minimal();
    g.nodes.clear();
    g.links.clear();
    g.safety = 0.65;
    g.bounding_radius_hint = 3.0;
    let input = node(&mut g, NodeKind::Input, "Espace commun 2D / 3D");
    let power = node(
        &mut g,
        NodeKind::Power { power: 3.0 },
        "Puissance polaire cubique — prolongement de w³",
    );
    let constant = node(
        &mut g,
        NodeKind::AddConstant {
            multiplier: Vec3::ZERO,
            julia: Vec3::new(f.c.y, 0.0, f.c.x),
        },
        "Julia : c = Re(c) + i Im(c), plongé en (Im(c),0,Re(c))",
    );
    let trap0 = node(
        &mut g,
        NodeKind::Trap {
            slot: 0,
            measure: Measure::Norm,
            scale: 2.0,
            mode: TrapMode::Minimum,
        },
        "Mémoire des rayons orbitaux",
    );
    let trap1 = node(
        &mut g,
        NodeKind::Trap {
            slot: 1,
            measure: Measure::Norm,
            scale: 1.5,
            mode: TrapMode::Maximum,
        },
        "Rayon maximal de l’orbite",
    );
    let trap2 = node(
        &mut g,
        NodeKind::Trap {
            slot: 2,
            measure: Measure::AngleXZ,
            scale: 1.0,
            mode: TrapMode::Last,
        },
        "Angle final de l’orbite",
    );
    let iterate = chain(
        &mut g,
        input,
        NodeKind::Iterate {
            iterations: f.iterations,
            bailout: 16.0,
            body: vec![power, constant, trap0, trap1, trap2],
        },
        "32 générations identiques dans les deux dimensions",
    );
    let field = chain(
        &mut g,
        iterate,
        NodeKind::NormFence { inset: 1.4 },
        "Préimage de la boule extérieure de rayon 1,4",
    );
    let bound = chain(
        &mut g,
        input,
        NodeKind::Sphere { radius: 1.6 },
        "Domaine sphérique de composition",
    );
    let intersection = node(
        &mut g,
        NodeKind::Intersection,
        "Intersection dimensionnelle",
    );
    link(&mut g, field, intersection, 0);
    link(&mut g, bound, intersection, 1);
    chain(
        &mut g,
        intersection,
        NodeKind::Output,
        "Chrysalide — empreinte et volume",
    );
    g
}
fn palette() -> Ramp {
    Ramp {
        stops: vec![
            [0.001, 0.003, 0.008],
            [0.005, 0.035, 0.055],
            [0.025, 0.21, 0.24],
            [0.16, 0.57, 0.52],
            [0.83, 0.82, 0.57],
            [0.95, 0.44, 0.095],
            [0.29, 0.07, 0.015],
            [0.003, 0.008, 0.016],
        ],
        cyclic: true,
        frequency: 7.0,
        phase: 0.02,
        interpolation: PaletteInterpolation::Smooth,
        ..Ramp::default()
    }
}
fn channels() -> OrbitColoring {
    OrbitColoring {
        traps: [0.78, 0.16, 0.06, 0.0],
        iterations: 0.0,
        normal: 0.0,
        logarithmic: 0.0,
    }
}

fn scenes(i: usize) -> (Scene, Scene, Graph) {
    let g = graph(formula(i));
    let mut flat = Scene::example_2d();
    flat.seed = 19102026;
    flat.fractal = Fractal::Graph(g.clone());
    flat.canvas = Canvas2D {
        zoom: 1.6,
        rotation: 0.0,
        ..Default::default()
    };
    let planar = PlanarFractal {
        family: PlanarFamily::Julia,
        power: 3,
        iterations: formula(i).iterations,
        julia: formula(i).c.to_array(),
        bailout: 16.0,
        stripes: 9.0,
        trap_radius: 0.52,
        ..Default::default()
    };
    flat.material.texture = Some(FractalTexture {
        graph: Graph::new_minimal(),
        planar: Some(planar.clone()),
        channel: TextureChannel::Orbit {
            channels: channels(),
        },
        scale: 1.0,
        offset: Vec3::ZERO,
        rotation: Vec3::ZERO,
        octaves: 1,
        contrast: 1.0,
        ramp: palette(),
        roughness: 0.0,
        bump: 0.0,
        ..Default::default()
    });
    flat.look = Look::neutral();
    flat.look.tonemap = Tonemap::Direct;
    flat.look.auto_exposure = None;
    let mut solid = Scene::example();
    solid.seed = flat.seed;
    solid.fractal = Fractal::Graph(g.clone());
    // The cap is the actual y=0 section, with w=z+ix, not a height map.
    let cg = solid.fractal.graph_mut();
    let output = cg
        .nodes
        .iter()
        .find(|n| matches!(n.kind, NodeKind::Output))
        .unwrap()
        .id;
    let root = cg.input_of(output, 0).unwrap().0;
    cg.links.retain(|l| l.to.0 != output);
    let plane = chain(
        cg,
        0,
        NodeKind::Plane {
            normal: Vec3::Y,
            offset: 0.0,
        },
        "Section exacte y = 0 ; w = z + ix",
    );
    let clipped = node(
        cg,
        NodeKind::Intersection,
        "Ouvrir le volume sur son plan invariant",
    );
    link(cg, root, clipped, 0);
    link(cg, plane, clipped, 1);
    let box_point = chain(
        cg,
        0,
        NodeKind::Translate {
            offset: Vec3::new(0.0, 0.725, 0.0),
        },
        "Centre de la demi-enveloppe",
    );
    let accelerated = node(
        cg,
        NodeKind::Bounded {
            half: Vec3::new(1.45, 0.725, 1.45),
            margin: 0.005,
        },
        "Enveloppe certifiée extérieure au Julia",
    );
    link(cg, box_point, accelerated, 0);
    link(cg, clipped, accelerated, 1);
    link(cg, accelerated, output, 0);
    solid.camera.mode = CameraMode::Free {
        position: Vec3::new(2.7, 2.55, 3.8),
        target: Vec3::new(0.0, -0.25, 0.0),
        roll: -18.0,
    };
    solid.camera.focal_mm = 58.0;
    solid.camera.f_number = 0.0;
    solid.camera.focus_distance = 5.0;
    solid.material.albedo = [0.90, 0.88, 0.80];
    solid.material.roughness = 0.26;
    solid.material.metallic = 0.45;
    let mut texture = flat.material.texture.clone().unwrap();
    texture.rotation = Vec3::new(0.0, 90.0, 90.0);
    for color in &mut texture.ramp.stops {
        for c in color {
            *c = 0.035 + 0.85 * *c;
        }
    }
    solid.material.texture = Some(texture);
    solid.material.coloring = Coloring::None;
    solid.material.finishes = vec![
        Finish {
            name: "Or brossé du plan invariant".into(),
            albedo: [0.72, 0.38, 0.09],
            roughness: 0.3,
            metallic: 0.8,
            ..Default::default()
        },
        Finish {
            name: "Socle de nuit".into(),
            albedo: [0.0015, 0.0045, 0.010],
            roughness: 0.6,
            metallic: 0.1,
            ..Default::default()
        },
    ];
    let area = |position: Vec3, rgb: [f32; 3], intensity: f32, size: [f32; 2]| Light::Area {
        position,
        normal: (Vec3::new(0.0, 0.0, -0.4) - position).normalize(),
        size,
        color: Color::Linear(rgb),
        intensity,
    };
    solid.lights = vec![
        Light::Environment {
            zenith: Color::Linear([0.10, 0.16, 0.26]),
            horizon: Color::Linear([0.25, 0.30, 0.36]),
            ground: Color::Linear([0.025, 0.028, 0.035]),
            intensity: 0.85,
            rotation: 0.0,
        },
        area(
            Vec3::new(-3.5, 6.0, 5.0),
            [1.0, 0.82, 0.57],
            22.0,
            [5.0, 4.0],
        ),
        area(
            Vec3::new(4.0, 0.8, -4.0),
            [0.12, 0.65, 0.9],
            11.0,
            [3.0, 5.0],
        ),
        area(Vec3::new(0.0, -0.8, 6.0), [0.8, 0.92, 1.0], 4.0, [4.0, 2.0]),
    ];
    solid.atmosphere = Atmosphere::none();
    solid.atmosphere.ground = Some(GroundPlane {
        height: -1.65,
        finish: 2,
        dunes: None,
    });
    solid.look = Look::neutral();
    solid.look.auto_exposure = None;
    solid.look.exposure_ev = 1.5;
    solid.look.tonemap = Tonemap::Agx {
        slope: 1.03,
        power: 1.03,
        offset: 0.0,
    };
    (flat, solid, g)
}

fn audit(r: &Renderer, g: &Graph, f: Formula, path: &Path) -> Result<()> {
    let points = syfrack_render::parity::corpus(16384, 1.6, 19102026)?;
    let points: Vec<_> = points.iter().map(|p| Vec3::new(p.y, 0.0, p.x)).collect();
    let mut scene = Scene::example();
    scene.fractal = Fractal::Graph(g.clone());
    let samples = r.control_distances(&scene, &points)?;
    let mut max_error = 0.0f32;
    let mut surface_error = 0.0f32;
    let mut surface_points = 0;
    let mut interior_band_error = 0.0f32;
    let mut wrong_sign = 0;
    let mut outside_normalized = 0.0f32;
    let mut squared_error = 0.0f64;
    for (p, sample) in points.iter().zip(&samples) {
        let mut z = Vec2::new(p.z, p.x);
        let mut dr = 1.0f32;
        for _ in 0..f.iterations {
            if z.length() > 16.0 {
                break;
            }
            dr *= 3.0 * z.length_squared().max(1e-24);
            z = syfrack_core::scene::planar::complex_power(z, 3) + f.c;
        }
        let d = ((z.length() - 1.4) / dr).max(p.length() - 1.6);
        let error = (d - sample.distance).abs();
        max_error = max_error.max(error);
        ensure!(
            d.is_finite() && sample.distance.is_finite(),
            "non-finite slice field at {p}"
        );
        squared_error += f64::from(error).powi(2);
        if d < 0.0 && sample.distance < 0.0 && d.abs().min(sample.distance.abs()) < 0.01 {
            interior_band_error = interior_band_error.max(error);
        }
        if d.signum() != sample.distance.signum() && d.abs().max(sample.distance.abs()) > 2e-5 {
            wrong_sign += 1;
        }
        if d >= 0.0 && sample.distance >= 0.0 {
            outside_normalized = outside_normalized.max(error / (2e-5 + 1e-3 * d.abs()));
            if d.min(sample.distance) < 0.01 {
                surface_points += 1;
                surface_error = surface_error.max(error);
            }
        }
    }
    let rms = (squared_error / points.len() as f64).sqrt();
    std::fs::write(path,format!("Independent complex polynomial w^3+c / fixed polar GPU Vec3 field at y=0, embedding (Im(w),0,Re(w))\npoints={}\nmax_absolute_field_error={max_error:.9}\nfield_RMS={rms:.9}\nexterior_surface_band_points={surface_points}\nexterior_surface_band_max_error={surface_error:.9}\ninterior_band_max_field_error={interior_band_error:.9}\nsign_mismatches_beyond_2e-5={wrong_sign}\noutside_normalized_error={outside_normalized:.9}\nInterior DE magnitudes are not Euclidean distances; their sign, rather than their magnitude, defines the solid. The test checks all signs and bounds exterior field errors separately from interior estimator magnitudes.\n",points.len()))?;
    println!("slice audit: {} points, field max {max_error:.9}, band max {surface_error:.9}, sign mismatches {wrong_sign}, outside normalized {outside_normalized:.6}",points.len());
    ensure!(
        wrong_sign == 0 && surface_error <= 2e-5 && outside_normalized <= 1.0,
        "slice geometry validation failed; inspect {}",
        path.display()
    );
    Ok(())
}
fn save(r: &Renderer, s: &Scene, cfg: &RenderConfig, stem: &Path) -> Result<()> {
    ensure!(
        !stem.with_extension("png").exists(),
        "output exists: {}",
        stem.display()
    );
    let start = Instant::now();
    let mode = if s.dimension == Dimension::ThreeD {
        ExportMode::Wavefront
    } else {
        ExportMode::Auto
    };
    let mut session = ExportSession::new(r.clone(), s, cfg, mode)?;
    let mut progress = Instant::now();
    while !session.step(Duration::from_millis(100))? {
        if progress.elapsed() > Duration::from_secs(15) {
            let p = session.progress();
            println!(
                "{} {:.1}% {} spp {:.1}s",
                stem.display(),
                p.fraction_complete() * 100.0,
                p.samples_min,
                start.elapsed().as_secs_f64()
            );
            progress = Instant::now();
        }
    }
    session.stop_workers();
    if let ExportSession::Tiled(t) = &session {
        ensure!(
            t.work_statistics().cpu_tiles == 0,
            "unexpected CPU rendering worker"
        );
    }
    let out = session.readback()?;
    ensure!(
        out.samples_min == cfg.samples_per_pixel && out.samples_max == cfg.samples_per_pixel,
        "incomplete samples"
    );
    ensure!(
        out.color.data.iter().flatten().all(|v| v.is_finite()),
        "non-finite radiance"
    );
    let filtered = if s.dimension == Dimension::TwoD {
        out.color.clone()
    } else {
        syfrack_render::denoise::denoise(&out.color, &out.depth, &out.normals)
    };
    let encoded = develop_and_encode(&filtered, &s.look, 16);
    let ron = s.to_ron();
    write_png(
        stem.with_extension("png"),
        &encoded,
        s.look.output,
        Some(&ron),
    )?;
    write_exr(stem.with_extension("exr"), &out.color)?;
    std::fs::write(stem.with_extension("ron"), ron)?;
    println!(
        "{}: {}x{} {} spp in {:.3}s",
        stem.display(),
        cfg.width,
        cfg.height,
        cfg.samples_per_pixel,
        start.elapsed().as_secs_f64()
    );
    Ok(())
}
fn control_image(r: &Renderer, s: &Scene, path: &Path) -> Result<()> {
    let mut session = ControlSession::new(r.clone(), s, (320, 240))?;
    session.wait_complete()?;
    let (w, h) = session.size();
    let bytes = r.context().read_buffer(
        session.accumulation_buffer(),
        u64::from(w) * u64::from(h) * 16,
    )?;
    let pixels = bytes
        .chunks_exact(16)
        .map(bytemuck::pod_read_unaligned)
        .collect();
    let image = HdrImage::from_data(w, h, pixels);
    let mut look = Look::neutral();
    look.auto_exposure = None;
    write_png(
        path,
        &develop_and_encode(&image, &look, 8),
        look.output,
        None,
    )?;
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let mode = args
        .get(1)
        .context("study|final INDEX OUTPUT [width height samples]")?;
    let index: usize = args.get(2).context("INDEX")?.parse()?;
    let dir = Path::new(args.get(3).context("OUTPUT")?);
    std::fs::create_dir_all(dir)?;
    let r = Renderer::new(Arc::new(GpuContext::new()?))?;
    println!("{} {:?}", r.context().info.name, r.context().info.backend);
    let parse = |i: usize, d: u32| -> Result<u32> {
        Ok(args.get(i).map(|s| s.parse()).transpose()?.unwrap_or(d))
    };
    let mut cfg = RenderConfig::new(parse(4, 480)?, parse(5, 360)?, parse(6, 4)?);
    cfg.max_bounces = 1;
    cfg.max_steps = 768;
    cfg.pixel_tolerance = 0.25;
    cfg.tile_size = 256;
    cfg.dispatch_budget_ms = 150;
    if mode == "study" {
        for i in 0..6 {
            let (flat, solid, _) = scenes(i);
            std::fs::write(dir.join(format!("{i:02}-3d.ron")), solid.to_ron())?;
            control_image(&r, &solid, &dir.join(format!("{i:02}-control.png")))?;
            save(&r, &flat, &cfg, &dir.join(format!("{i:02}-2d")))?;
        }
    } else {
        let (mut flat, mut solid, g) = scenes(index);
        audit(&r, &g, formula(index), &dir.join("slice-audit.txt"))?;
        control_image(&r, &solid, &dir.join("control-3d.png"))?;
        if mode == "curate" {
            let review = dir.join("review");
            std::fs::create_dir_all(&review)?;
            let mut draft = cfg.clone();
            draft.width = 640;
            draft.height = 480;
            draft.samples_per_pixel = 8;
            save(&r, &flat, &draft, &review.join("chrysalide-2d"))?;
            save(&r, &solid, &draft, &review.join("chrysalide-3d"))?;
            println!("REVIEW_READY: enter to render the final pair from the review scenes, preserving GPU caches.");
            let mut line = String::new();
            ensure!(
                std::io::stdin().read_line(&mut line)? > 0,
                "curation input closed"
            );
            if !line.trim().is_empty() {
                let values: Vec<u32> = line
                    .split_whitespace()
                    .map(str::parse)
                    .collect::<std::result::Result<_, _>>()?;
                ensure!(
                    values.len() == 4,
                    "curation override: width height samples bounces"
                );
                cfg.width = values[0];
                cfg.height = values[1];
                cfg.samples_per_pixel = values[2];
                cfg.max_bounces = values[3];
            }
            flat = Scene::load(&review.join("chrysalide-2d.ron"))?;
            solid = Scene::load(&review.join("chrysalide-3d.ron"))?;
        }
        cfg.integrator = syfrack_render::ComputeIntegrator::Wavefront;
        let mut flat_cfg = cfg.clone();
        flat_cfg.integrator = syfrack_render::ComputeIntegrator::Megakernel;
        std::fs::write(
            dir.join("render-2d-config.ron"),
            ron::ser::to_string_pretty(&flat_cfg, Default::default())?,
        )?;
        std::fs::write(
            dir.join("render-3d-config.ron"),
            ron::ser::to_string_pretty(&cfg, Default::default())?,
        )?;
        save(&r, &flat, &flat_cfg, &dir.join("chrysalide-2d"))?;
        save(&r, &solid, &cfg, &dir.join("chrysalide-3d"))?;
    }
    Ok(())
}
