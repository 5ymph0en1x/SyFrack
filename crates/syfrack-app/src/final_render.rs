//! The final render: a second `RenderSession` on the shared device, driven by
//! a background thread. The UI sends `Command`s and polls `Event`s; pausing
//! saves the engine's own state directory, so a render paused here resumes in
//! the CLI (`syfrack resume`) and the other way round.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use syfrack_color::{develop_and_encode, write_exr, write_png};
use syfrack_core::{HdrImage, Scene};
use syfrack_render::{Progress, RenderConfig, RenderSession, Renderer};

/// Time slice of one `step` call; also the reaction time to a command.
const STEP: Duration = Duration::from_millis(250);
/// Delay between two preview readbacks.
const PREVIEW_PERIOD: Duration = Duration::from_secs(10);
/// Delay between two checkpoints (as the CLI `drive`): after a device loss
/// the state cannot be saved any more, the last checkpoint is what survives.
const CHECKPOINT_PERIOD: Duration = Duration::from_secs(60);

pub enum Command {
    /// Save the state and stop; a refused save is reported and the render
    /// keeps going (the Pause button: nothing is lost).
    Pause,
    /// Stop without saving.
    Cancel,
    /// Window closing: save if possible, then exit either way (a refused
    /// save is reported as `Failed`; no output is written).
    Shutdown,
}

pub enum Event {
    Progress(Progress),
    Preview(HdrImage),
    Paused(PathBuf),
    Finished {
        exr: PathBuf,
        png: PathBuf,
        ron: PathBuf,
        elapsed: Duration,
    },
    /// Something went wrong but the job goes on (failed checkpoint or pause).
    Warning(String),
    /// Sent just before `Failed` when the GPU side failed (step, readback,
    /// save): the device is presumed lost, so resuming needs a new process.
    DeviceLost,
    Failed(String),
}

pub struct RenderJob {
    pub out_stem: PathBuf,
    pub width: u32,
    pub height: u32,
    pub samples: u32,
    pub bounces: u32,
    pub max_steps: u32,
    pub pixel_tolerance: f32,
    pub tile_size: u32,
    pub samples_per_dispatch: u32,
    pub denoise: bool,
    pub guiding: bool,
}

pub struct FinalRender {
    tx: Sender<Command>,
    rx: Receiver<Event>,
    handle: Option<JoinHandle<()>>,
    state_dir: PathBuf,
    last: Option<Progress>,
    done: bool,
}

/// `base` with `.ext` appended (stems may contain dots).
pub fn with_suffix(base: &Path, ext: &str) -> PathBuf {
    let mut s = base.as_os_str().to_owned();
    s.push(".");
    s.push(ext);
    PathBuf::from(s)
}

/// The pause/resume directory of a job: `<out_stem>.state`, as the CLI names it.
pub fn state_dir_for(out_stem: &Path) -> PathBuf {
    with_suffix(out_stem, "state")
}

impl FinalRender {
    pub fn start(renderer: Renderer, scene: Scene, job: RenderJob) -> Result<Self> {
        let mut cfg = RenderConfig::new(job.width, job.height, job.samples);
        cfg.max_bounces = job.bounces;
        cfg.max_steps = job.max_steps;
        cfg.pixel_tolerance = job.pixel_tolerance;
        cfg.tile_size = job.tile_size;
        cfg.dispatch_budget_ms = 800;
        cfg.samples_per_dispatch = job.samples_per_dispatch;
        cfg.denoise = job.denoise;
        cfg.guiding = job.guiding;
        let session =
            RenderSession::new(renderer.clone(), &scene, &cfg).context("final render session")?;
        Ok(Self::spawn(renderer, session, job.out_stem))
    }

    pub fn resume(renderer: Renderer, state_dir: PathBuf, out_stem: PathBuf) -> Result<Self> {
        let mut session =
            RenderSession::resume(renderer.clone(), &state_dir).context("resume final render")?;
        session.set_dispatch_budget_ms(800)?;
        Ok(Self::spawn(renderer, session, out_stem))
    }

