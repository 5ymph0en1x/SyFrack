use std::time::{Duration, Instant};

use std::sync::Arc;

use anyhow::Result;
use syfrack_core::{Graph, HdrImage, NodeId, Scene};
use syfrack_render::environment::scene_resource_fingerprint;
use syfrack_render::graph_wgsl::fingerprint;
use syfrack_render::{
    CompileStatus, CompileTicket, CompiledPipeline, RenderConfig, RenderSession, Renderer,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Quarter,
    Half,
}

/// Resolution ladder of the interactive preview: quarter until 16 samples, then half.
#[derive(Clone, Debug)]
pub struct PreviewLadder {
    full: (u32, u32),
    stage: Stage,
}

impl PreviewLadder {
    pub const PROMOTE_AT: u32 = 16;

    pub fn new(full: (u32, u32)) -> Self {
        Self {
            full,
            stage: Stage::Quarter,
        }
    }

    pub fn stage(&self) -> Stage {
        self.stage
    }

    pub fn size(&self) -> (u32, u32) {
        let d = match self.stage {
            Stage::Quarter => 4,
            Stage::Half => 2,
        };
        ((self.full.0 / d).max(1), (self.full.1 / d).max(1))
    }

    /// Any scene change: back to quarter. Always requires a rebuild.
    pub fn on_change(&mut self) -> bool {
        self.stage = Stage::Quarter;
        true
    }

    /// Promotes to half when the quarter stage has converged. Returns true on promotion.
    pub fn on_progress(&mut self, spp_min: u32) -> bool {
        if self.stage == Stage::Quarter && spp_min >= Self::PROMOTE_AT {
            self.stage = Stage::Half;
            return true;
        }
        false
    }

    pub fn resize(&mut self, full: (u32, u32)) {
        self.full = full;
        self.stage = Stage::Quarter;
    }
}

#[derive(Clone, Debug)]
pub struct Pick {
    pub depth: f32,
    pub normal: [f32; 3],
    pub steps: f32,
}

/// Box size of the averaging applied before measuring the auto-exposure.
const AUTO_BLOCK: u32 = 8;
/// The auto-exposure is only measured once every tile has this many samples.
const AUTO_MIN_SPP: u32 = 4;

/// Averages linear colour over `block` x `block` boxes (edge boxes may be
/// smaller). Averaging before taking the geometric mean removes the low bias
/// and the dropped black pixels that few-spp noise would otherwise cause.
pub fn block_average(img: &HdrImage, block: u32) -> HdrImage {
    let block = block.max(1);
    let (w, h) = (img.width.div_ceil(block), img.height.div_ceil(block));
    let mut out = HdrImage::new(w, h);
    for by in 0..h {
        for bx in 0..w {
            let mut sum = [0.0f32; 3];
            let mut n = 0.0f32;
            for y in by * block..((by + 1) * block).min(img.height) {
                for x in bx * block..((bx + 1) * block).min(img.width) {
                    let p = img.get(x, y);
                    sum[0] += p[0];
                    sum[1] += p[1];
                    sum[2] += p[2];
                    n += 1.0;
                }
            }
            out.set(bx, by, [sum[0] / n, sum[1] / n, sum[2] / n, 1.0]);
        }
    }
    out
}

/// Auto-exposure EV of a noisy preview image: `auto_ev` on block averages.
pub fn preview_auto_ev(img: &HdrImage, target: f32) -> f32 {
    syfrack_color::exposure::auto_ev(&block_average(img, AUTO_BLOCK), target)
}

/// What a scene change asks of the preview's pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneChange {
    /// Same topology: only the parameter buffer changes.
    Values,
    /// Another topology: a new pipeline must be compiled.
    Topology,
}

/// Values when both graphs share a topology fingerprint, else Topology.
pub fn scene_change(old: &Graph, new: &Graph) -> SceneChange {
    if fingerprint(old) == fingerprint(new) {
        SceneChange::Values
    } else {
        SceneChange::Topology
    }
}

