use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;
use syfrack_app_lib::final_render::{run_job, Command, Event, FinalRender, RenderJob};
use syfrack_core::Scene;
use syfrack_render::{GpuContext, RenderConfig, RenderSession, Renderer, SessionState};

/// A fresh directory per test, removed when the test ends (even on panic).
struct Tmp(std::path::PathBuf);

impl Tmp {
    fn new(name: &str) -> Self {
        let d = std::env::temp_dir().join(format!("syfrack-final-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Self(d)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
#[ignore = "needs GPU"]
fn job_completes_and_matches_a_direct_render_bit_for_bit() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let scene = Scene::example();
    let mut cfg = RenderConfig::new(64, 48, 8);
    cfg.tile_size = 64;
    cfg.samples_per_dispatch = 2;
    let reference = r.render(&scene, &cfg, &mut |_| {}).unwrap();
    let dir = Tmp::new("job");
    let stem = dir.0.join("job");
    let mut job = FinalRender::start(
        r.clone(),
        scene.clone(),
        RenderJob {
            out_stem: stem.clone(),
            width: 64,
            height: 48,
            samples: 8,
            bounces: 4,
            max_steps: 256,
            pixel_tolerance: 1.0,
            tile_size: 64,
            samples_per_dispatch: 2,
            denoise: false,
            guiding: false,
        },
    )
    .unwrap();
    let mut finished = None;
    let mut last_min = 0;
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while finished.is_none() && std::time::Instant::now() < deadline {
        for e in job.poll() {
            match e {
                Event::Progress(p) => {
                    assert!(p.samples_min >= last_min, "progress is monotone");
                    last_min = p.samples_min;
                    // Adaptive dispatch rectangles can cover several display cells.
                    assert_eq!(p.tiles_done.len(), (p.tile_grid.0 * p.tile_grid.1) as usize);
                    assert!(p.tiles <= p.tiles_done.len());
                    assert!(p
                        .tiles_done
                        .iter()
                        .all(|&done| (p.samples_min..=p.samples_max).contains(&done)));
                }
                Event::Finished { exr, .. } => finished = Some(exr),
                Event::Failed(m) => panic!("job failed: {m}"),
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let exr = finished.expect("finished within 60 s");
    assert!(!job.is_running());
    let img = syfrack_color::read_exr(&exr).unwrap();
    assert_eq!(img.data, reference.color.data);
    let saved: SessionState = ron::from_str(
        &std::fs::read_to_string(stem.with_extension("state").join("state.ron")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        (saved.cfg.tile_size, saved.cfg.samples_per_dispatch),
        (64, 2)
    );
    let png = dir.0.join("job.png");
    assert!(png.exists());
    let embedded = syfrack_color::read_png_text(&png, syfrack_color::SCENE_KEYWORD).unwrap();
    let ron = std::fs::read_to_string(dir.0.join("job.ron")).unwrap();
    assert_eq!(
        embedded.as_deref(),
        Some(ron.as_str()),
        "the PNG embeds the same scene as the .ron"
    );
}

#[test]
#[ignore = "needs GPU"]
fn pause_saves_state_and_resume_finishes_identically() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let scene = Scene::example_for("mandelbulb").unwrap();
    let cfg = RenderConfig::new(96, 96, 600);
    let reference = r.render(&scene, &cfg, &mut |_| {}).unwrap();
    let dir_guard = Tmp::new("paused");
    let stem = dir_guard.0.join("paused");
    let mut job = FinalRender::start(
        r.clone(),
        scene.clone(),
        RenderJob {
            out_stem: stem.clone(),
            width: 96,
            height: 96,
            samples: 600,
            bounces: 4,
            max_steps: 256,
            pixel_tolerance: 1.0,
            tile_size: 256,
            samples_per_dispatch: 8,
            denoise: false,
            guiding: false,
        },
    )
    .unwrap();
    // Pause as soon as the first progress arrives.
    let mut paused_dir = None;
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let mut sent = false;
    while paused_dir.is_none() && std::time::Instant::now() < deadline {
        for e in job.poll() {
            match e {
                Event::Progress(_) if !sent => {
                    job.send(Command::Pause);
                    sent = true;
                }
                Event::Paused(dir) => paused_dir = Some(dir),
                Event::Finished { .. } => {
                    panic!("finished before the pause took effect; make the render bigger")
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let dir = paused_dir.expect("paused");
    assert!(dir.join("state.ron").exists());
    assert!(!job.is_running());
    // The CLI reads the same state.
    let text = std::fs::read_to_string(dir.join("state.ron")).unwrap();
    let state: SessionState = ron::from_str(&text).expect("state readable by the engine");
    assert_eq!((state.cfg.width, state.cfg.height), (96, 96));
    let mut job2 = FinalRender::resume(r.clone(), dir, stem.clone()).unwrap();
    let mut finished = None;
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while finished.is_none() && std::time::Instant::now() < deadline {
        for e in job2.poll() {
            if let Event::Finished { exr, .. } = e {
                finished = Some(exr);
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let img = syfrack_color::read_exr(finished.unwrap()).unwrap();
    assert_eq!(
        img.data, reference.color.data,
        "resumed job must be bit-identical"
    );
}

#[test]
#[ignore = "needs GPU"]
fn run_job_reports_failure_instead_of_panicking_on_a_bad_output_dir() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let session =
        RenderSession::new(r.clone(), &Scene::example(), &RenderConfig::new(16, 16, 1)).unwrap();
    let (_ctx, rx) = mpsc::channel::<Command>();
    let (tx, events) = mpsc::channel::<Event>();
    let bad = std::path::PathBuf::from("Z:/definitely/not/here/out");
    run_job(r, session, bad, rx, tx);
    let got: Vec<Event> = events.try_iter().collect();
    assert!(got.iter().any(|e| matches!(e, Event::Failed(_))));
}

#[test]
#[ignore = "needs GPU"]
fn cancel_ends_the_thread_without_a_state() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let dir = Tmp::new("cancelled");
    let stem = dir.0.join("cancelled");
    let mut job = FinalRender::start(
        r.clone(),
        Scene::example(),
        RenderJob {
            out_stem: stem.clone(),
            width: 256,
            height: 256,
            samples: 100_000,
            bounces: 4,
            max_steps: 256,
            pixel_tolerance: 1.0,
            tile_size: 256,
            samples_per_dispatch: 8,
            denoise: false,
            guiding: false,
        },
    )
    .unwrap();
    job.send(Command::Cancel);
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while job.is_running() && std::time::Instant::now() < deadline {
        job.poll();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!job.is_running(), "cancel stops the job");
    assert!(!job.state_dir().exists(), "cancel writes no state");
}

#[test]
#[ignore = "needs GPU"]
fn dropping_a_running_job_pauses_it_and_keeps_the_state() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let dir = Tmp::new("dropped");
    let stem = dir.0.join("dropped");
    let mut job = FinalRender::start(
        r.clone(),
        Scene::example(),
        RenderJob {
            out_stem: stem.clone(),
            width: 256,
            height: 256,
            samples: 100_000,
            bounces: 4,
            max_steps: 256,
            pixel_tolerance: 1.0,
            tile_size: 256,
            samples_per_dispatch: 8,
            denoise: false,
            guiding: false,
        },
    )
    .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while job.last_progress().is_none() && std::time::Instant::now() < deadline {
        job.poll();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(job.last_progress().is_some(), "the job is rendering");
    drop(job);
    assert!(
        stem.with_extension("state").join("state.ron").exists()
            || std::path::Path::new(&format!("{}.state", stem.display()))
                .join("state.ron")
                .exists(),
        "closing pauses the render and keeps its state"
    );
}

#[test]
#[ignore = "needs GPU"]
fn a_refused_pause_warns_and_the_render_keeps_going() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let dir = Tmp::new("refused");
    let stem = dir.0.join("refused");
    // A foreign file in the state directory makes the engine refuse the save.
    let state = std::path::PathBuf::from(format!("{}.state", stem.display()));
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(state.join("holiday.jpg"), "x").unwrap();
    let mut job = FinalRender::start(
        r.clone(),
        Scene::example(),
        RenderJob {
            out_stem: stem.clone(),
            width: 256,
            height: 256,
            samples: 100_000,
            bounces: 4,
            max_steps: 256,
            pixel_tolerance: 1.0,
            tile_size: 256,
            samples_per_dispatch: 8,
            denoise: false,
            guiding: false,
        },
    )
    .unwrap();
    job.send(Command::Pause);
    let mut warned = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !warned && std::time::Instant::now() < deadline {
        for e in job.poll() {
            match e {
                Event::Warning(_) => warned = true,
                Event::Failed(m) => panic!("a refused pause must not fail the job: {m}"),
                Event::Paused(_) => panic!("the save was refused"),
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(warned, "a warning reports the refused pause");
    assert!(job.is_running(), "the render keeps stepping");
    job.send(Command::Cancel);
    while job.is_running() {
        job.poll();
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "needs GPU"]
fn dropping_a_job_whose_save_is_refused_still_returns_and_spares_the_foreign_file() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let dir = Tmp::new("drop-refused");
    let stem = dir.0.join("dropref");
    let state = std::path::PathBuf::from(format!("{}.state", stem.display()));
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(state.join("holiday.jpg"), "x").unwrap();
    let mut job = FinalRender::start(
        r.clone(),
        Scene::example(),
        RenderJob {
            out_stem: stem.clone(),
            width: 256,
            height: 256,
            samples: 1_000_000,
            bounces: 4,
            max_steps: 256,
            pixel_tolerance: 1.0,
            tile_size: 256,
            samples_per_dispatch: 8,
            denoise: false,
            guiding: false,
        },
    )
    .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while job.last_progress().is_none() && std::time::Instant::now() < deadline {
        job.poll();
        std::thread::sleep(Duration::from_millis(10));
    }
    let t = std::time::Instant::now();
    drop(job);
    assert!(
        t.elapsed() < Duration::from_secs(5),
        "closing must not wait for the render: {:?}",
        t.elapsed()
    );
    assert!(
        state.join("holiday.jpg").exists(),
        "the foreign file survives"
    );
}

#[test]
#[ignore = "needs GPU"]
fn a_finished_job_leaves_a_complete_state() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let dir = Tmp::new("complete");
    let stem = dir.0.join("done");
    let mut job = FinalRender::start(
        r.clone(),
        Scene::example(),
        RenderJob {
            out_stem: stem.clone(),
            width: 32,
            height: 32,
            samples: 4,
            bounces: 2,
            max_steps: 256,
            pixel_tolerance: 1.0,
            tile_size: 256,
            samples_per_dispatch: 8,
            denoise: false,
            guiding: false,
        },
    )
    .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let mut finished = false;
    while !finished && std::time::Instant::now() < deadline {
        finished = job
            .poll()
            .iter()
            .any(|e| matches!(e, Event::Finished { .. }));
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(finished);
    let text = std::fs::read_to_string(dir.0.join("done.state").join("state.ron")).unwrap();
    let st: SessionState = ron::from_str(&text).unwrap();
    assert!(st.tiles.iter().all(|t| t.done >= st.cfg.samples_per_pixel));
}

#[test]
#[ignore = "needs GPU"]
fn cancel_after_a_checkpoint_removes_the_state_it_created() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let dir = Tmp::new("cancel-checkpoint");
    let stem = dir.0.join("job");
    let state_dir = syfrack_app_lib::final_render::state_dir_for(&stem);
    let session = RenderSession::new(
        r.clone(),
        &Scene::example(),
        &RenderConfig::new(256, 256, 100_000),
    )
    .unwrap();
    let (ctx, rx) = mpsc::channel::<Command>();
    let (tx, events) = mpsc::channel::<Event>();
    let s = stem.clone();
    let th = std::thread::spawn(move || {
        syfrack_app_lib::final_render::run_job_with(
            r,
            session,
            s,
            rx,
            tx,
            Duration::from_millis(100),
        )
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !state_dir.join("state.ron").exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        state_dir.join("state.ron").exists(),
        "a checkpoint was written"
    );
    ctx.send(Command::Cancel).unwrap();
    th.join().unwrap();
    assert!(!state_dir.exists(), "cancel removes the state it created");
    assert!(!events.try_iter().any(|e| matches!(e, Event::Failed(_))));
}
