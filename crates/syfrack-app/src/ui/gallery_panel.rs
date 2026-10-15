//! The « Galerie » tab: the renders folder as a grid of cards. Drawing returns
//! an action per frame, applied right after, as in the render panel.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use egui::{Color32, RichText, Sense, TextureHandle, Ui};

use crate::gallery::{self, Entry, Thumbnails};
use crate::state::{AppState, Banner, Tab};
use crate::ui::render_panel::{clock, resume_request_id, ResumeRequest};
use crate::ui::theme;

/// Folder shown at start: where the CLI and « Rendre » write by default.
const DEFAULT_DIR: &str = "renders";
/// The folder is scanned again this often while the tab is open.
const RESCAN_PERIOD: Duration = Duration::from_secs(2);
/// Outer width of a card, in points.
const CARD_W: f32 = 40.0 * theme::GRID;
/// Width of the thumbnail box inside a card.
const THUMB_BOX_W: f32 = CARD_W - 2.0 * theme::GRID;
/// 16:9 box: other aspects are letterboxed inside it.
const THUMB_BOX_H: f32 = THUMB_BOX_W * 9.0 / 16.0;
/// Height of the progress bar of a paused render.
const BAR_H: f32 = 4.0;

enum Action {
    None,
    Select(usize),
    Open(usize),
    Resume(usize),
    Reveal(usize),
    AskDelete(usize),
}

pub struct GalleryPanel {
    dir_text: String,
    scanned: Option<(PathBuf, Instant)>,
    entries: Vec<Entry>,
    /// The folder does not exist (shown in the panel, not as a banner).
    missing: bool,
    thumbs: Thumbnails,
    /// One texture per visible PNG, with the thumbnail version it holds.
    textures: HashMap<PathBuf, (Option<SystemTime>, TextureHandle)>,
    selected: Option<PathBuf>,
    /// Stem waiting for the deletion to be confirmed.
    confirm_delete: Option<PathBuf>,
    /// Stem whose opening waits for « ouvrir quand même ».
    confirm_open: Option<PathBuf>,
}

impl Default for GalleryPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl GalleryPanel {
    pub fn new() -> Self {
        Self::with_dir(
            &crate::paths::content_root()
                .join(DEFAULT_DIR)
                .to_string_lossy(),
        )
    }

    pub fn with_dir(dir: &str) -> Self {
        Self {
            dir_text: dir.to_string(),
            scanned: None,
            entries: Vec::new(),
            missing: false,
            thumbs: Thumbnails::new(),
            textures: HashMap::new(),
            selected: None,
            confirm_delete: None,
            confirm_open: None,
        }
    }

    fn dir(&self) -> PathBuf {
        PathBuf::from(self.dir_text.trim())
    }

    fn rescan(&mut self) {
        let dir = self.dir();
        self.missing = !dir.is_dir();
        self.entries = gallery::scan(&dir);
        let pngs: HashSet<PathBuf> = self.entries.iter().filter_map(|e| e.png.clone()).collect();
        for png in &pngs {
            self.thumbs.request(png);
        }
        self.thumbs.forget_except(&pngs);
        self.textures.retain(|p, _| pngs.contains(p));
        if self
            .selected
            .as_ref()
            .is_some_and(|s| !self.entries.iter().any(|e| &e.stem == s))
        {
            self.selected = None;
        }
        self.scanned = Some((dir, Instant::now()));
    }

    fn rescan_due(&self) -> bool {
        match &self.scanned {
            None => true,
            Some((dir, at)) => *dir != self.dir() || at.elapsed() >= RESCAN_PERIOD,
        }
    }