/// What `Preview::set_scene` does with a scene, from the topology the
/// session runs, the one being compiled, the one known to fail and the one
/// wanted (fingerprints).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScenePlan {
    /// Not renderable: no session.
    Inactive,
    /// The running topology: restart it with the new values.
    Restart,
    /// The topology being compiled: it will carry the new values.
    UpdatePending,
    /// The topology last failed to compile: nothing is requested again.
    KnownFailure,
    /// Compile it in the background (the old pipeline, if any, keeps going).
    Request,
}

pub fn plan_scene(
    renderable: bool,
    running: Option<u64>,
    pending: Option<u64>,
    failed: Option<u64>,
    wanted: u64,
) -> ScenePlan {
    if !renderable {
        ScenePlan::Inactive
    } else if running == Some(wanted) {
        ScenePlan::Restart
    } else if pending == Some(wanted) {
        ScenePlan::UpdatePending
    } else if failed == Some(wanted) {
        ScenePlan::KnownFailure
    } else {
        ScenePlan::Request
    }
}

/// Whether a scene change waits for the pointer release: a topology other
/// than the one running or compiling, while a button is held (a ⟳ slider
/// being dragged), would request one compilation per value. The release
/// frame then requests exactly one.
pub fn defer_topology(
    pointer_down: bool,
    wanted: u64,
    running: Option<u64>,
    pending: Option<u64>,
) -> bool {
    pointer_down && running != Some(wanted) && pending != Some(wanted)
}

/// The node a compilation error names (« … near node 7 Plier … »).
pub fn compile_error_node(msg: &str) -> Option<NodeId> {
    let (_, rest) = msg.split_once("near node ")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Why the preview cannot render `scene`, if it cannot: the full graph
/// validation (an editable scene may still have unwired ports).
pub fn inactive_reason_for(scene: &Scene) -> Option<String> {
    scene.validate().err().map(|e| e.to_string())
}

/// Prefix of the banner raised while the preview waits for a complete form.
pub const INCOMPLETE: &str = "forme incomplète";

/// The banner raised while the preview waits for a complete form.
pub fn incomplete_banner(reason: &str) -> String {
    format!("{} : {reason}", syfrack_i18n::text(INCOMPLETE))
}

/// A banner the preview raised itself: the next good build clears it.
pub fn is_preview_banner(text: &str) -> bool {
    text.to_ascii_lowercase().starts_with("preview")
        || [
            syfrack_i18n::Language::English,
            syfrack_i18n::Language::French,
        ]
        .iter()
        .any(|&language| text.starts_with(syfrack_i18n::text_in(language, INCOMPLETE)))
}

/// What a preview (re)build does to the banner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BannerChange {
    Keep,
    Clear,
    Raise(String),
}

/// The banner after a preview build that succeeded: « forme incomplète »
/// while the preview is inactive (not restarted for the same reason), else
/// the preview's own banners go.
pub fn banner_after_build(current: Option<&str>, inactive: Option<&str>) -> BannerChange {
    match inactive {
        Some(reason) => {
            let text = incomplete_banner(reason);
            if current == Some(text.as_str()) {
                BannerChange::Keep
            } else {
                BannerChange::Raise(text)
            }
        }
        None if current.is_some_and(is_preview_banner) => BannerChange::Clear,
        None => BannerChange::Keep,
    }
}