    fn spawn(renderer: Renderer, session: RenderSession, out_stem: PathBuf) -> Self {
        let (ctx, crx) = mpsc::channel();
        let (etx, erx) = mpsc::channel();
        let state_dir = state_dir_for(&out_stem);
        let handle = std::thread::Builder::new()
            .name("syfrack-final-render".into())
            .spawn(move || run_job(renderer, session, out_stem, crx, etx))
            .expect("spawn render thread");
        Self {
            tx: ctx,
            rx: erx,
            handle: Some(handle),
            state_dir,
            last: None,
            done: false,
        }
    }

    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(cmd);
    }

    /// Drains the channel without blocking. The job is over once the thread
    /// has exited (a cancel sends no event); the exit is read before the
    /// drain, so every event of a finished thread is collected.
    pub fn poll(&mut self) -> Vec<Event> {
        let exited = self.handle.as_ref().is_none_or(|h| h.is_finished());
        let events: Vec<Event> = self.rx.try_iter().collect();
        for e in &events {
            if let Event::Progress(p) = e {
                self.last = Some(p.clone());
            }
        }
        if exited {
            self.done = true;
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
        }
        events
    }

    pub fn is_running(&self) -> bool {
        !self.done
    }

    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    pub fn last_progress(&self) -> Option<&Progress> {
        self.last.as_ref()
    }
}

impl Drop for FinalRender {
    /// Closing the window must neither leave a detached thread rendering nor
    /// throw the render away: ask it to save and exit (whether or not the save
    /// succeeds, so a refused save cannot hold the window hostage) and wait.
    /// The join is bounded by one step (a dispatch is itself bounded by the
    /// engine's DISPATCH_TIMEOUT) plus one save attempt; only the explicit
    /// « Annuler » button sends `Cancel`.
    fn drop(&mut self) {
        if let Some(h) = self.handle.take() {
            let _ = self.tx.send(Command::Shutdown);
            let _ = h.join();
        }
    }
}

/// The render thread. Public so the GPU test can drive it synchronously.
///
/// GPU errors: the thread relies on the errors the engine's `step`, `readback`
/// and `save` return (they drain the device error inbox themselves) and never
/// calls `GpuContext::check()` on its own; the UI thread owns that call for
/// its banner. A lost device surfaces as an error from `step`; the state can
/// then no longer be saved, so a checkpoint is written every minute and the
/// failure message names the last one.
pub fn run_job(
    renderer: Renderer,
    session: RenderSession,
    out_stem: PathBuf,
    rx: Receiver<Command>,
    tx: Sender<Event>,
) {
    run_job_with(renderer, session, out_stem, rx, tx, CHECKPOINT_PERIOD);
}

