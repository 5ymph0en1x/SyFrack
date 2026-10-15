//! Equal-sample comparison of rectangle-only NEE and NEE + BSDF MIS.
//! A convex sphere with a black environment has no indirect surface lighting,
//! so both modes integrate the same softboxes. The differences are sampling.
use anyhow::Result;
use glam::Vec3;
use std::path::PathBuf;
use std::sync::Arc;
use syfrack_color::{develop_and_encode, write_exr, write_png};
use syfrack_core::{
    Atmosphere, CameraMode, Color, Coloring, Fractal, Graph, HdrImage, Look, Scene,
};
use syfrack_render::{GpuContext, RenderConfig, Renderer};

fn main() -> Result<()> {
    let dir = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/lighting-lab"));
    std::fs::create_dir_all(&dir)?;
    let mut scene = Scene::example();
    scene.fractal = Fractal::Graph(Graph::new_minimal());
    scene.camera.mode = CameraMode::Free {
        position: Vec3::new(0.0, 0.0, 4.8),
        target: Vec3::ZERO,
        roll: 0.0,
    };
    scene.camera.focal_mm = 40.0;
    scene.camera.f_number = 0.0;
    scene.material.albedo = [0.85, 0.85, 0.85];
    scene.material.roughness = 0.035;
    scene.material.metallic = 1.0;
    scene.material.coloring = Coloring::None;
    scene.material.texture = None;
    scene.material.surface = None;
    scene.atmosphere = Atmosphere::none();
    scene.look = Look::neutral();
    scene.lights = vec![
        syfrack_core::Light::Area {
            position: Vec3::new(-3.0, 4.0, 5.0),
            normal: Vec3::new(3.0, -4.0, -5.0).normalize(),
            size: [4.0, 4.0],
            color: Color::Linear([1.0, 0.8, 0.6]),
            intensity: 6.0,
        },
        syfrack_core::Light::Area {
            position: Vec3::new(3.0, 0.5, 5.0),
            normal: Vec3::new(-3.0, -0.5, -5.0).normalize(),
            size: [2.0, 3.5],
            color: Color::Linear([0.35, 0.65, 1.0]),
            intensity: 4.0,
        },
    ];
    scene.save(dir.join("softbox-sphere.ron"))?;
    let context = Arc::new(GpuContext::new()?);
    println!("GPU: {}", context.info.name);
    let renderer = Renderer::new(context)?;
    let mut comparison = HdrImage::new(640, 200);
    for (panel, name, bounces) in [(0, "nee", 0), (1, "mis", 1)] {
        let mut cfg = RenderConfig::new(320, 200, 32);
        cfg.max_bounces = bounces;
        let out = renderer.render(&scene, &cfg, &mut |_| {})?;
        println!("{name}: 32 spp, {:.2}s", out.elapsed.as_secs_f64());
        write_exr(dir.join(format!("{name}.exr")), &out.color)?;
        let encoded = develop_and_encode(&out.color, &scene.look, 8);
        write_png(
            dir.join(format!("{name}.png")),
            &encoded,
            scene.look.output,
            Some(&scene.to_ron()),
        )?;
        for y in 0..200 {
            for x in 0..320 {
                comparison.set(x + panel * 320, y, out.color.get(x, y));
            }
        }
    }
    let encoded = develop_and_encode(&comparison, &scene.look, 8);
    write_png(
        dir.join("comparison.png"),
        &encoded,
        scene.look.output,
        None,
    )?;
    println!(
        "NEE left / MIS right: {}",
        dir.join("comparison.png").display()
    );
    Ok(())
}
