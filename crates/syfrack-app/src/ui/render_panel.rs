//! The « Rendre » window: output settings, then progress of the background
//! final render (`crate::final_render`). Drawing is split from acting: the
//! window returns an `Action`, applied right after, so the layout is testable
//! without a GPU.

use std::path::{Path, PathBuf};
use std::time::Duration;

use egui::{Color32, RichText, Sense, Ui};
use syfrack_color::develop_and_encode;
use syfrack_core::{Look, Scene};
use syfrack_render::{Progress, Renderer};

use crate::final_render::{state_dir_for, with_suffix, Command, Event, FinalRender, RenderJob};
use crate::preview::block_average;
use crate::state::{AppState, Banner};
use crate::ui::theme;
use crate::ui::widgets::ROW_H;

mod layout;

/// Output size presets of the window.
const PRESETS: [(&str, u32, u32); 4] = [
    ("1080p", 1920, 1080),
    ("2K", 2048, 1152),
    ("4K", 3840, 2160),
    ("8K", 7680, 4320),
];
/// Longest side of the thumbnail texture before it is averaged down.
const THUMB_SIDE: u32 = 480;
/// Gap between two cells of the tile grid, in points.
const CELL_GAP: f32 = 1.0;
/// Width of the window's content.
const PANEL_W: f32 = 940.0;

const QUALITY: [(&str, u32, u32); 3] = [
    ("Étude", 48, 2),
    ("Production", 128, 4),
    ("Haute qualité", 512, 6),
];

/// Opens (or closes) the window; set by the title bar button.
/// A « Reprendre » asked from the gallery: it travels through egui memory and
/// the panel, which owns the only render job, takes it up (or refuses).
#[derive(Clone)]
pub struct ResumeRequest {
    pub stem: PathBuf,
    pub state_dir: PathBuf,
    /// The look the thumbnail is developed with, when the scene is known.
    pub look: Option<Look>,
}

pub fn resume_request_id() -> egui::Id {
    egui::Id::new("render-resume-request")
}

pub fn toggle_id() -> egui::Id {
    egui::Id::new("render-panel-toggle")
}

pub fn activity_id() -> egui::Id {
    egui::Id::new("render-activity")
}

#[derive(Clone, Debug, PartialEq)]
enum Status {
    Idle,
    Running,
    Pausing,
    Paused,
    Cancelled,
    Done(PathBuf),
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct FlameQuality {
    supersampling: u32,
    points_per_sample: u32,
    filter_radius: f32,
}

impl FlameQuality {
    fn from_scene(scene: &Scene) -> Option<Self> {
        if scene.dimension != syfrack_core::Dimension::TwoD {
            return None;
        }
        scene.canvas.flame.as_ref().map(|f| Self {
            supersampling: f.supersampling,
            points_per_sample: f.points_per_sample,
            filter_radius: f.filter_radius,
        })
    }
}

struct Settings {
    width: u32,
    height: u32,
    samples: u32,
    bounces: u32,
    max_steps: u32,
    pixel_tolerance: f32,
    tile_size: u32,
    samples_per_dispatch: u32,
    dimension: syfrack_core::Dimension,
    flame: Option<FlameQuality>,
    denoise: bool,
    guiding: bool,
    dir: String,
    /// Empty: the default name (scene file stem, or `studio`).
    name: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            samples: 128,
            bounces: 4,
            max_steps: 512,
            pixel_tolerance: 1.0,
            tile_size: 256,
            samples_per_dispatch: 8,
            dimension: syfrack_core::Dimension::ThreeD,
            flame: None,
            denoise: true,
            guiding: false,
            dir: crate::paths::content_root()
                .join("renders")
                .display()
                .to_string(),
            name: String::new(),
        }
    }
}

impl Settings {
    /// Quality overrides belong to the exported snapshot, never to the editor.
    fn render_scene(&self, scene: &Scene) -> Scene {
        let mut snapshot = scene.clone();
        if snapshot.dimension == syfrack_core::Dimension::TwoD {
            if let (Some(quality), Some(flame)) = (self.flame, snapshot.canvas.flame.as_mut()) {
                flame.supersampling = quality.supersampling;
                flame.points_per_sample = quality.points_per_sample;
                flame.filter_radius = quality.filter_radius;
            }
        }
        snapshot
    }

    fn job(&self, out_stem: PathBuf) -> RenderJob {
        RenderJob {
            out_stem,
            width: self.width,
            height: self.height,
            samples: self.samples,
            bounces: self.bounces,
            max_steps: self.max_steps,
            pixel_tolerance: self.pixel_tolerance,
            tile_size: self.tile_size,
            samples_per_dispatch: self.samples_per_dispatch,
            denoise: self.denoise,
            guiding: self.guiding,
        }
    }

