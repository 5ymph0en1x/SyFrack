//! Reproducible raw-linear error and equal-time GI guiding comparison.
//! First run `hdri_lab` to create the self-contained lighting asset/scene.
use anyhow::Result;
use glam::Vec3;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use syfrack_color::{develop_and_encode, write_exr, write_png};
use syfrack_core::{
    Atmosphere, CameraMode, Color, Coloring, Fractal, Graph, HdrImage, Light, Link, Node, NodeKind,
    Scene,
};
use syfrack_render::{GpuContext, RenderConfig, RenderOutput, RenderSession, Renderer};

fn render(r: &Renderer, scene: &Scene, cfg: &RenderConfig) -> Result<RenderOutput> {
    let mut session = RenderSession::new(r.clone(), scene, cfg)?;
    while !session.is_done() {
        let slice = cfg.time_budget.map_or(Duration::from_millis(250), |limit| {
            limit
                .saturating_sub(session.progress().elapsed)
                .min(Duration::from_millis(10))
        });
        session.step(slice)?;
        if cfg
            .time_budget
            .is_some_and(|limit| session.progress().elapsed >= limit)
        {
            break;
        }
    }
    let stats = session.guiding_stats();
    println!(
        "guide updates {}, active cells {}, trusted {}, leaves {}, observations {}, bytes {}",
        stats.updates,
        stats.active_cells,
        stats.trusted_cells,
        stats.spatial_leaves,
        stats.training_records,
        stats.memory_bytes
    );
    session.readback()
}

fn mse(image: &HdrImage, reference: &HdrImage) -> f64 {
    image
        .data
        .iter()
        .zip(&reference.data)
        .flat_map(|(a, b)| (0..3).map(move |c| (a[c] as f64 - b[c] as f64).powi(2)))
        .sum::<f64>()
        / (image.data.len() * 3) as f64
}

fn indirect_room() -> Scene {
    let mut s = Scene::example();
    let kinds = [
        NodeKind::Input,
        NodeKind::Box {
            half: Vec3::splat(1.05),
        },
        NodeKind::Box { half: Vec3::ONE },
        NodeKind::Subtraction,
        NodeKind::Translate {
            offset: Vec3::new(-1.0, -0.65, 0.0),
        },
        NodeKind::Box {
            half: Vec3::new(0.2, 0.15, 0.2),
        },
        NodeKind::Subtraction,
        NodeKind::Output,
    ];
    let mut g = Graph::new_minimal();
    g.nodes = kinds
        .into_iter()
        .enumerate()
        .map(|(id, kind)| Node {
            id: id as u32,
            name: kind.label().into(),
            kind,
            pos: [(id as f32) * 120.0, 0.0],
            enabled: true,
        })
        .collect();
    g.links = [
        (0, 1, 0),
        (0, 2, 0),
        (1, 3, 0),
        (2, 3, 1),
        (0, 4, 0),
        (4, 5, 0),
        (3, 6, 0),
        (5, 6, 1),
        (6, 7, 0),
    ]
    .into_iter()
    .map(|(from, to, port)| Link {
        from: (from, 0),
        to: (to, port),
    })
    .collect();
    g.bounding_radius_hint = 3.0;
    s.fractal = Fractal::Graph(g);
    s.camera.mode = CameraMode::Free {
        position: Vec3::new(0.0, -0.1, 0.8),
        target: Vec3::new(-0.4, -0.65, -0.6),
        roll: 0.0,
    };
    s.camera.focal_mm = 28.0;
    s.camera.f_number = 0.0;
    s.material.coloring = Coloring::None;
    s.material.texture = None;
    s.material.surface = None;
    s.material.specular = 0.0;
    s.atmosphere = Atmosphere::none();
    s.lights = vec![Light::Area {
        position: Vec3::new(1.3, 0.65, 0.0),
        normal: Vec3::NEG_X,
        size: [0.25, 0.25],
        color: Color::Linear([1.0; 3]),
        intensity: 100.0,
    }];
    s
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().collect();
    let dir = args
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/guiding-lab"));
    let input = args
        .get(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/hdri-lab/menger-hdri.ron"));
    let spp = args
        .get(3)
        .and_then(|v| v.to_str())
        .map_or(Ok(256), |v| v.parse::<u32>())?;
    std::fs::create_dir_all(&dir)?;
    let dir = dir.canonicalize()?;
    let mut scene = if input.as_os_str() == "indirect" || input.as_os_str() == "indirect-close" {
        let mut room = indirect_room();
        if input.as_os_str() == "indirect-close" {
            room.camera.focal_mm = 500.0;
        }
        room
    } else {
        Scene::load(input)?
    };
    // Rough, predominantly diffuse walls expose indirect transport in the
    // holes. A sharp mirror should stay with the existing GGX VNDF sampler.
    scene.material.metallic = 0.0;
    scene.material.roughness = 0.6;
    scene.material.albedo = [0.8, 0.65, 0.4];
    scene.save(dir.join("scene.ron"))?;
    let context = Arc::new(GpuContext::new()?);
    println!("GPU: {}", context.info.name);
    let r = Renderer::new(context)?;
    let mut cfg = RenderConfig::new(160, 120, 8192);
    cfg.max_bounces = 6;
    let mut independent = scene.clone();
    independent.seed ^= 0x9148a353;
    let reference = render(&r, &independent, &cfg)?;
    cfg.samples_per_pixel = spp;
    cfg.guiding = true;
    let guided = render(&r, &scene, &cfg)?;
    cfg.guiding = false;
    let plain = render(&r, &scene, &cfg)?;
    // Spend the guided render's total sampling/training time on the baseline.
    cfg.samples_per_pixel = 100_000;
    cfg.time_budget = Some(guided.elapsed);
    let equal_time = render(&r, &scene, &cfg)?;
    let mut comparison = HdrImage::new(cfg.width * 3, cfg.height);
    let mut report = format!("GPU: {}\nRaw linear RGB MSE; independent 8192 spp reference; no denoising.\nTiming includes training, cache readback/build/upload and tile dispatches; excludes initial pipeline/asset creation and final export.\n", r.context().info.name);
    for (panel, name, out) in [
        (0, "baseline", &plain),
        (1, "guided", &guided),
        (2, "baseline-equal-time", &equal_time),
    ] {
        let line = format!(
            "{name}: {}..{} spp, {:.4}s, MSE {:.8}\n",
            out.samples_min,
            out.samples_max,
            out.elapsed.as_secs_f64(),
            mse(&out.color, &reference.color)
        );
        print!("{line}");
        report.push_str(&line);
        write_exr(dir.join(format!("{name}.exr")), &out.color)?;
        write_png(
            dir.join(format!("{name}.png")),
            &develop_and_encode(&out.color, &scene.look, 8),
            scene.look.output,
            Some(&scene.to_ron()),
        )?;
        for y in 0..cfg.height {
            for x in 0..cfg.width {
                comparison.set(x + panel * cfg.width, y, out.color.get(x, y));
            }
        }
    }
    write_exr(dir.join("reference.exr"), &reference.color)?;
    write_png(
        dir.join("comparison.png"),
        &develop_and_encode(&comparison, &scene.look, 8),
        scene.look.output,
        None,
    )?;
    std::fs::write(dir.join("metrics.txt"), report)?;
    Ok(())
}