/// The interactive session, restarted at the ladder's quarter stage on every
/// change. A value change keeps the pipeline; a topology change compiles the
/// new one in the background while the old one keeps rendering, then swaps.
/// Inactive (no session) while the scene does not pass the full validation,
/// an incomplete form graph for instance; a later valid scene reactivates it.
pub struct Preview {
    renderer: Renderer,
    session: Option<RenderSession>,
    /// Why there is no session.
    inactive: Option<String>,
    ladder: PreviewLadder,
    /// The scene the session renders (not the one being compiled).
    scene: Scene,
    /// A compilation under way: its ticket, the scene waiting for it, and
    /// when it was requested. A newer topology replaces it.
    pending: Option<(CompileTicket, Scene, Instant)>,
    /// The topology whose compilation failed last, with the error: it is
    /// not requested again while it stays the wanted one.
    failed: Option<(u64, String)>,
    /// The wanted topology is the failed one.
    failing: bool,
    last_compile_ms: Option<u32>,
    auto_ev: f32,
    last_auto: Instant,
    /// Re-measure at the next eligible tick (after a rebuild).
    auto_stale: bool,
}

impl Preview {
    pub fn new(renderer: Renderer, scene: &Scene, full: (u32, u32)) -> Result<Self> {
        let ladder = PreviewLadder::new(full);
        let inactive = inactive_reason_for(scene);
        let asynchronous = inactive.is_none()
            && scene
                .lights
                .iter()
                .any(|l| matches!(l, syfrack_core::Light::Hdri { .. }));
        let pending =
            asynchronous.then(|| (renderer.request_scene(scene), scene.clone(), Instant::now()));
        let session = match inactive {
            None if asynchronous => None,
            None => Some(Self::build(&renderer, scene, ladder.size())?),
            Some(_) => None,
        };
        Ok(Self {
            renderer,
            session,
            inactive,
            ladder,
            scene: scene.clone(),
            pending,
            failed: None,
            failing: false,
            last_compile_ms: None,
            auto_ev: 0.0,
            last_auto: Instant::now(),
            auto_stale: true,
        })
    }

    fn build(renderer: &Renderer, scene: &Scene, size: (u32, u32)) -> Result<RenderSession> {
        let mut cfg = RenderConfig::new(size.0, size.1, u32::MAX / 2);
        cfg.samples_per_dispatch = 1;
        if scene.canvas.flame.is_some() {
            cfg.samples_per_pixel = 128;
        }
        cfg.tile_size = 128;
        cfg.max_bounces = 3;
        RenderSession::new(renderer.clone(), scene, &cfg)
    }

    /// Why the preview shows nothing, while the scene cannot be rendered.
    pub fn inactive_reason(&self) -> Option<&str> {
        self.inactive.as_deref()
    }

    /// A new scene. Inactive (no session) when the full validation refuses
    /// it; restarted on the same pipeline when only values changed;
    /// otherwise its pipeline is compiled in the background (the old one
    /// keeps rendering, if any) until `poll_compile` adopts it. A topology
    /// that failed to compile is not requested again.
    pub fn set_scene(&mut self, scene: &Scene) -> Result<()> {
        // Grading is a display operation. Keep the converged fractal while it changes.
        let mut grading_only = self.scene.clone();
        grading_only.look = scene.look.clone();
        if (self.session.is_some() || self.pending.is_some())
            && !self.failing
            && self.inactive.is_none()
            && grading_only == *scene
            && scene.look.validate().is_ok()
        {
            self.auto_stale |= self.scene.look.auto_exposure != scene.look.auto_exposure;
            self.scene.look = scene.look.clone();
            if let Some((_, wanted, _)) = &mut self.pending {
                wanted.look = scene.look.clone();
            }
            return Ok(());
        }
        let fp = scene_resource_fingerprint(scene);
        let reason = inactive_reason_for(scene);
        if self.failed.as_ref().is_some_and(|(f, _)| *f != fp) {
            self.failed = None;
        }
        let mut plan = plan_scene(
            reason.is_none(),
            self.session.as_ref().map(|s| s.resource_fingerprint()),
            self.pending.as_ref().map(|(t, ..)| t.fingerprint),
            self.failed.as_ref().map(|(f, _)| *f),
            fp,
        );
        if plan == ScenePlan::Restart && !self.renderer.environment_ready(scene) {
            plan = ScenePlan::Request;
        }
        self.failing = plan == ScenePlan::KnownFailure;
        match plan {
            ScenePlan::Inactive => {
                self.scene = scene.clone();
                self.session = None;
                self.pending = None;
                self.inactive = reason;
                self.ladder.on_change();
                Ok(())
            }
            ScenePlan::Restart => {
                // Back on the running topology: a compilation under way is stale.
                self.pending = None;
                self.restart(scene.clone(), None)
            }
            ScenePlan::UpdatePending => {
                if let Some((_, wanted, _)) = &mut self.pending {
                    *wanted = scene.clone();
                }
                Ok(())
            }
            ScenePlan::KnownFailure => {
                self.pending = None;
                Ok(())
            }
            ScenePlan::Request => {
                if self.session.is_none() {
                    // Valid, waiting for its pipeline: no longer incomplete.
                    self.inactive = None;
                }
                self.pending = Some((
                    self.renderer.request_scene(scene),
                    scene.clone(),
                    Instant::now(),
                ));
                Ok(())
            }
        }
    }