    fn flame_cells(&self) -> Option<u64> {
        self.flame.map(|f| {
            u64::from(self.width) * u64::from(self.height) * u64::from(f.supersampling).pow(2)
        })
    }

    fn flame_limit_exceeded(&self) -> bool {
        // The CPU histogram is bounded to 512 MiB by FlameAccumulator::new.
        self.flame_cells().is_some_and(|cells| cells > 16_777_216)
    }
}

/// What the window asks for this frame.
#[derive(Debug, PartialEq)]
enum Action {
    None,
    Start,
    Resume,
    Pause,
    Cancel,
}

pub struct RenderPanel {
    open: bool,
    settings: Settings,
    /// Last editor quality imported; keep subsequent export overrides intact.
    flame_source: Option<FlameQuality>,
    job: Option<FinalRender>,
    status: Status,
    progress: Option<Progress>,
    /// Output stem of the current or last job.
    stem: Option<PathBuf>,
    /// Look the thumbnail is developed with: the scene's at the start.
    look: Look,
    thumb: Option<egui::TextureHandle>,
    /// The GPU side failed: this process cannot resume (dead device).
    device_lost: bool,
    /// A size field is being edited or hovered: the viewport shows the frame.
    size_active: bool,
}

impl Default for RenderPanel {
    fn default() -> Self {
        Self::new()
    }
}

/// The default job name: the scene file's stem, or `studio`.
pub fn default_name(scene_path: Option<&Path>) -> String {
    scene_path
        .and_then(|p| p.file_stem())
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "studio".into())
}

/// `<dir>/<name>`: the stem of the three outputs and of the `.state`.
pub fn out_stem(dir: &str, name: &str) -> PathBuf {
    Path::new(dir.trim()).join(name.trim())
}

/// True when the saved state has every tile at its target.
fn is_complete(state_file: &Path) -> bool {
    std::fs::read_to_string(state_file)
        .ok()
        .and_then(|t| ron::from_str::<syfrack_render::SessionState>(&t).ok())
        .is_some_and(|st| st.tiles.iter().all(|t| t.done >= st.cfg.samples_per_pixel))
}

/// Why a start is refused.
#[derive(Debug, PartialEq)]
pub struct Blocker {
    pub message: String,
    /// The directory holds a paused render of ours: « Reprendre » can take it.
    pub resumable: bool,
}

/// A finished render already carries this name: never overwrite it silently.
fn already_rendered(stem: &Path) -> Blocker {
    Blocker {
        message: syfrack_i18n::tr_format!("« {} » est déjà rendu : choisir un nouveau nom (ou un autre dossier) pour ne pas l'écraser",
            stem.file_name().unwrap_or_default().to_string_lossy()),
        resumable: false,
    }
}

/// A new render must not run over a paused one (its `.state` would be
/// overwritten by the next pause), nor into a foreign non-empty directory:
/// the CLI's `check_state_dir_free` rule.
pub fn start_blocker(stem: &Path) -> Option<Blocker> {
    let dir = state_dir_for(stem);
    let state_file = dir.join("state.ron");
    if state_file.exists() {
        if is_complete(&state_file) {
            return Some(already_rendered(stem));
        }
        return Some(Blocker {
            message: syfrack_i18n::tr_format!(
                "{} contient déjà un rendu en pause : le reprendre, ou changer le nom",
                dir.display()
            ),
            resumable: true,
        });
    }
    if ["png", "exr"].iter().any(|e| with_suffix(stem, e).exists()) {
        return Some(already_rendered(stem));
    }
    let non_empty = std::fs::read_dir(&dir).is_ok_and(|mut d| d.next().is_some());
    non_empty.then(|| Blocker {
        message: syfrack_i18n::tr_format!(
            "{} n'est pas vide et n'est pas un état SyFrack : changer le nom ou le dossier",
            dir.display()
        ),
        resumable: false,
    })
}

/// Time left at the pace so far: `elapsed * (target - done) / done`.
pub fn remaining(elapsed: Duration, samples_min: u32, target: u32) -> Duration {
    let left = target.saturating_sub(samples_min) as f64;
    elapsed.mul_f64(left / samples_min.max(1) as f64)
}

/// « 2 h 05 », « 3 min 07 s », « 12 s ».
pub fn clock(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{} h {:02}", s / 3600, s % 3600 / 60)
    } else if s >= 60 {
        format!("{} min {:02} s", s / 60, s % 60)
    } else {
        format!("{s} s")
    }
}

