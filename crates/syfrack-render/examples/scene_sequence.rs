//! Sequential native studies/finals sharing one renderer and its pipeline cache.
//! cargo run -p syfrack-render --example scene_sequence -- JOB_DIRECTORY
//! Drop *.job.ron files atomically, in lexical order. Create STOP to finish.
use anyhow::{ensure, Result};
use serde::Deserialize;
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use syfrack_color::{develop_and_encode, write_exr, write_png};
use syfrack_core::{HdrImage, Scene};
use syfrack_render::{denoise, GpuContext, RenderConfig, RenderSession, Renderer};

// Keep the accurate trace tolerance while using the fully guided reconstruction.
// The engine currently couples raw-detail reinjection to trace tolerance; for
// these low-sample studies that reinjection restores specular noise on the lace.
struct GuidedExport<'a> {
    session: &'a RenderSession,
    config: RenderConfig,
}
impl denoise::ExportSource for GuidedExport<'_> {
    fn scene(&self) -> &Scene {
        self.session.scene()
    }
    fn config(&self) -> &RenderConfig {
        &self.config
    }
    fn export_guides(&self) -> Result<(HdrImage, HdrImage)> {
        self.session.export_guides()
    }
    fn export_surface_guides(&self) -> Result<(HdrImage, HdrImage)> {
        self.session.export_surface_guides()
    }
}

#[derive(Deserialize)]
struct Job {
    scene: String,
    output: String,
    width: u32,
    height: u32,
    samples: u32,
    bounces: u32,
}

pub fn export(session: &RenderSession, stem: &Path, final_image: bool) -> Result<()> {
    let output = session.readback()?;
    let scene = session.scene();
    let ron = scene.to_ron();
    if final_image {
        write_exr(stem.with_extension("exr"), &output.color)?;
        std::fs::write(stem.with_extension("ron"), &ron)?;
    }
    let mut config = session.config().clone();
    config.pixel_tolerance = 0.4; // Export policy only; no rays are changed.
    let filtered = denoise::session_output(&GuidedExport { session, config }, &output);
    write_png(
        stem.with_extension("png"),
        &develop_and_encode(&filtered, &scene.look, 16),
        scene.look.output,
        Some(&ron),
    )?;
    Ok(())
}

fn main() -> Result<()> {
    let directory = std::path::PathBuf::from(std::env::args().nth(1).expect("JOB_DIRECTORY"));
    std::fs::create_dir_all(&directory)?;
    let renderer = Renderer::new(Arc::new(GpuContext::new()?))?;
    let mut idle = Instant::now();
    loop {
        let mut jobs = std::fs::read_dir(&directory)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.to_string_lossy().ends_with(".job.ron") && !p.with_extension("done").exists()
            })
            .collect::<Vec<_>>();
        jobs.sort();
        let Some(path) = jobs.first() else {
            if directory.join("STOP").exists() || idle.elapsed() > Duration::from_secs(3600) {
                break;
            }
            std::thread::sleep(Duration::from_millis(1000));
            continue;
        };
        let job: Job = ron::from_str(&std::fs::read_to_string(path)?)?;
        let scene = Scene::load(&job.scene)?;
        let stem = Path::new(&job.output);
        ensure!(
            !stem.with_extension("png").exists(),
            "output already exists"
        );
        if let Some(parent) = stem.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut config = RenderConfig::new(job.width, job.height, job.samples);
        config.max_bounces = job.bounces;
        config.max_steps = 256;
        config.pixel_tolerance = 0.25;
        config.tile_size = 64;
        config.samples_per_dispatch = 1;
        config.denoise = true;
        let start = Instant::now();
        println!(
            "BEGIN {} {}x{} {} spp / {} bounces",
            job.output, job.width, job.height, job.samples, job.bounces
        );
        let mut session = RenderSession::new(renderer.clone(), &scene, &config)?;
        println!("Pipeline ready after {:.1}s", start.elapsed().as_secs_f64());
        let mut report = Instant::now();
        let mut snapshot = Instant::now();
        loop {
            let done = session.step(Duration::from_millis(500))?;
            if report.elapsed() > Duration::from_secs(15) || done {
                let p = session.progress();
                println!(
                    "{} {:.2}% {}..{} spp, {:.1}s",
                    job.output,
                    p.fraction_complete() * 100.0,
                    p.samples_min,
                    p.samples_max,
                    start.elapsed().as_secs_f64()
                );
                report = Instant::now();
            }
            if done {
                break;
            }
            if snapshot.elapsed() > Duration::from_secs(240) && session.progress().samples_min > 0 {
                session.save(&stem.with_extension("state"))?;
                let preview = stem.with_file_name(format!(
                    "{}-preview",
                    stem.file_name().unwrap().to_string_lossy()
                ));
                export(&session, &preview, false)?;
                snapshot = Instant::now();
            }
        }
        export(&session, stem, true)?;
        session.save(&stem.with_extension("state"))?;
        let p = session.progress();
        let proof = format!(
            "complete: {}x{}, {}..{} spp, {} bounces, tolerance .25; wall {:.1}s; trace {:.1}s\n",
            job.width,
            job.height,
            p.samples_min,
            p.samples_max,
            job.bounces,
            start.elapsed().as_secs_f64(),
            p.elapsed.as_secs_f64()
        );
        std::fs::write(path.with_extension("done"), &proof)?;
        println!("DONE {proof}");
        idle = Instant::now();
    }
    Ok(())
}