/// `run_job` with an explicit checkpoint period (tests shorten it).
pub fn run_job_with(
    _renderer: Renderer,
    mut session: RenderSession,
    out_stem: PathBuf,
    rx: Receiver<Command>,
    tx: Sender<Event>,
    checkpoint_period: Duration,
) {
    let state_dir = state_dir_for(&out_stem);
    // A paused state that predates this job is kept on cancel.
    let state_existed = state_dir.exists();
    let started = Instant::now();
    let mut last_preview = Instant::now();
    let mut last_checkpoint = Instant::now();
    // (samples per pixel, elapsed seconds) of the last successful checkpoint.
    let mut checkpointed: Option<(u32, u64)> = None;
    let mut gpu_failure = false;
    let result: Result<()> = (|| {
        if let Some(dir) = out_stem.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)
                    .with_context(|| format!("creating {}", dir.display()))?;
            }
        }
        loop {
            match rx.try_recv() {
                Ok(Command::Pause) => match session.save(&state_dir) {
                    Ok(()) => {
                        let _ = tx.send(Event::Paused(state_dir.clone()));
                        return Ok(());
                    }
                    // Nothing is lost: report and keep rendering.
                    Err(e) => {
                        let _ = tx.send(Event::Warning(syfrack_i18n::tr_format!(
                            "pause impossible, le rendu continue : saving {} : {e:#}",
                            state_dir.display(),
                            e = e
                        )));
                    }
                },
                Ok(Command::Shutdown) => {
                    match session.save(&state_dir) {
                        Ok(()) => {
                            let _ = tx.send(Event::Paused(state_dir.clone()));
                        }
                        Err(e) => {
                            let _ = tx.send(Event::Failed(syfrack_i18n::tr_format!(
                                "état non sauvegardé à la fermeture : saving {} : {e:#}",
                                state_dir.display(),
                                e = e
                            )));
                        }
                    }
                    return Ok(());
                }
                Ok(Command::Cancel) => {
                    // Abandoned: take back the checkpoints this job wrote.
                    if !state_existed
                        && syfrack_render::session::check_replaceable(&state_dir).is_ok()
                    {
                        let _ = std::fs::remove_dir_all(&state_dir);
                    }
                    return Ok(());
                }
                Err(_) => {}
            }
            gpu_failure = true;
            let done = session.step(STEP)?;
            let _ = tx.send(Event::Progress(session.progress()));
            if last_preview.elapsed() >= PREVIEW_PERIOD && !done {
                let out = session.readback()?;
                let _ = tx.send(Event::Preview(out.color));
                last_preview = Instant::now();
            }
            gpu_failure = false;
            if last_checkpoint.elapsed() >= checkpoint_period && !done {
                last_checkpoint = Instant::now();
                match session.save(&state_dir) {
                    Ok(()) => {
                        checkpointed =
                            Some((session.progress().samples_min, started.elapsed().as_secs()));
                    }
                    Err(e) => {
                        let _ = tx.send(Event::Warning(syfrack_i18n::tr_format!(
                            "checkpoint : {e:#}",
                            e = e
                        )));
                    }
                }
            }
            if done {
                break;
            }
        }
        // The same three writes as the CLI's `drive`: the files are identical.
        gpu_failure = true;
        let output = session.readback()?;
        gpu_failure = false;
        let scene = session.scene();
        let ron = scene.to_ron();
        let exr = with_suffix(&out_stem, "exr");
        let png = with_suffix(&out_stem, "png");
        let ron_path = with_suffix(&out_stem, "ron");
        write_exr(&exr, &output.color).with_context(|| format!("writing {}", exr.display()))?;
        let filtered;
        let image = if session.config().denoise
            && session.scene().dimension == syfrack_core::Dimension::ThreeD
        {
            filtered = syfrack_render::denoise::session_output(&session, &output);
            &filtered
        } else {
            &output.color
        };
        let encoded = develop_and_encode(image, &scene.look, 16);
        write_png(&png, &encoded, scene.look.output, Some(&ron))
            .with_context(|| format!("writing {}", png.display()))?;
        std::fs::write(&ron_path, &ron)
            .with_context(|| format!("writing {}", ron_path.display()))?;
        // As the CLI: leave a complete state (min == target) next to the
        // outputs, replacing the periodic checkpoints.
        if let Err(e) = session.save(&state_dir) {
            let _ = tx.send(Event::Warning(syfrack_i18n::tr_format!(
                "état final non sauvegardé : saving {} : {e:#}",
                state_dir.display(),
                e = e
            )));
        }
        let _ = tx.send(Event::Finished {
            exr,
            png,
            ron: ron_path,
            elapsed: started.elapsed(),
        });
        Ok(())
    })();
    if let Err(e) = result {
        // After a device loss this fails too; the checkpoint is what is left.
        let saved = session.save(&state_dir).is_ok();
        let note = if saved {
            syfrack_i18n::tr_format!(" (état sauvegardé dans {})", state_dir.display())
        } else if let Some((spp, secs)) = checkpointed {
            syfrack_i18n::tr_format!(
                " (dernier point de sauvegarde : {spp} éch., à {} de rendu, dans {})",
                crate::ui::render_panel::clock(Duration::from_secs(secs)),
                state_dir.display(),
                spp = spp
            )
        } else {
            String::new()
        };
        if gpu_failure {
            let _ = tx.send(Event::DeviceLost);
        }
        let _ = tx.send(Event::Failed(format!("{e:#}{note}")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffixes_append_instead_of_replacing_an_extension() {
        let stem = PathBuf::from("renders/my.scene");
        assert_eq!(
            with_suffix(&stem, "png"),
            PathBuf::from("renders/my.scene.png")
        );
        assert_eq!(
            state_dir_for(&stem),
            PathBuf::from("renders/my.scene.state")
        );
    }
}