/// Colour of a tile cell: `SEPARATOR` when empty, `ACCENT` when complete.
pub fn tile_color(done: u32, target: u32) -> Color32 {
    let t = (done as f32 / target.max(1) as f32).clamp(0.0, 1.0);
    theme::SEPARATOR.lerp_to_gamma(theme::ACCENT, t)
}

impl RenderPanel {
    pub fn new() -> Self {
        Self {
            open: false,
            settings: Settings::default(),
            flame_source: None,
            job: None,
            status: Status::Idle,
            progress: None,
            stem: None,
            look: Scene::example().look,
            thumb: None,
            device_lost: false,
            size_active: false,
        }
    }

    /// A final render is running (the interactive preview then slows down).
    pub fn is_running(&self) -> bool {
        self.job.as_ref().is_some_and(|j| j.is_running())
    }

    /// The output size to mark over the preview while the size is edited.
    pub fn frame_guide(&self) -> Option<(u32, u32)> {
        (self.open && self.size_active).then_some((self.settings.width, self.settings.height))
    }

    /// Pauses the job (state saved) and waits for its thread (window closing).
    pub fn shutdown(&mut self) {
        self.job = None;
    }

    /// Polls the job, draws the window, applies what it asked for.
    pub fn show(&mut self, ui: &mut Ui, state: &mut AppState, renderer: &Renderer) {
        if ui
            .data_mut(|d| d.remove_temp::<bool>(toggle_id()))
            .unwrap_or(false)
        {
            self.open = !self.open;
        }
        self.poll(ui, state);
        if let Some(req) = ui.data_mut(|d| {
            let req = d.get_temp::<ResumeRequest>(resume_request_id());
            d.remove::<ResumeRequest>(resume_request_id());
            req
        }) {
            self.resume_from_gallery(req, state, renderer);
        }
        let action = self.window(ui, state);
        self.apply(action, state, renderer);
        let activity = match self.status {
            Status::Running | Status::Pausing => self
                .progress
                .as_ref()
                .map(|p| {
                    syfrack_i18n::tr_format!(
                        "Rendu · {} / {} échantillons",
                        p.samples_min,
                        p.target
                    )
                })
                .or_else(|| Some(syfrack_i18n::tr!("Rendu · préparation…").into())),
            Status::Paused => Some(syfrack_i18n::tr!("Rendu en pause · reprendre").into()),
            _ => None,
        };
        ui.data_mut(|d| d.insert_temp(activity_id(), activity));
    }

    /// Takes over a paused render picked in the gallery: one job at a time.
    /// On success the window opens on the Atelier.
    fn resume_from_gallery(
        &mut self,
        req: ResumeRequest,
        state: &mut AppState,
        renderer: &Renderer,
    ) {
        if self.job.is_some() {
            state.banner = Some(Banner::error(
                syfrack_i18n::tr!("un rendu est déjà en cours : le mettre en pause ou l'annuler avant d'en reprendre un autre"),
            ));
            return;
        }
        match FinalRender::resume(renderer.clone(), req.state_dir, req.stem.clone()) {
            Ok(j) => {
                self.job = Some(j);
                self.restore_checkpoint_settings(&req.stem);
                self.stem = Some(req.stem);
                if let Some(look) = req.look {
                    self.look = look;
                }
                self.progress = None;
                self.thumb = None;
                self.device_lost = false;
                self.status = Status::Running;
                self.open = true;
                state.tab = crate::state::Tab::Atelier;
            }
            Err(e) => state.banner = Some(Banner::error(format!("{e:#}"))),
        }
    }

    fn fail(&mut self, state: &mut AppState, msg: String) {
        state.banner = Some(Banner::error(msg.clone()));
        self.status = Status::Failed(msg);
    }

    fn poll(&mut self, ui: &Ui, state: &mut AppState) {
        let Some(job) = self.job.as_mut() else {
            return;
        };
        let events = job.poll();
        if let Some(p) = job.last_progress() {
            self.progress = Some(p.clone());
        }
        let over = !job.is_running();
        for e in events {
            match e {
                Event::Progress(_) => {}
                Event::Preview(img) => self.set_thumb(ui.ctx(), &img),
                Event::Paused(_) => self.status = Status::Paused,
                Event::Finished { png, .. } => {
                    state.banner = Some(Banner::info(syfrack_i18n::tr_format!(
                        "rendu écrit : {}",
                        png.display()
                    )));
                    self.status = Status::Done(png);
                }
                Event::Warning(msg) => {
                    state.banner = Some(Banner::error(msg));
                    // A refused pause leaves the render running.
                    if self.status == Status::Pausing {
                        self.status = Status::Running;
                    }
                }
                Event::DeviceLost => self.device_lost = true,
                Event::Failed(msg) => self.fail(state, msg),
            }
        }
        if over {
            // A cancel ends the thread without any event.
            if matches!(self.status, Status::Running | Status::Pausing) {
                self.status = Status::Cancelled;
            }
            self.job = None;
        }
    }