    /// Whether `scene` should wait for the pointer release (`defer_topology`).
    pub fn defers(&self, scene: &Scene, pointer_down: bool) -> bool {
        pointer_down
            && defer_topology(
                true,
                scene_resource_fingerprint(scene),
                self.session.as_ref().map(|s| s.resource_fingerprint()),
                self.pending.as_ref().map(|(t, ..)| t.fingerprint),
            )
    }

    /// Checks the compilation under way: `Some(Ok(elapsed))` once its
    /// pipeline is adopted (swapped in, or the first session built),
    /// `Some(Err)` when it failed (the old pipeline stays; without one the
    /// preview stays inactive with the error), `None` while it runs or when
    /// there is none.
    pub fn poll_compile(&mut self) -> Option<Result<Duration, String>> {
        let (ticket, wanted, _) = self.pending.as_ref()?;
        let fp = ticket.fingerprint;
        let pipeline = match Renderer::poll(ticket) {
            CompileStatus::Pending => return None,
            CompileStatus::Failed(e) => {
                self.pending = None;
                return Some(Err(self.fail(fp, e)));
            }
            CompileStatus::Ready(p) => p,
        };
        // Only the topology still wanted is adopted (set_scene keeps the
        // ticket in step with it; this guards that invariant).
        if fp != scene_resource_fingerprint(wanted) {
            self.pending = Some((
                self.renderer.request_scene(wanted),
                wanted.clone(),
                Instant::now(),
            ));
            return None;
        }
        // A stale worker may have replaced the bounded asset cache. Warm the
        // wanted resources off the UI thread before building the session.
        if !self.renderer.environment_ready(wanted) {
            self.pending = Some((
                self.renderer.request_scene(wanted),
                wanted.clone(),
                Instant::now(),
            ));
            return None;
        }
        let (_, scene, since) = self.pending.take().expect("checked above");
        let elapsed = since.elapsed();
        let adopted = if self.session.is_some() {
            self.restart(scene, Some(pipeline))
        } else {
            // First session: its pipeline is in the renderer's cache now.
            self.scene = scene;
            self.rebuild(|l| {
                l.on_change();
            })
        };
        if let Err(e) = adopted {
            return Some(Err(self.fail(fp, format!("{e:#}"))));
        }
        self.failed = None;
        self.failing = false;
        self.last_compile_ms = Some(elapsed.as_millis().min(u32::MAX as u128) as u32);
        Some(Ok(elapsed))
    }

    /// Remembers that topology `fp` failed with `e`; without a session the
    /// preview stays inactive, with the error as its reason.
    fn fail(&mut self, fp: u64, e: String) -> String {
        self.failed = Some((fp, e.clone()));
        self.failing = true;
        if self.session.is_none() {
            self.inactive = Some(e.clone());
        }
        e
    }

    /// The wanted topology failed to compile: its error (the banner that
    /// reported it stays up).
    pub fn known_failure(&self) -> Option<&str> {
        self.failed
            .as_ref()
            .filter(|_| self.failing)
            .map(|(_, e)| e.as_str())
    }

