//! Reconstruct saved native renders without adding or changing traced samples.
//! Keep one renderer for reusable material-guide pipelines across checkpoints.
#[allow(dead_code)]
#[path = "scene_sequence.rs"]
mod scene_sequence;
use anyhow::Result;
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use syfrack_render::{GpuContext, RenderSession, Renderer};

#[derive(Deserialize)]
struct Job {
    state: String,
    output: String,
}

fn main() -> Result<()> {
    let dir = PathBuf::from(std::env::args().nth(1).expect("JOB_DIRECTORY"));
    std::fs::create_dir_all(&dir)?;
    let renderer = Renderer::new(Arc::new(GpuContext::new()?))?;
    let mut idle = Instant::now();
    loop {
        let mut jobs = std::fs::read_dir(&dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.to_string_lossy().ends_with(".job.ron") && !p.with_extension("done").exists()
            })
            .collect::<Vec<_>>();
        jobs.sort();
        let Some(path) = jobs.first() else {
            if dir.join("STOP").exists() || idle.elapsed() > Duration::from_secs(7200) {
                break;
            }
            std::thread::sleep(Duration::from_secs(1));
            continue;
        };
        let job: Job = ron::from_str(&std::fs::read_to_string(path)?)?;
        let start = Instant::now();
        println!("RECONSTRUCT {}", job.state);
        let session = RenderSession::resume(renderer.clone(), Path::new(&job.state))?;
        scene_sequence::export(&session, Path::new(&job.output), false)?;
        let p = session.progress();
        let proof = format!(
            "{}: {}..{} spp unchanged, native separated reconstruction in {:.1}s\n",
            job.output,
            p.samples_min,
            p.samples_max,
            start.elapsed().as_secs_f64()
        );
        std::fs::write(path.with_extension("done"), &proof)?;
        println!("DONE {proof}");
        idle = Instant::now();
    }
    Ok(())
}