    fn set_thumb(&mut self, ctx: &egui::Context, img: &syfrack_core::HdrImage) {
        let block = img.width.max(img.height).div_ceil(THUMB_SIDE);
        let small = if block > 1 {
            block_average(img, block)
        } else {
            img.clone()
        };
        let enc = develop_and_encode(&small, &self.look, 8);
        let bytes: Vec<u8> = enc.data.iter().map(|&v| v as u8).collect();
        let image = egui::ColorImage::from_rgb([enc.width as usize, enc.height as usize], &bytes);
        match &mut self.thumb {
            Some(t) => t.set(image, egui::TextureOptions::LINEAR),
            None => {
                self.thumb =
                    Some(ctx.load_texture("render-thumb", image, egui::TextureOptions::LINEAR));
            }
        }
    }

    fn apply(&mut self, action: Action, state: &mut AppState, renderer: &Renderer) {
        match action {
            Action::None => {}
            Action::Start => {
                if self.refuse_start_if_blocked(state) {
                    return;
                }
                let stem = self.stem_for(state);
                let job = self.settings.job(stem.clone());
                let snapshot = self.settings.render_scene(&state.scene);
                match FinalRender::start(renderer.clone(), snapshot, job) {
                    Ok(j) => {
                        self.job = Some(j);
                        self.stem = Some(stem);
                        self.look = state.scene.look.clone();
                        self.progress = None;
                        self.thumb = None;
                        self.device_lost = false;
                        self.status = Status::Running;
                    }
                    Err(e) => self.fail(state, format!("{e:#}")),
                }
            }
            Action::Resume => {
                let Some(stem) = self.stem.clone() else {
                    return;
                };
                match FinalRender::resume(renderer.clone(), state_dir_for(&stem), stem.clone()) {
                    Ok(j) => {
                        self.restore_checkpoint_settings(&stem);
                        self.job = Some(j);
                        self.status = Status::Running;
                        self.device_lost = false;
                    }
                    Err(e) => self.fail(state, format!("{e:#}")),
                }
            }
            Action::Pause => {
                if let Some(j) = &self.job {
                    j.send(Command::Pause);
                    self.status = Status::Pausing;
                }
            }
            Action::Cancel => {
                if let Some(j) = &self.job {
                    j.send(Command::Cancel);
                }
            }
        }
    }

    fn stem_for(&self, state: &AppState) -> PathBuf {
        let name = if self.settings.name.trim().is_empty() {
            default_name(state.path.as_deref())
        } else {
            self.settings.name.trim().to_string()
        };
        out_stem(&self.settings.dir, &name)
    }

    /// Raises the refusal of a start that would run over a state directory.
    /// A paused state of ours becomes the one « Reprendre » takes.
    fn refuse_start_if_blocked(&mut self, state: &mut AppState) -> bool {
        let stem = self.stem_for(state);
        let Some(b) = start_blocker(&stem) else {
            return false;
        };
        if b.resumable {
            self.stem = Some(stem);
            self.device_lost = false;
        }
        self.fail(state, b.message);
        true
    }

    fn can_resume(&self) -> bool {
        !self.device_lost
            && matches!(
                self.status,
                Status::Paused | Status::Failed(_) | Status::Cancelled | Status::Idle
            )
            && self.job.is_none()
            && self
                .stem
                .as_deref()
                .is_some_and(|s| state_dir_for(s).join("state.ron").exists())
    }

    /// The resumed image is defined by its checkpoint, not by the current
    /// document or by settings edited while the job was paused.
    fn restore_checkpoint_settings(&mut self, stem: &Path) {
        let Some(saved) = std::fs::read_to_string(state_dir_for(stem).join("state.ron"))
            .ok()
            .and_then(|text| ron::from_str::<syfrack_render::SessionState>(&text).ok())
        else {
            return;
        };
        let c = saved.cfg;
        self.settings = Settings {
            width: c.width,
            height: c.height,
            samples: c.samples_per_pixel,
            bounces: c.max_bounces,
            max_steps: c.max_steps,
            pixel_tolerance: c.pixel_tolerance,
            tile_size: c.tile_size,
            samples_per_dispatch: c.samples_per_dispatch,
            dimension: saved.scene.dimension,
            flame: FlameQuality::from_scene(&saved.scene),
            denoise: c.denoise,
            guiding: c.guiding,
            dir: stem
                .parent()
                .unwrap_or(Path::new("."))
                .display()
                .to_string(),
            name: stem
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        };
    }

