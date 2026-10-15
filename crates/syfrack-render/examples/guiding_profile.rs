//! Repeatable host-side guiding cost breakdown. No GPU timestamp claims.
//! Arguments: OUTPUT_DIR [SPP=1024] [REPEATS=5] [menger,room,close]
use anyhow::Result;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use syfrack_color::{read_exr, write_exr};
use syfrack_core::{HdrImage, Scene};
use syfrack_render::{GpuContext, RenderConfig, RenderOutput, RenderSession, Renderer};

fn mse(a: &HdrImage, b: &HdrImage) -> f64 {
    assert_eq!((a.width, a.height), (b.width, b.height));
    a.data
        .iter()
        .zip(&b.data)
        .flat_map(|(a, b)| (0..3).map(move |c| (a[c] as f64 - b[c] as f64).powi(2)))
        .sum::<f64>()
        / (a.data.len() * 3) as f64
}

fn run(
    r: &Renderer,
    scene: &Scene,
    cfg: &RenderConfig,
) -> Result<(
    RenderOutput,
    syfrack_render::guiding::GuidingProfile,
    syfrack_render::guiding::GuidingStats,
    f64,
)> {
    let t = Instant::now();
    let mut session = RenderSession::new(r.clone(), scene, cfg)?;
    let init = t.elapsed().as_secs_f64();
    while !session.is_done() {
        session.step(Duration::from_millis(100))?;
    }
    Ok((
        session.readback()?,
        session.guiding_profile(),
        session.guiding_stats(),
        init,
    ))
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("compare") {
        anyhow::ensure!(
            (4..=5).contains(&args.len()),
            "compare BEFORE_DIR AFTER_DIR [menger,room,close]"
        );
        for name in args
            .get(4)
            .map_or("menger,room,close", String::as_str)
            .split(',')
        {
            for mode in ["baseline", "guided"] {
                let file = format!("{name}-{mode}.exr");
                let a = read_exr(PathBuf::from(&args[2]).join(&file))?;
                let b = read_exr(PathBuf::from(&args[3]).join(&file))?;
                let differences = a.data.iter().zip(&b.data).filter(|(a, b)| a != b).count();
                println!(
                    "{file}: {differences} different pixels, MSE {:.12}",
                    mse(&a, &b)
                );
                anyhow::ensure!(differences == 0, "image changed: {file}");
            }
        }
        return Ok(());
    }
    let dir = PathBuf::from(args.get(1).map_or("target/guiding-profile", String::as_str));
    let spp: u32 = args.get(2).map_or(Ok(1024), |s| s.parse())?;
    let repeats: u32 = args.get(3).map_or(Ok(5), |s| s.parse())?;
    let selection = args.get(4).map_or("menger,room,close", String::as_str);
    anyhow::ensure!(repeats > 0, "repeats must be positive");
    std::fs::create_dir_all(&dir)?;
    let r = Renderer::new(Arc::new(GpuContext::new()?))?;
    let mut csv = String::from(
        "scene,repeat,mode,elapsed_s,init_s,dispatch_s,readback_s,decode_s,fit_s,upload_s,other_s,readback_bytes,upload_bytes,leaves,trusted,mse,stage0_s,stage1_s,stage2_s,stage3_s,stage4_s,stage5_s\n",
    );
    let mut stage_csv = String::from("scene,repeat,mode,stage,dispatches,pixel_samples,wall_s\n");
    let mut variant_csv =
        String::from("scene,repeat,mode,variant,dispatches,pixel_samples,wall_s\n");
    let mut report = format!(
        "GPU: {} ({:?})\nSPP: {spp}; repeats: {repeats}; raw linear RGB; no denoising.\nDispatch time is host wall time, including submission/wait/validation, not GPU timestamps.\nSession initialization and final image readback/export are excluded from elapsed time.\n",
        r.context().info.name,
        r.context().info.backend
    );
    for name in selection.split(',') {
        let (scene_path, reference_path, dimensions, bounces) = match name {
            "menger" => (
                "docs/benchmarks/guiding-cost-2026-10-09/fixtures/menger/scene.ron",
                Some("docs/benchmarks/guiding-cost-2026-10-09/fixtures/menger/reference.exr"),
                (160, 120),
                6,
            ),
            "room" => (
                "docs/benchmarks/guiding-cost-2026-10-09/fixtures/room/scene.ron",
                Some("docs/benchmarks/guiding-cost-2026-10-09/fixtures/room/reference.exr"),
                (160, 120),
                6,
            ),
            "close" => (
                "docs/benchmarks/guiding-cost-2026-10-09/fixtures/close/scene.ron",
                Some("docs/benchmarks/guiding-cost-2026-10-09/fixtures/close/reference.exr"),
                (160, 120),
                6,
            ),
            "cathedral" => (
                "scenes/environments/cathedrale-neon.ron",
                None,
                (160, 90),
                4,
            ),
            _ => anyhow::bail!("unknown fixture {name}"),
        };
        let scene = Scene::load(scene_path)?;
        let reference = reference_path.map(read_exr).transpose()?;
        let mut cfg = RenderConfig::new(dimensions.0, dimensions.1, 64);
        cfg.max_bounces = bounces;
        // Warm the same scene and both modes before recording trials.
        for guided in [false, true] {
            cfg.guiding = guided;
            run(&r, &scene, &cfg)?;
        }
        cfg.samples_per_pixel = spp;
        let mut times = [Vec::new(), Vec::new()];
        for trial in 0..repeats {
            let order = if trial % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            };
            for guided in order {
                cfg.guiding = guided;
                let (out, profile, stats, init) = run(&r, &scene, &cfg)?;
                let mode = if guided { "guided" } else { "baseline" };
                let dispatch: f64 = profile.stages.iter().map(|s| s.wall.as_secs_f64()).sum();
                let readback: f64 = profile
                    .updates
                    .iter()
                    .map(|u| u.readback.as_secs_f64())
                    .sum();
                let decode: f64 = profile.updates.iter().map(|u| u.decode.as_secs_f64()).sum();
                let fit: f64 = profile.updates.iter().map(|u| u.fit.as_secs_f64()).sum();
                let upload: f64 = profile.updates.iter().map(|u| u.upload.as_secs_f64()).sum();
                let readback_bytes: u64 = profile.updates.iter().map(|u| u.readback_bytes).sum();
                let upload_bytes: u64 = profile.updates.iter().map(|u| u.upload_bytes).sum();
                let elapsed = out.elapsed.as_secs_f64();
                let other = elapsed - dispatch - readback - decode - fit - upload;
                let error = reference
                    .as_ref()
                    .map(|ref_img| format!("{:.10}", mse(&out.color, ref_img)))
                    .unwrap_or_default();
                let stages = profile
                    .stages
                    .iter()
                    .map(|s| format!("{:.9}", s.wall.as_secs_f64()))
                    .collect::<Vec<_>>()
                    .join(",");
                csv.push_str(&format!("{name},{trial},{mode},{elapsed:.9},{init:.9},{dispatch:.9},{readback:.9},{decode:.9},{fit:.9},{upload:.9},{other:.9},{readback_bytes},{upload_bytes},{},{},{error},{stages}\n", stats.spatial_leaves, stats.trusted_cells));
                for (stage, s) in profile.stages.iter().enumerate() {
                    stage_csv.push_str(&format!(
                        "{name},{trial},{mode},{stage},{},{},{:.9}\n",
                        s.dispatches,
                        s.pixel_samples,
                        s.wall.as_secs_f64()
                    ));
                }
                for (variant, s) in profile.variants.iter().enumerate() {
                    variant_csv.push_str(&format!(
                        "{name},{trial},{mode},{variant},{},{},{:.9}\n",
                        s.dispatches,
                        s.pixel_samples,
                        s.wall.as_secs_f64()
                    ));
                }
                println!(
                    "{name} {trial} {mode}: {elapsed:.4}s; dispatch {dispatch:.4}s; fit {fit:.4}s; transfer {:.4}s; leaves {} trusted {} MSE {error}",
                    readback + upload,
                    stats.spatial_leaves,
                    stats.trusted_cells
                );
                times[usize::from(guided)].push(elapsed);
                if trial == repeats - 1 {
                    write_exr(dir.join(format!("{name}-{mode}.exr")), &out.color)?;
                }
                std::fs::write(dir.join("trials.csv"), &csv)?;
                std::fs::write(dir.join("stages.csv"), &stage_csv)?;
                std::fs::write(dir.join("variants.csv"), &variant_csv)?;
            }
        }
        let mut medians = [0.0; 2];
        for (mode, ts) in times.iter_mut().enumerate() {
            ts.sort_by(f64::total_cmp);
            medians[mode] = ts[ts.len() / 2];
        }
        let line = format!(
            "{name}: median baseline {:.6}s; guided {:.6}s; ratio {:.4}\n",
            medians[0],
            medians[1],
            medians[1] / medians[0]
        );
        print!("{line}");
        report.push_str(&line);
        std::fs::write(dir.join("summary.txt"), &report)?;
    }
    Ok(())
}