    /// A compilation is under way: the image still shows the old topology.
    pub fn compiling(&self) -> bool {
        self.pending.is_some()
    }

    /// How long the last background compilation took, in ms.
    pub fn last_compile_ms(&self) -> Option<u32> {
        self.last_compile_ms
    }

    /// Restarts the session on `scene` (on `pipeline` when given) at the
    /// quarter stage: in place when already there, else rebuilt smaller
    /// (the pipeline then comes from the renderer's cache).
    fn restart(&mut self, scene: Scene, pipeline: Option<Arc<CompiledPipeline>>) -> Result<()> {
        let quarter = self.ladder.stage() == Stage::Quarter;
        if let (Some(session), true) = (&mut self.session, quarter) {
            match pipeline {
                Some(p) => session.swap_pipeline(p, &scene)?,
                None => {
                    session.set_scene_params(&scene)?;
                }
            }
            self.scene = scene;
            self.ladder.on_change();
            self.auto_stale = true;
            return Ok(());
        }
        let previous = std::mem::replace(&mut self.scene, scene);
        let built = self.rebuild(|l| {
            l.on_change();
        });
        if built.is_err() {
            self.scene = previous;
        }
        built
    }

    pub fn resize(&mut self, full: (u32, u32)) -> Result<()> {
        if full == (0, 0) {
            return Ok(());
        }
        if self.session.is_none() {
            self.ladder.resize(full);
            return Ok(());
        }
        self.rebuild(|l| l.resize(full))
    }

    /// Applies `change` to a copy of the ladder, builds the session at the
    /// resulting size and commits both only on success: a failed build leaves
    /// the old session and its matching ladder.
    fn rebuild(&mut self, change: impl FnOnce(&mut PreviewLadder)) -> Result<()> {
        let mut ladder = self.ladder.clone();
        change(&mut ladder);
        let session = Self::build(&self.renderer, &self.scene, ladder.size())?;
        self.ladder = ladder;
        self.session = Some(session);
        self.inactive = None;
        self.auto_stale = true;
        Ok(())
    }

    /// One slice of work. Promotes the ladder when the quarter stage converged.
    /// Nothing to do while inactive.
    pub fn tick(&mut self, budget: Duration) -> Result<()> {
        let Some(session) = &mut self.session else {
            return Ok(());
        };
        session.step(budget)?;
        let spp = session.progress().samples_min;
        let mut promoted = self.ladder.clone();
        if self.pending.is_none() && promoted.on_progress(spp) {
            self.session = Some(Self::build(&self.renderer, &self.scene, promoted.size())?);
            self.ladder = promoted;
            self.auto_stale = true;
            return Ok(());
        }
        if let Some(target) = self.scene.look.auto_exposure {
            // The full readback also fetches depth and aux buffers; the engine
            // has no colour-only one yet. Rate-limited to once per second.
            if spp >= AUTO_MIN_SPP
                && (self.auto_stale || self.last_auto.elapsed() >= Duration::from_secs(1))
            {
                let out = session.readback()?;
                self.auto_ev = preview_auto_ev(&out.color, target);
                self.last_auto = Instant::now();
                self.auto_stale = false;
            }
        }
        Ok(())
    }

    /// The running session; `None` while inactive.
    pub fn session(&self) -> Option<&RenderSession> {
        self.session.as_ref()
    }

    /// Fewest samples of any tile; 0 while inactive.
    pub fn samples_min(&self) -> u32 {
        self.session
            .as_ref()
            .map_or(0, |s| s.progress().samples_min)
    }

    /// Size of the preview image (the ladder's size while inactive).
    pub fn size(&self) -> (u32, u32) {
        self.session
            .as_ref()
            .map_or_else(|| self.ladder.size(), |s| s.image_size())
    }

    pub fn stage(&self) -> Stage {
        self.ladder.stage()
    }

