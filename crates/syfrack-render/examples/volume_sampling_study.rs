//! Raw variance diagnostic: a nearly constant camera ray, coloured area lights,
//! homogeneous fog and no visible surface. No denoiser or image grading.
use anyhow::Result;
use glam::Vec3;
use std::{path::Path, sync::Arc, time::Instant};
use syfrack_core::{Atmosphere, CameraMode, Color, Fractal, Graph, Light, NodeKind, Scene};
use syfrack_render::{GpuContext, RenderConfig, Renderer};

fn main() -> Result<()> {
    let destination = std::env::args().nth(1).expect("output directory");
    let destination = Path::new(&destination);
    std::fs::create_dir_all(destination)?;
    let renderer = Renderer::new(Arc::new(GpuContext::new()?))?;
    let mut scene = Scene::example();
    let mut graph = Graph::new_minimal();
    graph.node_mut(1).unwrap().kind = NodeKind::Plane {
        normal: Vec3::X,
        offset: 100.0,
    };
    scene.fractal = Fractal::Graph(graph);
    scene.camera.mode = CameraMode::Free {
        position: Vec3::ZERO,
        target: Vec3::Z,
        roll: 0.0,
    };
    scene.camera.focal_mm = 1200.0;
    scene.camera.f_number = 0.0;
    scene.atmosphere = Atmosphere::none();
    scene.atmosphere.fog_density = 0.014;
    scene.atmosphere.fog_color = [0.9; 3];
    scene.atmosphere.fog_falloff = 0.0;
    scene.atmosphere.anisotropy = 0.15;
    scene.lights.clear();
    for (x, rgb) in [(-3.0, [1.0, 0.0, 0.0]), (3.0, [0.0, 0.0, 1.0])] {
        scene.lights.push(Light::Area {
            position: Vec3::new(x, 0.0, 10.0),
            normal: Vec3::new(-x.signum(), 0.0, 0.0),
            size: [2.0; 2],
            color: Color::Linear(rgb),
            intensity: 50.0,
        });
    }
    let mut report = String::from("Raw linear RGB, no denoiser. Nearly constant field.\n");
    for samples in [32, 512] {
        let mut cfg = RenderConfig::new(32, 32, samples);
        cfg.max_bounces = 0;
        let started = Instant::now();
        let output = renderer.render(&scene, &cfg, &mut |_| {})?;
        let elapsed = started.elapsed().as_secs_f64();
        syfrack_color::write_exr(&destination.join(format!("{samples}.exr")), &output.color)?;
        for channel in [0, 2] {
            let n = output.color.data.len() as f64;
            let mean = output
                .color
                .data
                .iter()
                .map(|p| p[channel] as f64)
                .sum::<f64>()
                / n;
            let variance = output
                .color
                .data
                .iter()
                .map(|p| (p[channel] as f64 - mean).powi(2))
                .sum::<f64>()
                / n;
            report.push_str(&format!("spp={samples} channel={channel} mean={mean:.9} variance={variance:.9} cv={:.6} seconds={elapsed:.3}\n", variance.sqrt()/mean));
        }
    }
    std::fs::write(destination.join("report.txt"), &report)?;
    print!("{report}");
    Ok(())
}