    /// Uploads the thumbnails the worker finished since the last frame.
    fn sync_textures(&mut self, ctx: &egui::Context) {
        self.thumbs.poll();
        for e in &self.entries {
            let Some(png) = &e.png else { continue };
            let (Some((w, h, px)), Some(stamp)) = (self.thumbs.get(png), self.thumbs.stamp(png))
            else {
                continue;
            };
            if self.textures.get(png).is_some_and(|(s, _)| *s == stamp) {
                continue;
            }
            let image = egui::ColorImage::from_rgba_unmultiplied([*w as usize, *h as usize], px);
            match self.textures.get_mut(png) {
                Some((s, tex)) => {
                    tex.set(image, egui::TextureOptions::LINEAR);
                    *s = stamp;
                }
                None => {
                    let tex = ctx.load_texture(
                        format!("gallery-{}", e.name),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    self.textures.insert(png.clone(), (stamp, tex));
                }
            }
        }
    }

    pub fn show(&mut self, ui: &mut Ui, state: &mut AppState) {
        if self.rescan_due() {
            self.rescan();
        }
        self.sync_textures(ui.ctx());
        let mut refresh = false;
        ui.horizontal(|ui| {
            ui.heading(syfrack_i18n::tr!("Galerie"));
            ui.separator();
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.dir_text)
                    .hint_text(syfrack_i18n::tr!("dossier des rendus"))
                    .desired_width(36.0 * theme::GRID),
            );
            if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                refresh = true;
            }
            if ui.button(syfrack_i18n::tr!("Actualiser")).clicked() {
                refresh = true;
            }
            let n = self.entries.len();
            let count = match n {
                0 => syfrack_i18n::tr!("aucun rendu").to_string(),
                1 => syfrack_i18n::tr!("1 rendu").to_string(),
                n => syfrack_i18n::tr_format!("{n} rendus", n = n),
            };
            ui.label(RichText::new(count).color(theme::TEXT_DIM));
        });
        if refresh {
            self.rescan();
        }
        ui.add_space(theme::GRID);
        if self.missing {
            ui.label(
                RichText::new(syfrack_i18n::tr_format!(
                    "{} : dossier introuvable",
                    self.dir().display()
                ))
                .color(theme::ERROR),
            );
            return;
        }
        let mut action = Action::None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let cols = (((ui.available_width() + theme::GRID) / (CARD_W + theme::GRID)).floor()
                    as usize)
                    .max(1);
                ui.spacing_mut().item_spacing = egui::vec2(theme::GRID, theme::GRID);
                let indices: Vec<usize> = (0..self.entries.len()).collect();
                for row in indices.chunks(cols) {
                    ui.horizontal_top(|ui| {
                        for &i in row {
                            let e = &self.entries[i];
                            let tex = e
                                .png
                                .as_ref()
                                .and_then(|p| self.textures.get(p))
                                .map(|(_, t)| t);
                            let selected = self.selected.as_ref() == Some(&e.stem);
                            let a = card(ui, e, tex, selected, i);
                            if !matches!(a, Action::None) {
                                action = a;
                            }
                        }
                    });
                }
            });
        self.apply(ui, state, action);
    }

    pub fn dialogs(&mut self, ui: &Ui, state: &mut AppState) {
        self.open_confirmation(ui, state);
        self.delete_dialog(ui, state);
    }

    fn apply(&mut self, ui: &Ui, state: &mut AppState, action: Action) {
        match action {
            Action::None => {}
            Action::Select(i) => self.selected = Some(self.entries[i].stem.clone()),
            Action::Open(i) => {
                self.selected = Some(self.entries[i].stem.clone());
                if state.has_unsaved_workspaces() {
                    self.confirm_open = Some(self.entries[i].stem.clone());
                } else {
                    self.open_in_atelier(i, state);
                }
            }
            Action::Resume(i) => {
                let e = &self.entries[i];
                self.selected = Some(e.stem.clone());
                if let Some(st) = &e.state {
                    let look = gallery::scene_from_entry(e).ok().map(|s| s.look);
                    let req = ResumeRequest {
                        stem: e.stem.clone(),
                        state_dir: st.dir.clone(),
                        look,
                    };
                    ui.data_mut(|d| d.insert_temp(resume_request_id(), req));
                }
            }
            Action::Reveal(i) => {
                let e = &self.entries[i];
                self.selected = Some(e.stem.clone());
                if let Err(err) = reveal(e) {
                    state.banner = Some(Banner::error(format!("{} : {err:#}", e.name)));
                }
            }
            Action::AskDelete(i) => self.confirm_delete = Some(self.entries[i].stem.clone()),
        }
    }

    fn open_in_atelier(&mut self, i: usize, state: &mut AppState) {
        let e = &self.entries[i];
        match gallery::scene_with_source(e) {
            Ok((scene, from_ron)) => {
                let source = if from_ron { e.ron.clone() } else { None };
                if state.load_scene(scene, source) {
                    state.tab = Tab::Atelier;
                }
            }
            Err(err) => state.banner = Some(Banner::error(format!("{} : {err:#}", e.name))),
        }
    }

    /// « Modifications non enregistrées » before a gallery open replaces the scene.
    fn open_confirmation(&mut self, ui: &Ui, state: &mut AppState) {
        let Some(stem) = self.confirm_open.clone() else {
            return;
        };
        if let Some(yes) = super::confirm_discard(ui.ctx(), egui::Id::new("gallery-open-confirm")) {
            self.confirm_open = None;
            if let (true, Some(i)) = (yes, self.entries.iter().position(|e| e.stem == stem)) {
                self.open_in_atelier(i, state);
            }
        }
    }

    fn delete_dialog(&mut self, ui: &Ui, state: &mut AppState) {
        let Some(stem) = self.confirm_delete.clone() else {
            return;
        };
        let Some(idx) = self.entries.iter().position(|e| e.stem == stem) else {
            self.confirm_delete = None;
            return;
        };
        let name = self.entries[idx].name.clone();
        let answer = crate::ui::floating::confirmation(
            ui.ctx(),
            egui::Id::new("gallery-delete"),
            syfrack_i18n::tr!("Supprimer ce rendu ?"),
            syfrack_i18n::tr!("Supprimer"),
            syfrack_i18n::tr!("Annuler"),
            |ui| {
                ui.add(egui::Label::new(RichText::new(&name).strong()).truncate())
                    .on_hover_text(&name);
                ui.label(syfrack_i18n::tr!(
                    "L’image, la scène et les données de reprise seront effacées définitivement."
                ));
            },
        );
        if answer == Some(true) {
            match gallery::delete_entry(&self.entries[idx]) {
                Ok(()) => {
                    state.banner = Some(Banner::info(syfrack_i18n::tr_format!(
                        "« {name} » supprimé",
                        name = name
                    )))
                }
                Err(e) => state.banner = Some(Banner::error(format!("{e:#}"))),
            }
            self.confirm_delete = None;
            self.rescan();
        } else if answer == Some(false) {
            self.confirm_delete = None;
        }
    }
}