    pub fn exposure_gain(&self) -> f32 {
        let auto = if self.scene.look.auto_exposure.is_some() {
            self.auto_ev
        } else {
            0.0
        };
        2f32.powf(self.scene.look.exposure_ev + auto)
    }

    /// Index of the pixel under (u, v) in [0, 1]^2, if it lies in a buffer of
    /// `len` pixels at the session's size.
    fn pixel(&self, u: f32, v: f32, len: usize) -> Option<usize> {
        pixel_index(u, v, self.size(), len)
    }

    /// The surface under (u, v) in [0, 1]^2, from a readback of the current
    /// session; none while inactive.
    pub fn pick(&self, u: f32, v: f32) -> Result<Option<Pick>> {
        let Some(session) = &self.session else {
            return Ok(None);
        };
        let out = session.readback()?;
        let Some(i) = self.pixel(u, v, out.depth.len().min(out.normals.len())) else {
            return Ok(None);
        };
        Ok(surface_pick(out.depth[i], out.normals[i]))
    }

    /// Distance to whatever the first sample met under (u, v): a surface or,
    /// in fog, a scattering point. Scales the free-camera speed. None while
    /// inactive.
    pub fn depth(&self, u: f32, v: f32) -> Result<Option<f32>> {
        let Some(session) = &self.session else {
            return Ok(None);
        };
        let out = session.readback()?;
        let Some(i) = self.pixel(u, v, out.depth.len()) else {
            return Ok(None);
        };
        let d = out.depth[i];
        Ok((d > 0.0).then_some(d))
    }
}

/// Index of the pixel under (u, v) in [0, 1]^2 for an image of `size`, or
/// `None` when it falls outside a buffer of `len` pixels.
pub fn pixel_index(u: f32, v: f32, size: (u32, u32), len: usize) -> Option<usize> {
    let (w, h) = size;
    let x = (u.clamp(0.0, 0.9999) * w as f32) as usize;
    let y = (v.clamp(0.0, 0.9999) * h as f32) as usize;
    let i = y * w as usize + x;
    (x < w as usize && y < h as usize && i < len).then_some(i)
}