    fn sync_scene_quality(&mut self, scene: &Scene) {
        if self.job.is_some() || self.can_resume() {
            return;
        }
        let source = FlameQuality::from_scene(scene);
        if source != self.flame_source || scene.dimension != self.settings.dimension {
            self.flame_source = source;
            self.settings.flame = source;
            self.settings.dimension = scene.dimension;
        }
    }

    fn status_line(&self, ui: &mut Ui) {
        let (text, color) = match &self.status {
            Status::Idle => return,
            Status::Running => (
                syfrack_i18n::tr!("Calcul de l’image").to_string(),
                theme::TEXT,
            ),
            Status::Pausing => (
                syfrack_i18n::tr!("Enregistrement de la progression…").to_string(),
                theme::TEXT_DIM,
            ),
            Status::Paused => (
                syfrack_i18n::tr!("Progression enregistrée. Le rendu peut être repris.").into(),
                theme::TEXT_DIM,
            ),
            Status::Cancelled => (
                syfrack_i18n::tr!("Calcul interrompu.").to_string(),
                theme::TEXT_DIM,
            ),
            Status::Done(png) => (
                syfrack_i18n::tr_format!(
                    "Image enregistrée · {}",
                    png.file_name().unwrap_or_default().to_string_lossy()
                ),
                theme::ACCENT,
            ),
            Status::Failed(msg) if self.device_lost => (
                syfrack_i18n::tr_format!(
                    "{msg}\npériphérique perdu — relancez l'application puis Reprendre",
                    msg = msg
                ),
                theme::ERROR,
            ),
            Status::Failed(msg) => (msg.clone(), theme::ERROR),
        };
        ui.add_space(theme::GRID / 2.0);
        ui.add(egui::Label::new(RichText::new(text).color(color)).wrap());
    }

    fn progress_block(&self, ui: &mut Ui) {
        let Some(p) = &self.progress else {
            return;
        };
        ui.add_space(theme::GRID);
        let frac = p.samples_min as f32 / p.target.max(1) as f32;
        ui.add(egui::ProgressBar::new(frac.clamp(0.0, 1.0)).desired_width(ui.available_width()));
        ui.label(
            RichText::new(syfrack_i18n::tr_format!(
                "{:.0} % · {} / {} échantillons",
                frac * 100.0,
                p.samples_min,
                p.target
            ))
            .small(),
        );
        let eta = remaining(p.elapsed, p.samples_min, p.target);
        let stats = syfrack_i18n::tr_format!(
            "Écoulé : {} · Restant estimé : {}",
            clock(p.elapsed),
            if p.samples_min == 0 {
                "—".to_string()
            } else {
                clock(eta)
            }
        );
        ui.add(egui::Label::new(RichText::new(stats).color(theme::TEXT_DIM).small()).wrap());
        ui.add_space(theme::GRID / 2.0);
        egui::CollapsingHeader::new(syfrack_i18n::tr!("Détail du calcul"))
            .id_salt("render-diagnostics")
            .show(ui, |ui| {
                ui.label(
                    RichText::new(syfrack_i18n::tr_format!(
                        "{} tuiles · {} passes GPU · max {} ms",
                        p.tiles,
                        p.dispatches,
                        p.max_dispatch_ms
                    ))
                    .small()
                    .color(theme::TEXT_DIM),
                );
                self.tile_grid(ui, p);
            });
        if let Some(t) = &self.thumb {
            ui.add_space(theme::GRID);
            let size = t.size_vec2();
            let w = ui.available_width().min(size.x);
            let scale = w / size.x.max(1.0);
            ui.add(egui::Image::new((t.id(), size * scale)));
        }
    }