/// One card: thumbnail, name, facts, quiet action buttons.
fn card(
    ui: &mut Ui,
    e: &Entry,
    tex: Option<&TextureHandle>,
    selected: bool,
    index: usize,
) -> Action {
    let mut action = Action::None;
    let stroke = if selected {
        egui::Stroke::new(1.5, theme::ACCENT)
    } else {
        egui::Stroke::new(1.0, theme::SEPARATOR)
    };
    egui::Frame::NONE
        .fill(theme::PANEL)
        .stroke(stroke)
        .corner_radius(4.0)
        .inner_margin(theme::GRID)
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.set_width(THUMB_BOX_W);
                ui.spacing_mut().item_spacing = egui::vec2(theme::GRID / 2.0, theme::GRID / 2.0);
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(THUMB_BOX_W, THUMB_BOX_H), Sense::click());
                ui.painter().rect_filled(rect, 2.0, theme::BG);
                match tex {
                    Some(t) => {
                        let [w, h] = t.size();
                        let scale = (rect.width() / w as f32).min(rect.height() / h as f32);
                        let img = egui::Rect::from_center_size(
                            rect.center(),
                            egui::vec2(w as f32, h as f32) * scale,
                        );
                        ui.painter().image(
                            t.id(),
                            img,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    None => {
                        let text = if e.png.is_some() {
                            "…"
                        } else {
                            syfrack_i18n::tr!("pas d'image")
                        };
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            text,
                            egui::FontId::proportional(13.0),
                            theme::TEXT_DIM,
                        );
                    }
                }
                if resp.double_clicked() && (e.png.is_some() || e.ron.is_some()) {
                    action = Action::Open(index);
                } else if resp.clicked() {
                    action = Action::Select(index);
                }

                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(RichText::new(&e.name)).truncate());
                    if e.state.as_ref().is_some_and(|s| s.resumable()) {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            egui::Frame::NONE
                                .stroke(egui::Stroke::new(1.0, theme::ACCENT))
                                .corner_radius(3.0)
                                .inner_margin(egui::Margin::symmetric(4, 0))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(syfrack_i18n::tr!("en pause"))
                                            .small()
                                            .color(theme::ACCENT),
                                    );
                                });
                        });
                    }
                });
                ui.add(
                    egui::Label::new(RichText::new(facts_line(e)).small().color(theme::TEXT_DIM))
                        .truncate(),
                );
                if let Some(st) = e.state.as_ref().filter(|s| s.resumable()) {
                    let frac = st.samples_min as f32 / st.target.max(1) as f32;
                    let (bar, _) =
                        ui.allocate_exact_size(egui::vec2(THUMB_BOX_W, BAR_H), Sense::hover());
                    ui.painter().rect_filled(bar, 2.0, theme::SEPARATOR);
                    let mut fill = bar;
                    fill.set_width(bar.width() * frac.clamp(0.0, 1.0));
                    ui.painter().rect_filled(fill, 2.0, theme::ACCENT);
                }
                ui.horizontal(|ui| {
                    let can_open = e.png.is_some() || e.ron.is_some();
                    if ui
                        .add_enabled(
                            can_open,
                            egui::Button::new(syfrack_i18n::tr!("Ouvrir")).small(),
                        )
                        .on_hover_text(syfrack_i18n::tr!("charger la scène dans l'atelier"))
                        .clicked()
                    {
                        action = Action::Open(index);
                    }
                    if e.state.as_ref().is_some_and(|s| s.resumable())
                        && ui
                            .add(egui::Button::new(syfrack_i18n::tr!("Reprendre")).small())
                            .on_hover_text(syfrack_i18n::tr!("reprendre le rendu en pause"))
                            .clicked()
                    {
                        action = Action::Resume(index);
                    }
                    if ui
                        .add(egui::Button::new(syfrack_i18n::tr!("Révéler")).small())
                        .on_hover_text(syfrack_i18n::tr!("montrer dans l'explorateur"))
                        .clicked()
                    {
                        action = Action::Reveal(index);
                    }
                    if ui
                        .add(egui::Button::new(syfrack_i18n::tr!("Supprimer")).small())
                        .clicked()
                    {
                        action = Action::AskDelete(index);
                    }
                });
            });
        });
    action
}