/// A pick from one pixel of the depth and aux buffers. The engine writes the
/// depth of the first event, which in fog can be a scattering point in the
/// medium: its aux is then the miss value (steps -1, no normal). Only a
/// surface (at least one march step) is a pick.
pub fn surface_pick(depth: f32, aux: [f32; 4]) -> Option<Pick> {
    (depth > 0.0 && aux[3] >= 1.0).then(|| Pick {
        depth,
        normal: [aux[0], aux[1], aux[2]],
        steps: aux[3],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_topology_waits_for_the_pointer_release() {
        // A ⟳ slider dragged through 3, 4, 5 then released on 6: the held
        // values never request a compile; the release requests one.
        for held in [3, 4, 5] {
            assert!(defer_topology(true, held, Some(1), None));
        }
        assert!(!defer_topology(false, 6, Some(1), None), "on release");
        // Values of the running or compiling topology apply live.
        assert!(!defer_topology(true, 1, Some(1), None));
        assert!(!defer_topology(true, 6, Some(1), Some(6)));
        // Nothing to defer to without a button down.
        assert!(!defer_topology(false, 6, None, None));
    }

    fn incomplete() -> Scene {
        let mut scene = Scene::example();
        let mut g = syfrack_core::Graph::new_minimal();
        g.links.retain(|l| l.to.0 != 2); // the Sortie waits for a wire
        scene.fractal = syfrack_core::Fractal::Graph(g);
        scene
    }

    #[test]
    fn a_value_change_keeps_the_pipeline_and_a_topology_change_requests_one() {
        use syfrack_core::{Graph, NodeKind};
        let a = Graph::bundled("mandelbox").unwrap();
        let mut b = a.clone();
        for n in &mut b.nodes {
            if let NodeKind::Scale { factor } = &mut n.kind {
                *factor = 2.4;
            }
        }
        assert_eq!(scene_change(&a, &b), SceneChange::Values);
        let mut c = a.clone();
        c.nodes[2].enabled = false;
        assert_eq!(scene_change(&a, &c), SceneChange::Topology);
    }

    #[test]
    fn a_scene_plan_follows_the_running_pending_and_failed_topologies() {
        use ScenePlan::*;
        let p = |r: bool, run, pend, fail, want| plan_scene(r, run, pend, fail, want);
        assert_eq!(p(false, Some(1), None, None, 1), Inactive);
        assert_eq!(
            p(true, Some(1), Some(2), None, 1),
            Restart,
            "back on the running one"
        );
        assert_eq!(p(true, Some(1), Some(2), None, 2), UpdatePending);
        assert_eq!(p(true, Some(1), None, None, 2), Request);
        // No session yet: compiled in the background too, never on the UI thread.
        assert_eq!(p(true, None, None, None, 2), Request);
        assert_eq!(p(true, None, Some(2), None, 2), UpdatePending);
        // A topology known to fail is not requested again.
        assert_eq!(p(true, Some(1), None, Some(2), 2), KnownFailure);
        assert_eq!(p(true, None, None, Some(2), 2), KnownFailure);
        assert_eq!(
            p(true, Some(1), None, Some(2), 3),
            Request,
            "another one is tried"
        );
        assert_eq!(p(true, Some(2), None, Some(2), 2), Restart, "running wins");
    }

    #[test]
    fn a_compilation_error_names_its_node() {
        assert_eq!(
            compile_error_node("wgsl: bad thing near node 12 Ma boîte"),
            Some(12)
        );
        assert_eq!(compile_error_node("near node 3"), Some(3));
        assert_eq!(compile_error_node("compilation thread died"), None);
        assert_eq!(compile_error_node("near node x"), None);
    }

    #[test]
    fn only_a_renderable_scene_activates_the_preview() {
        assert_eq!(inactive_reason_for(&Scene::example()), None);
        let reason = inactive_reason_for(&incomplete()).expect("an incomplete graph waits");
        assert!(reason.contains("Output"), "{reason}");
        assert!(incomplete().validate_editable().is_ok());
    }

    #[test]
    fn a_good_build_clears_only_the_preview_banners() {
        use BannerChange::*;
        assert_eq!(banner_after_build(Some("preview: x"), None), Clear);
        assert_eq!(
            banner_after_build(Some(&incomplete_banner("a")), None),
            Clear
        );
        assert_eq!(banner_after_build(Some("chargé scène"), None), Keep);
        assert_eq!(banner_after_build(None, None), Keep);
        assert_eq!(
            banner_after_build(None, Some("a")),
            Raise(incomplete_banner("a"))
        );
        assert_eq!(
            banner_after_build(Some(&incomplete_banner("a")), Some("a")),
            Keep,
            "the same reason does not restart the banner"
        );
    }

    #[test]
    fn preview_banners_are_recognised() {
        assert!(is_preview_banner("preview: lost device"));
        assert!(is_preview_banner(&incomplete_banner("x")));
        assert!(!is_preview_banner("chargé scène"));
    }

    #[test]
    fn pixel_index_is_bounds_checked() {
        assert_eq!(pixel_index(0.0, 0.0, (4, 2), 8), Some(0));
        assert_eq!(pixel_index(1.0, 1.0, (4, 2), 8), Some(7));
        assert_eq!(pixel_index(0.5, 0.5, (4, 2), 8), Some(6));
        assert_eq!(pixel_index(2.0, -1.0, (4, 2), 8), Some(3), "clamped");
        // A readback smaller than the size the caller believes in.
        assert_eq!(pixel_index(1.0, 1.0, (4, 2), 4), None);
        assert_eq!(pixel_index(0.5, 0.5, (0, 0), 0), None);
    }

    #[test]
    fn only_a_surface_is_a_pick() {
        let hit = surface_pick(2.5, [0.0, 1.0, 0.0, 17.0]).expect("a surface");
        assert_eq!(
            (hit.depth, hit.normal, hit.steps),
            (2.5, [0.0, 1.0, 0.0], 17.0)
        );
        assert!(surface_pick(-1.0, [0.0, 0.0, 0.0, -1.0]).is_none(), "sky");
        assert!(
            surface_pick(67.0, [0.0, 0.0, 0.0, -1.0]).is_none(),
            "a scattering point in the fog has a depth but no surface"
        );
    }

    #[test]
    fn ladder_starts_at_quarter_and_promotes_at_sixteen() {
        let mut l = PreviewLadder::new((1921, 1080));
        assert_eq!(l.size(), (480, 270));
        assert!(!l.on_progress(15));
        assert!(l.on_progress(16));
        assert_eq!(l.size(), (960, 540));
        assert!(!l.on_progress(1000), "half is the top stage");
        assert!(l.on_change());
        assert_eq!(l.size(), (480, 270));
        l.resize((100, 40));
        assert_eq!(l.size(), (25, 10));
        let mut tiny = PreviewLadder::new((3, 2));
        assert_eq!(tiny.size(), (1, 1));
        assert!(tiny.on_progress(16));
        assert_eq!(tiny.size(), (1, 1));
    }

    fn lcg(state: &mut u32) -> u32 {
        *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *state >> 16
    }

    #[test]
    fn block_average_means_each_box() {
        let mut img = HdrImage::new(16, 16);
        for y in 0..16 {
            for x in 0..16 {
                let v = if x < 8 { 1.0 } else { 3.0 } * if y < 8 { 1.0 } else { 2.0 };
                // Checker of +/- 0.5 inside a box: the mean must be unchanged.
                let d = if (x + y) % 2 == 0 { 0.5 } else { -0.5 };
                img.set(x, y, [v + d, 2.0 * v + d, 0.0, 1.0]);
            }
        }
        let avg = block_average(&img, 8);
        assert_eq!((avg.width, avg.height), (2, 2));
        let expect = [[1.0, 3.0], [2.0, 6.0]];
        for (by, row) in expect.iter().enumerate() {
            for (bx, v) in row.iter().enumerate() {
                let p = avg.get(bx as u32, by as u32);
                assert!((p[0] - v).abs() < 1e-5, "r {bx},{by}: {}", p[0]);
                assert!((p[1] - 2.0 * v).abs() < 1e-5, "g {bx},{by}: {}", p[1]);
            }
        }
        // Ragged edges: 5x3 with block 4 gives 2x1.
        assert_eq!(block_average(&HdrImage::new(5, 3), 4).width, 2);
    }

    #[test]
    fn auto_ev_on_noise_matches_auto_ev_on_its_mean() {
        // Per-pixel 0 or 2 with mean 1: raw auto_ev drops the zeros and is
        // biased; the block-averaged measurement must agree with the mean.
        let mut state = 12345u32;
        let mut noisy = HdrImage::new(128, 128);
        for y in 0..128 {
            for x in 0..128 {
                let v = if lcg(&mut state) & 1 == 0 { 0.0 } else { 2.0 };
                noisy.set(x, y, [v, v, v, 1.0]);
            }
        }
        let mut flat = HdrImage::new(128, 128);
        flat.map_rgb(|_| [1.0, 1.0, 1.0]);
        let gain = |ev: f32| 2f32.powf(ev);
        let reference = gain(syfrack_color::exposure::auto_ev(&flat, 0.18));
        let measured = gain(preview_auto_ev(&noisy, 0.18));
        let raw = gain(syfrack_color::exposure::auto_ev(&noisy, 0.18));
        assert!(
            (measured / reference - 1.0).abs() < 0.10,
            "block-averaged gain {measured} vs reference {reference}"
        );
        assert!(
            (raw / reference - 1.0).abs() > 0.4,
            "raw gain {raw} should be visibly biased"
        );
    }
}