    fn tile_grid(&self, ui: &mut Ui, p: &Progress) {
        let (cols, rows) = p.tile_grid;
        if cols == 0 || rows == 0 || p.tiles_done.len() != (cols * rows) as usize {
            return;
        }
        let aspect = self.settings.width as f32 / self.settings.height.max(1) as f32;
        let w = ui.available_width();
        let h = (w / aspect).clamp(ROW_H * 2.0, 20.0 * theme::GRID);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), Sense::hover());
        let painter = ui.painter_at(rect);
        let cell = egui::vec2(w / cols as f32, h / rows as f32);
        for (i, done) in p.tiles_done.iter().enumerate() {
            let (cx, cy) = (i as u32 % cols, i as u32 / cols);
            let min = rect.min + egui::vec2(cx as f32 * cell.x, cy as f32 * cell.y);
            let r = egui::Rect::from_min_size(min, cell).shrink(CELL_GAP / 2.0);
            painter.rect_filled(r, 1.0, tile_color(*done, p.target));
        }
    }
}

/// One row: label left, widgets right of it, on the panel's 8 px grid.
fn row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    crate::ui::panels::labeled(ui, label, add);
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Pos2, RawInput, Rect};

    #[test]
    fn remaining_follows_the_pace_so_far() {
        let eta = remaining(Duration::from_secs(30), 10, 100);
        assert_eq!(eta, Duration::from_secs(270));
        assert_eq!(
            remaining(Duration::from_secs(5), 0, 8),
            Duration::from_secs(40)
        );
        assert_eq!(remaining(Duration::from_secs(5), 8, 8), Duration::ZERO);
    }

    #[test]
    fn clock_picks_the_readable_unit() {
        assert_eq!(clock(Duration::from_secs(12)), "12 s");
        assert_eq!(clock(Duration::from_secs(187)), "3 min 07 s");
        assert_eq!(clock(Duration::from_secs(7500)), "2 h 05");
    }

    #[test]
    fn tile_colour_runs_from_separator_to_accent() {
        assert_eq!(tile_color(0, 64), theme::SEPARATOR);
        assert_eq!(tile_color(64, 64), theme::ACCENT);
        assert_eq!(tile_color(900, 64), theme::ACCENT, "clamped");
    }

    #[test]
    fn the_default_name_is_the_scene_stem_or_studio() {
        assert_eq!(
            default_name(Some(Path::new("scenes/exhibition.ron"))),
            "exhibition"
        );
        assert_eq!(default_name(None), "studio");
        assert_eq!(out_stem(" renders ", " a "), Path::new("renders").join("a"));
    }

    #[test]
    fn resumed_summary_uses_the_checkpoint_dimensions_quality_and_destination() {
        let dir =
            std::env::temp_dir().join(format!("syfrack-resume-summary-{}", std::process::id()));
        let stem = dir.join("portrait");
        let mut cfg = syfrack_render::RenderConfig::new(800, 1200, 512);
        cfg.max_bounces = 6;
        cfg.max_steps = 1024;
        cfg.pixel_tolerance = 0.125;
        cfg.denoise = false;
        cfg.guiding = true;
        cfg.tile_size = 64;
        cfg.samples_per_dispatch = 2;
        let saved = syfrack_render::SessionState {
            shader_fingerprint: None,
            environment_fingerprint: 0,
            particle_fingerprint: 0,
            guiding_updates: 0,
            guiding_fingerprint: None,
            guiding_version: 0,
            scene: Scene::example(),
            cfg,
            tiles: vec![],
            tile_size: 256,
            spd: 1,
            dispatches: 0,
            elapsed_secs: 0.0,
        };
        std::fs::create_dir_all(state_dir_for(&stem)).unwrap();
        std::fs::write(
            state_dir_for(&stem).join("state.ron"),
            ron::to_string(&saved).unwrap(),
        )
        .unwrap();
        let mut panel = RenderPanel::new();
        panel.restore_checkpoint_settings(&stem);
        let s = &panel.settings;
        assert_eq!(
            (s.width, s.height, s.samples, s.bounces, s.max_steps),
            (800, 1200, 512, 6, 1024)
        );
        assert_eq!(s.pixel_tolerance, 0.125);
        assert_eq!(s.tile_size, 64);
        assert_eq!(s.samples_per_dispatch, 2);
        assert!(!s.denoise);
        assert!(s.guiding);
        assert_eq!(
            panel.stem_for(&AppState::new(
                Scene::example(),
                Some("different.ron".into())
            )),
            stem
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn custom_flame_quality_reaches_the_export_without_editing_the_document() {
        let scene = Scene::example_for("flame").unwrap();
        let original = scene.clone();
        let mut panel = RenderPanel::new();
        panel.sync_scene_quality(&scene);
        panel.settings.flame = Some(FlameQuality {
            supersampling: 3,
            points_per_sample: 16,
            filter_radius: 0.7,
        });
        panel.settings.samples = 512;
        panel.settings.bounces = 12;
        panel.settings.tile_size = 64;
        panel.settings.samples_per_dispatch = 2;
        panel.sync_scene_quality(&scene);
        let snapshot = panel.settings.render_scene(&scene);
        snapshot.validate().unwrap();
        let f = snapshot.canvas.flame.as_ref().unwrap();
        assert_eq!(
            (f.supersampling, f.points_per_sample, f.filter_radius),
            (3, 16, 0.7)
        );
        assert_eq!(scene, original);
        let job = panel.settings.job("renders/test".into());
        assert_eq!(
            (
                job.samples,
                job.bounces,
                job.tile_size,
                job.samples_per_dispatch
            ),
            (512, 12, 64, 2)
        );
        panel.settings.width = 2048;
        panel.settings.height = 1152;
        assert!(
            panel.settings.flame_limit_exceeded(),
            "2K at ×3 exceeds the histogram capacity"
        );
        panel.settings.flame.as_mut().unwrap().supersampling = 2;
        assert!(!panel.settings.flame_limit_exceeded());
        panel.sync_scene_quality(&Scene::example());
        assert!(
            panel.settings.flame.is_none(),
            "3D must not inherit the flame override"
        );
    }

    #[test]
    fn a_paused_render_blocks_a_new_start_with_the_same_name() {
        let dir = std::env::temp_dir().join(format!("syfrack-panel-{}", std::process::id()));
        let stem = dir.join("job");
        assert!(start_blocker(&stem).is_none());
        let state = state_dir_for(&stem);
        std::fs::create_dir_all(&state).unwrap();
        assert!(start_blocker(&stem).is_none(), "an empty directory is free");
        std::fs::write(state.join("state.ron"), "x").unwrap();
        let b = start_blocker(&stem).expect("blocked");
        assert!(b.resumable);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_complete_state_or_existing_outputs_block_a_new_start() {
        let dir = std::env::temp_dir().join(format!("syfrack-panel-done-{}", std::process::id()));
        let stem = dir.join("job");
        let state = state_dir_for(&stem);
        std::fs::create_dir_all(&state).unwrap();
        let write = |done: u32| {
            let st = syfrack_render::SessionState {
                shader_fingerprint: None,
                environment_fingerprint: 0,
                particle_fingerprint: 0,
                guiding_updates: 0,
                guiding_fingerprint: None,
                guiding_version: 0,
                scene: Scene::example(),
                cfg: syfrack_render::RenderConfig::new(16, 16, 4),
                tiles: vec![syfrack_render::Tile {
                    x: 0,
                    y: 0,
                    w: 16,
                    h: 16,
                    done,
                }],
                tile_size: 256,
                spd: 1,
                dispatches: 1,
                elapsed_secs: 1.0,
            };
            std::fs::write(state.join("state.ron"), ron::to_string(&st).unwrap()).unwrap();
        };
        write(2);
        assert!(
            start_blocker(&stem).is_some_and(|b| b.resumable),
            "half done"
        );
        write(4);
        let b = start_blocker(&stem).expect("finished: must not be overwritten");
        assert!(!b.resumable);
        assert!(b.message.contains("already been rendered"), "{}", b.message);
        std::fs::remove_dir_all(&dir).ok();
        // A free stem passes; a leftover png alone blocks too.
        std::fs::create_dir_all(&dir).unwrap();
        assert!(start_blocker(&stem).is_none());
        std::fs::write(with_suffix(&stem, "png"), "x").unwrap();
        assert!(start_blocker(&stem).is_some_and(|b| b.message.contains("already been rendered")));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_foreign_non_empty_state_directory_refuses_the_start_and_is_not_resumable() {
        let dir =
            std::env::temp_dir().join(format!("syfrack-panel-foreign-{}", std::process::id()));
        let stem = dir.join("job");
        let state = state_dir_for(&stem);
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(state.join("holiday.jpg"), "x").unwrap();
        let b = start_blocker(&stem).expect("blocked");
        assert!(!b.resumable);
        assert!(b.message.contains("not empty"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_refused_start_over_a_paused_state_makes_that_state_resumable() {
        // Another stem is held from an earlier job: the blocked one replaces it.
        let dir = std::env::temp_dir().join(format!("syfrack-panel-resume-{}", std::process::id()));
        let stem = dir.join("job");
        let state = state_dir_for(&stem);
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(state.join("state.ron"), "x").unwrap();
        let mut panel = RenderPanel::new();
        panel.stem = Some(dir.join("older"));
        panel.settings.dir = dir.display().to_string();
        panel.settings.name = "job".into();
        assert!(!panel.can_resume(), "the older stem has no state");
        panel.refuse_start_if_blocked(&mut AppState::new(Scene::example(), None));
        assert_eq!(panel.stem.as_deref(), Some(stem.as_path()));
        assert!(panel.can_resume());
        panel.device_lost = true;
        assert!(
            !panel.can_resume(),
            "a dead device cannot resume in process"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    fn frame(ctx: &egui::Context, panel: &mut RenderPanel, events: Vec<egui::Event>) -> Action {
        frame_at_size(ctx, panel, events, egui::vec2(1600.0, 900.0))
    }

    fn frame_at_size(
        ctx: &egui::Context,
        panel: &mut RenderPanel,
        events: Vec<egui::Event>,
        size: egui::Vec2,
    ) -> Action {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            events,
            ..Default::default()
        };
        let state = AppState::new(Scene::example(), None);
        let mut action = Action::None;
        let mut out = ctx.run_ui(raw, |ui| action = panel.window(ui, &state));
        out.textures_delta.clear();
        action
    }

    #[test]
    fn render_action_stays_visible_below_scrolling_settings_on_a_compact_screen() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut panel = RenderPanel::new();
        panel.open = true;
        let size = egui::vec2(1000.0, 640.0);
        for _ in 0..4 {
            frame_at_size(&ctx, &mut panel, vec![], size);
        }
        let rect = ctx
            .data(|d| d.get_temp::<Rect>(egui::Id::new("render-start-button")))
            .unwrap();
        assert!(
            Rect::from_min_size(Pos2::ZERO, size).contains_rect(rect),
            "render footer stays on screen: {rect:?}"
        );
        let pos = rect.center();
        let click = |pressed| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        frame_at_size(&ctx, &mut panel, click(true), size);
        assert_eq!(
            frame_at_size(&ctx, &mut panel, click(false), size),
            Action::Start
        );
    }

    #[test]
    fn portrait_presets_keep_orientation_and_the_active_job_offers_pause() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut panel = RenderPanel::new();
        panel.open = true;
        panel.settings.width = 1080;
        panel.settings.height = 1920;
        for _ in 0..4 {
            frame(&ctx, &mut panel, vec![]);
        }
        let click = |pos: Pos2, pressed| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        let pos = ctx
            .data(|d| d.get_temp::<Rect>(egui::Id::new(("render-size", "4K"))))
            .unwrap()
            .center();
        frame(&ctx, &mut panel, click(pos, true));
        frame(&ctx, &mut panel, click(pos, false));
        assert_eq!((panel.settings.width, panel.settings.height), (2160, 3840));
        panel.status = Status::Running;
        frame(&ctx, &mut panel, vec![]);
        let pos = ctx
            .data(|d| d.get_temp::<Rect>(egui::Id::new("render-start-button")))
            .unwrap()
            .center();
        frame(&ctx, &mut panel, click(pos, true));
        assert_eq!(frame(&ctx, &mut panel, click(pos, false)), Action::Pause);
    }

    #[test]
    fn the_window_asks_for_a_start_and_marks_the_frame_while_a_size_is_hovered() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut panel = RenderPanel::new();
        panel.open = true;
        frame(&ctx, &mut panel, vec![]);
        frame(&ctx, &mut panel, vec![]);
        assert_eq!(panel.frame_guide(), None, "quiet at rest");

        // Hover the 4K preset: find it from the layout it was given.
        let mut hot = None;
        for y in (60..400).step_by(4) {
            for x in (60..700).step_by(8) {
                frame(
                    &ctx,
                    &mut panel,
                    vec![egui::Event::PointerMoved(Pos2::new(x as f32, y as f32))],
                );
                if panel.frame_guide().is_some() {
                    hot = Some((x, y));
                    break;
                }
            }
            if hot.is_some() {
                break;
            }
        }
        assert!(hot.is_some(), "hovering a size control marks the frame");
        assert_eq!(panel.frame_guide(), Some((1920, 1080)));
        frame(
            &ctx,
            &mut panel,
            vec![egui::Event::PointerMoved(Pos2::new(1500.0, 800.0))],
        );
        assert_eq!(panel.frame_guide(), None, "quiet again off the controls");
    }

    #[test]
    fn a_closed_window_draws_nothing_and_asks_for_nothing() {
        let ctx = egui::Context::default();
        let mut panel = RenderPanel::new();
        assert_eq!(frame(&ctx, &mut panel, vec![]), Action::None);
        assert_eq!(panel.frame_guide(), None);
        assert!(!panel.is_running());
    }
}