/// « 3840×2160 · 41 / 200 spp · 3 min 07 s · 2026-10-06 14:32 ».
fn facts_line(e: &Entry) -> String {
    let mut parts = Vec::new();
    if let Some((w, h)) = e.size {
        parts.push(format!("{w}×{h}"));
    }
    if let Some(st) = &e.state {
        if st.resumable() {
            parts.push(format!("{} / {} spp", st.samples_min, st.target));
        } else {
            parts.push(format!("{} spp", st.target));
        }
        parts.push(clock(Duration::from_secs_f64(st.elapsed_secs.max(0.0))));
    }
    if let Some(secs) = e
        .modified
        .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
    {
        parts.push(gallery::format_utc(secs));
    }
    parts.join(" · ")
}

/// What « Révéler » selects: the image, else the next best file of the entry.
fn reveal_target(e: &Entry) -> Option<&Path> {
    e.png
        .as_deref()
        .or(e.exr.as_deref())
        .or(e.ron.as_deref())
        .or(e.state.as_ref().map(|s| s.dir.as_path()))
}

fn reveal(e: &Entry) -> Result<()> {
    let target = reveal_target(e).ok_or_else(|| anyhow!("nothing to show"))?;
    let abs = std::fs::canonicalize(target).with_context(|| target.display().to_string())?;
    #[cfg(windows)]
    {
        // `canonicalize` yields `\\?\C:\…`, which Explorer's /select, refuses.
        let shown = abs.display().to_string();
        let shown = shown.strip_prefix(r"\\?\").unwrap_or(&shown).to_string();
        std::process::Command::new("explorer")
            .arg(format!("/select,{shown}"))
            .spawn()
            .context("explorer")?;
    }
    #[cfg(not(windows))]
    {
        let parent = abs.parent().unwrap_or(&abs);
        std::process::Command::new("xdg-open")
            .arg(parent)
            .spawn()
            .context("xdg-open")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use syfrack_color::{encode::EncodedImage, write_png};
    use syfrack_core::{OutputSpace, Scene};

    fn folder(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "syfrack-gallery-panel-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut scene = Scene::example();
        scene.seed = 7;
        let ron = scene.to_ron();
        let img = EncodedImage {
            width: 4,
            height: 2,
            bits: 8,
            data: vec![90; 4 * 2 * 3],
        };
        write_png(dir.join("r.png"), &img, OutputSpace::Srgb, Some(&ron)).unwrap();
        std::fs::write(dir.join("r.ron"), ron).unwrap();
        dir
    }

    /// One frame of the tab, then `f` with a live `Ui`.
    fn with_ui(f: impl FnOnce(&mut Ui)) {
        let ctx = egui::Context::default();
        let mut f = Some(f);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| (f.take().unwrap())(ui));
        out.textures_delta.clear();
    }

    #[test]
    fn opening_an_entry_loads_its_scene_and_switches_to_the_atelier() {
        let dir = folder("open");
        let mut g = GalleryPanel::with_dir(&dir.display().to_string());
        g.rescan();
        assert_eq!(g.entries.len(), 1);
        let mut state = AppState::new(Scene::example(), None);
        state.tab = Tab::Galerie;
        with_ui(|ui| g.apply(ui, &mut state, Action::Open(0)));
        assert_eq!(state.scene.seed, 7);
        assert_eq!(state.tab, Tab::Atelier);
        assert!(
            state.path.is_none(),
            "the scene came from the PNG: Ctrl+S must ask where to save"
        );
        assert!(state.take_preview_dirty());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn opening_an_entry_without_a_scene_raises_a_banner_and_stays() {
        let dir = folder("noscene");
        std::fs::remove_file(dir.join("r.ron")).unwrap();
        let img = EncodedImage {
            width: 2,
            height: 2,
            bits: 8,
            data: vec![0; 12],
        };
        write_png(dir.join("r.png"), &img, OutputSpace::Srgb, None).unwrap();
        let mut g = GalleryPanel::with_dir(&dir.display().to_string());
        g.rescan();
        let mut state = AppState::new(Scene::example(), None);
        state.tab = Tab::Galerie;
        with_ui(|ui| g.apply(ui, &mut state, Action::Open(0)));
        assert_eq!(state.tab, Tab::Galerie);
        assert!(state.banner.as_ref().unwrap().error);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_tab_draws_a_missing_folder_and_a_real_one_without_panicking() {
        let dir = folder("draw");
        for path in ["Z:/no/such/dir".to_string(), dir.display().to_string()] {
            let mut g = GalleryPanel::with_dir(&path);
            let mut state = AppState::new(Scene::example(), None);
            for _ in 0..3 {
                with_ui(|ui| g.show(ui, &mut state));
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn opening_with_unsaved_edits_waits_for_a_confirmation() {
        let dir = folder("dirty");
        let mut g = GalleryPanel::with_dir(&dir.display().to_string());
        g.rescan();
        let mut state = AppState::new(Scene::example(), None);
        state.tab = Tab::Galerie;
        state.set_param("look.exposure_ev", 1.0);
        with_ui(|ui| g.apply(ui, &mut state, Action::Open(0)));
        assert_eq!(state.tab, Tab::Galerie, "nothing opened yet");
        assert!(g.confirm_open.is_some());
        assert_eq!(state.scene.look.exposure_ev, 1.0);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_ron_only_entry_keeps_its_path_for_saving() {
        let dir = folder("ronpath");
        let img = EncodedImage {
            width: 2,
            height: 2,
            bits: 8,
            data: vec![0; 12],
        };
        write_png(dir.join("r.png"), &img, OutputSpace::Srgb, None).unwrap();
        let mut g = GalleryPanel::with_dir(&dir.display().to_string());
        g.rescan();
        let mut state = AppState::new(Scene::example(), None);
        with_ui(|ui| g.apply(ui, &mut state, Action::Open(0)));
        assert_eq!(state.path.as_deref(), Some(dir.join("r.ron").as_path()));
        std::fs::remove_dir_all(dir).ok();
    }
}
