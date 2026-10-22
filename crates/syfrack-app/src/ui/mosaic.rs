//! Independent image workshop. Jobs, edits and textures survive tab switches;
//! the scene, its render inputs and its undo history remain untouched.
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{Color32, ColorImage, Id, Pos2, Rect, RichText, Sense, Stroke, TextureHandle, Vec2};
use syfrack_color::source::ImageAdjustments;
use syfrack_core::{Bounds, HdrImage};
use syfrack_render::mosaic::MosaicControl;

use super::{file_browser, theme, widgets};
use crate::state::AppState;

mod priority;
#[cfg(test)]
mod tests;
mod worker;
use worker::{Built, Imported, Preview, Product, Settings};

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Source,
    Layout,
    Mosaic,
    Compare,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Import,
    Preview,
    Build,
    Export,
}
enum Reply {
    Imported(Imported),
    Preview(Preview),
    Built(Built),
    Exported(PathBuf),
}
struct Task {
    kind: Kind,
    control: Arc<MosaicControl>,
    receiver: mpsc::Receiver<Result<Reply, String>>,
}
impl Task {
    fn spawn(
        ctx: &egui::Context,
        kind: Kind,
        work: impl FnOnce(&MosaicControl) -> anyhow::Result<Reply> + Send + 'static,
    ) -> Self {
        let control = Arc::new(MosaicControl::default());
        let cancellation = control.clone();
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&cancellation)))
                    .map_err(|_| "mosaic worker interrupted".to_owned())
                    .and_then(|r| r.map_err(|e| format!("{e:#}")));
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        Self {
            kind,
            control,
            receiver,
        }
    }
    fn poll(&self) -> Option<Result<Reply, String>> {
        match self.receiver.try_recv() {
            Ok(value) => Some(value),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err("mosaic worker disconnected".into())),
        }
    }
}
impl Drop for Task {
    fn drop(&mut self) {
        self.control.cancel();
    }
}

pub struct MosaicPanel {
    settings: Settings,
    raw: Option<Arc<HdrImage>>,
    path: Option<PathBuf>,
    raw_texture: Option<TextureHandle>,
    source_texture: Option<TextureHandle>,
    layout_texture: Option<TextureHandle>,
    mosaic_texture: Option<TextureHandle>,
    preview_settings: Option<Settings>,
    regions: usize,
    product: Option<Arc<Product>>,
    saved_settings: Option<Settings>,
    product_saved: bool,
    task: Option<Task>,
    preview: Option<Task>,
    pending_preview: bool,
    changed_at: Instant,
    message: Option<(String, bool)>,
    view: View,
    zoom: f32,
    pan: Vec2,
    crop_mode: bool,
    crop_start: Option<Pos2>,
    brush_mode: bool,
    brush_erase: bool,
    brush_radius: f32,
    brush_active: bool,
    show_priority: bool,
    priority_texture: Option<TextureHandle>,
    priority_dirty: bool,
    compare: f32,
    directory: String,
    name: String,
    scale: u32,
    undo: Vec<Settings>,
    redo: Vec<Settings>,
    gesture: bool,
    generate_requested: bool,
    export_requested: bool,
}
impl Default for MosaicPanel {
    fn default() -> Self {
        Self {
            settings: Settings::default(),
            raw: None,
            path: None,
            raw_texture: None,
            source_texture: None,
            layout_texture: None,
            mosaic_texture: None,
            preview_settings: None,
            regions: 0,
            product: None,
            saved_settings: None,
            product_saved: true,
            task: None,
            preview: None,
            pending_preview: false,
            changed_at: Instant::now(),
            message: None,
            view: View::Source,
            zoom: 1.0,
            pan: Vec2::ZERO,
            crop_mode: false,
            crop_start: None,
            brush_mode: false,
            brush_erase: false,
            brush_radius: 0.045,
            brush_active: false,
            show_priority: false,
            priority_texture: None,
            priority_dirty: true,
            compare: 0.5,
            directory: crate::paths::content_root()
                .join("renders/mosaic")
                .display()
                .to_string(),
            name: String::new(),
            scale: 2,
            undo: vec![],
            redo: vec![],
            gesture: false,
            generate_requested: false,
            export_requested: false,
        }
    }
}

fn panel_id() -> Id {
    Id::new("mosaic-workshop")
}
fn image_id() -> Id {
    Id::new("mosaic-import-image")
}
fn folder_id() -> Id {
    Id::new("mosaic-output-folder")
}
fn memory(ctx: &egui::Context) -> Arc<Mutex<MosaicPanel>> {
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Arc<Mutex<MosaicPanel>>>(panel_id())
            .clone()
    })
}

pub fn choose_image(ui: &egui::Ui, state: &AppState) {
    let initial = memory(ui.ctx())
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .path
        .clone()
        .unwrap_or_else(crate::paths::content_root);
    file_browser::open_image_dialog(ui, state, &initial, image_id());
}
pub fn open_image(ctx: &egui::Context, path: &Path) {
    memory(ctx)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .import(ctx, path.to_path_buf());
    ctx.request_repaint();
}
pub fn is_busy(ctx: &egui::Context) -> bool {
    let binding = memory(ctx);
    let panel = binding.lock().unwrap_or_else(|e| e.into_inner());
    panel.task.is_some()
        || panel.preview.is_some()
        || panel.pending_preview
        || panel.generate_requested
}
pub fn has_result(ctx: &egui::Context) -> bool {
    memory(ctx)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .current()
}
pub fn has_unsaved_work(ctx: &egui::Context) -> bool {
    let binding = memory(ctx);
    let panel = binding.lock().unwrap_or_else(|e| e.into_inner());
    panel.raw.is_some()
        && (panel.saved_settings.as_ref() != Some(&panel.settings)
            || (panel.product.is_some() && !panel.product_saved)
            || panel
                .task
                .as_ref()
                .is_some_and(|t| matches!(t.kind, Kind::Build | Kind::Export)))
}
pub fn request_generate(ctx: &egui::Context) {
    memory(ctx)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .generate_requested = true;
    ctx.request_repaint();
}
pub fn request_export(ctx: &egui::Context) {
    memory(ctx)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .export_requested = true;
    ctx.request_repaint();
}
pub fn title(ctx: &egui::Context) -> String {
    memory(ctx)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .path
        .as_ref()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| syfrack_i18n::tr!("Choisir une image").into())
}
pub fn history(ctx: &egui::Context, redo: bool, apply: bool) -> bool {
    let binding = memory(ctx);
    let mut panel = binding.lock().unwrap_or_else(|e| e.into_inner());
    let available = if redo {
        !panel.redo.is_empty()
    } else {
        !panel.undo.is_empty()
    };
    if available && apply {
        let current = panel.settings.clone();
        panel.settings = if redo {
            panel.redo.pop().unwrap()
        } else {
            panel.undo.pop().unwrap()
        };
        if redo {
            panel.undo.push(current);
        } else {
            panel.redo.push(current);
        }
        panel.dirty();
        panel.gesture = false;
        panel.brush_active = false;
    }
    available
}

impl MosaicPanel {
    fn current(&self) -> bool {
        self.product
            .as_ref()
            .is_some_and(|p| p.settings == self.settings)
    }
    fn dirty(&mut self) {
        self.priority_dirty = true;
        self.pending_preview = self.raw.is_some();
        self.changed_at = Instant::now();
        self.preview = None;
        if self.view == View::Compare {
            self.view = View::Source;
        }
    }
    fn track(&mut self, before: Settings, held: bool) {
        if before != self.settings {
            if !self.gesture {
                self.undo.push(before);
                if self.undo.len() > 64 {
                    self.undo.remove(0);
                }
                self.redo.clear();
            }
            self.gesture = held;
            self.dirty();
        }
        if !held {
            self.gesture = false;
        }
    }
    fn import(&mut self, ctx: &egui::Context, path: PathBuf) {
        self.task = None;
        self.preview = None;
        self.pending_preview = false;
        self.message = None;
        self.task = Some(Task::spawn(ctx, Kind::Import, move |_| {
            Ok(Reply::Imported(worker::import(&path)?))
        }));
    }
    fn generate(&mut self, ctx: &egui::Context) {
        if self.task.is_some() {
            return;
        }
        let Some(raw) = self.raw.clone() else {
            return;
        };
        let settings = self.settings.clone();
        self.preview = None;
        self.pending_preview = false;
        self.crop_mode = false;
        self.brush_mode = false;
        self.brush_active = false;
        self.message = None;
        self.task = Some(Task::spawn(ctx, Kind::Build, move |control| {
            Ok(Reply::Built(worker::build(&raw, settings, control)?))
        }));
    }
    fn export(&mut self, ctx: &egui::Context) {
        if self.task.is_some() {
            return;
        }
        if !self.current() {
            self.message = Some((
                syfrack_i18n::tr!(
                    "Générer la mosaïque avec les réglages actuels avant d’exporter."
                )
                .into(),
                true,
            ));
            return;
        }
        let product = self.product.clone().unwrap();
        let raw = self.raw.clone().unwrap();
        let directory = PathBuf::from(&self.directory);
        let name = self.name.clone();
        let scale = self.scale;
        self.message = None;
        self.task = Some(Task::spawn(ctx, Kind::Export, move |control| {
            Ok(Reply::Exported(worker::export(
                &product, &raw, &directory, &name, scale, control,
            )?))
        }));
    }
    fn texture(ctx: &egui::Context, name: &str, image: ColorImage) -> TextureHandle {
        ctx.load_texture(name, image, egui::TextureOptions::LINEAR)
    }
    fn adopt(&mut self, ctx: &egui::Context, reply: Reply) {
        match reply {
            Reply::Imported(imported) => {
                self.name = format!(
                    "{}-mosaique",
                    imported
                        .path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_")
                );
                self.raw = Some(imported.raw);
                self.path = Some(imported.path);
                self.settings = imported.settings;
                self.saved_settings = Some(self.settings.clone());
                self.product_saved = true;
                self.raw_texture = Some(Self::texture(ctx, "mosaic-original", imported.image));
                self.product = None;
                self.source_texture = None;
                self.layout_texture = None;
                self.mosaic_texture = None;
                self.undo.clear();
                self.redo.clear();
                self.gesture = false;
                self.view = View::Source;
                self.crop_mode = false;
                self.crop_start = None;
                self.brush_mode = false;
                self.brush_active = false;
                self.priority_texture = None;
                self.priority_dirty = true;
                self.zoom = 1.0;
                self.pan = Vec2::ZERO;
                self.dirty();
                self.changed_at = Instant::now() - Duration::from_millis(200);
            }
            Reply::Preview(preview) => {
                if preview.settings != self.settings {
                    return;
                }
                let previous_size = self.source_texture.as_ref().map(TextureHandle::size);
                if previous_size != Some(preview.source.size) {
                    self.zoom = 1.0;
                    self.pan = Vec2::ZERO;
                }
                self.source_texture = Some(Self::texture(ctx, "mosaic-adjusted", preview.source));
                self.layout_texture =
                    Some(Self::texture(ctx, "mosaic-subdivision", preview.layout));
                self.regions = preview.regions;
                self.preview_settings = Some(preview.settings);
            }
            Reply::Built(built) => {
                let max_scale = (1..=4)
                    .filter(|s| {
                        built.product.image.data.len() as u64 * (*s as u64).pow(2)
                            <= syfrack_core::mosaic::MAX_MOSAIC_PIXELS
                    })
                    .max()
                    .unwrap_or(1);
                self.scale = self.scale.min(max_scale);
                if built.product.settings == self.settings {
                    self.source_texture = Some(Self::texture(ctx, "mosaic-adjusted", built.source));
                    self.layout_texture =
                        Some(Self::texture(ctx, "mosaic-subdivision", built.layout));
                    self.preview_settings = Some(self.settings.clone());
                    self.regions = built.product.recipe.tiles.len();
                }
                self.mosaic_texture = Some(Self::texture(ctx, "mosaic-result", built.mosaic));
                self.message = Some((
                    syfrack_i18n::tr_format!(
                        "{count} régions · {seconds:.1} s",
                        count = built.product.recipe.tiles.len(),
                        seconds = built.product.seconds
                    ),
                    false,
                ));
                self.product = Some(built.product);
                self.product_saved = false;
                self.view = View::Mosaic;
                self.zoom = 1.0;
                self.pan = Vec2::ZERO;
            }
            Reply::Exported(path) => {
                self.saved_settings = self.product.as_ref().map(|p| p.settings.clone());
                self.product_saved = true;
                self.message = Some((
                    syfrack_i18n::tr_format!("Mosaïque exportée · {path}", path = path.display()),
                    false,
                ))
            }
        }
    }
    fn poll(&mut self, ctx: &egui::Context) {
        for is_preview in [true, false] {
            let result = if is_preview {
                self.preview.as_ref().and_then(Task::poll)
            } else {
                self.task.as_ref().and_then(Task::poll)
            };
            if let Some(result) = result {
                let task = if is_preview {
                    self.preview.take()
                } else {
                    self.task.take()
                }
                .unwrap();
                if task.control.is_cancelled() {
                    if !is_preview {
                        self.message = Some((syfrack_i18n::tr!("Traitement annulé").into(), false));
                    }
                } else {
                    match result {
                        Ok(reply) => self.adopt(ctx, reply),
                        Err(error) => {
                            self.message = Some((
                                syfrack_i18n::tr_format!(
                                    "Traitement impossible · {error}",
                                    error = error
                                ),
                                true,
                            ))
                        }
                    }
                }
            }
        }
        if self.generate_requested {
            self.generate_requested = false;
            self.generate(ctx);
        }
        if self.export_requested {
            self.export_requested = false;
            self.export(ctx);
        }
        if self.pending_preview
            && !self.brush_active
            && self.changed_at.elapsed() >= Duration::from_millis(180)
            && !self.task.as_ref().is_some_and(|t| t.kind == Kind::Import)
        {
            if let Some(raw) = self.raw.clone() {
                self.pending_preview = false;
                let settings = self.settings.clone();
                self.preview = Some(Task::spawn(ctx, Kind::Preview, move |control| {
                    Ok(Reply::Preview(worker::preview(&raw, settings, control)?))
                }));
            }
        }
        if self.task.is_some() || self.preview.is_some() || self.pending_preview {
            ctx.request_repaint_after(Duration::from_millis(40));
        }
    }
}

pub fn poll(ctx: &egui::Context) {
    let (image, folder) = ctx.data_mut(|d| {
        (
            d.remove_temp::<PathBuf>(image_id()),
            d.remove_temp::<PathBuf>(folder_id()),
        )
    });
    let binding = memory(ctx);
    let mut panel = binding.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(path) = image {
        panel.import(ctx, path);
    }
    if let Some(path) = folder {
        panel.directory = path.display().to_string();
    }
    panel.poll(ctx);
}

fn slider(
    ui: &mut egui::Ui,
    key: &str,
    label: &str,
    value: &mut f32,
    min: f32,
    max: f32,
    integer: bool,
) -> bool {
    let response = widgets::ParamSlider::new(label, Bounds::new(min, max, &[]))
        .decimals(if integer { 0 } else { 2 })
        .show(ui, value);
    if integer {
        *value = value.round();
    }
    #[cfg(test)]
    ui.data_mut(|d| d.insert_temp(Id::new(("mosaic-param", key)), response.rect));
    let _ = key;
    response.changed()
}
fn button_id(ui: &egui::Ui, key: &str, response: &egui::Response) {
    #[cfg(test)]
    ui.data_mut(|d| d.insert_temp(Id::new(("mosaic-button", key)), response.rect));
    let _ = (ui, key, response);
}

pub fn show(ui: &mut egui::Ui, state: &AppState) {
    let binding = memory(ui.ctx());
    let mut panel = binding.lock().unwrap_or_else(|e| e.into_inner());
    let before = panel.settings.clone();
    egui::Panel::right("mosaic-controls").default_size(384.0).size_range(368.0..=460.0).resizable(true).show(ui, |ui| {
        ui.add_space(8.0);
        ui.heading(syfrack_i18n::tr!("MOSAÏQUE"));
        ui.label(RichText::new(syfrack_i18n::tr!("Une image, des milliers de fractales")).small().color(theme::TEXT_DIM));
        ui.add_space(8.0);
        let open = ui.add_sized([ui.available_width(), 30.0], egui::Button::new(syfrack_i18n::tr!("Ouvrir une image / un projet…")));
        button_id(ui, "open", &open);
        if open.clicked() {
            let initial = panel.path.clone().unwrap_or_else(crate::paths::content_root);
            file_browser::open_image_dialog(ui, state, &initial, image_id());
        }
        if let Some(path) = &panel.path {
            ui.add(egui::Label::new(path.file_name().unwrap_or_default().to_string_lossy()).truncate()).on_hover_text(path.display().to_string());
        }
        ui.separator();
        let scroll_height = (ui.available_height() - 126.0).max(80.0);
        egui::ScrollArea::vertical().id_salt("mosaic-settings").max_height(scroll_height).auto_shrink([false, false]).show(ui, |ui| {
            ui.add_enabled_ui(panel.raw.is_some(), |ui| {
                egui::CollapsingHeader::new(syfrack_i18n::tr!("Ajuster l’image")).default_open(true).show(ui, |ui| {
                    slider(ui, "exposure", syfrack_i18n::tr!("Exposition"), &mut panel.settings.image.exposure, -4.0, 4.0, false);
                    slider(ui, "contrast", syfrack_i18n::tr!("Contraste"), &mut panel.settings.image.contrast, 0.25, 3.0, false);
                    slider(ui, "saturation", syfrack_i18n::tr!("Saturation"), &mut panel.settings.image.saturation, 0.0, 3.0, false);
                    ui.horizontal(|ui| {
                        let crop = ui.selectable_label(panel.crop_mode, syfrack_i18n::tr!("Recadrer à la souris"));
                        button_id(ui, "crop", &crop);
                        if crop.clicked() { panel.crop_mode = !panel.crop_mode; panel.brush_mode = false; panel.brush_active = false; panel.zoom = 1.0; panel.pan = Vec2::ZERO; panel.crop_start = None; }
                        let rotate = ui.button("+90°").on_hover_text(syfrack_i18n::tr!("Tourner l’image"));
                        button_id(ui, "rotate", &rotate);
                        if rotate.clicked() { panel.settings.image.rotation = (panel.settings.image.rotation + 1) % 4; }
                    });
                    ui.horizontal(|ui| {
                        if ui.small_button(syfrack_i18n::tr!("Image entière")).clicked() { panel.settings.image.crop = [0.0, 0.0, 1.0, 1.0]; }
                        if ui.small_button(syfrack_i18n::tr!("Réinitialiser l’image")).clicked() { panel.settings.image = ImageAdjustments::default(); }
                    });
                });
                ui.add_space(8.0);
                egui::CollapsingHeader::new(syfrack_i18n::tr!("Préserver les détails")).default_open(true).show(ui, |ui| {
                    let mut fidelity = panel.settings.refinement * 100.0;
                    if slider(ui, "refinement", syfrack_i18n::tr!("Raffinement par erreur"), &mut fidelity, 0.0, 100.0, true) { panel.settings.refinement = fidelity / 100.0; }
                    ui.label(RichText::new(syfrack_i18n::tr!("Les régions mal restituées reçoivent davantage de petites fractales.")).small().color(theme::TEXT_DIM));
                    ui.horizontal(|ui| {
                        for (key, label, erase) in [("paint", syfrack_i18n::tr!("Peindre"), false), ("erase", syfrack_i18n::tr!("Effacer"), true)] {
                            let response = ui.selectable_label(panel.brush_mode && panel.brush_erase == erase, label);
                            button_id(ui, key, &response);
                            if response.clicked() {
                                panel.brush_mode = !(panel.brush_mode && panel.brush_erase == erase);
                                panel.brush_erase = erase;
                                panel.brush_active = false;
                                panel.crop_mode = false;
                                panel.view = View::Source;
                            }
                        }
                        if ui.small_button(syfrack_i18n::tr!("Tout effacer")).clicked() { panel.settings.priority.clear(); panel.brush_active = false; }
                    });
                    let mut radius = panel.brush_radius * 100.0;
                    if slider(ui, "brush-radius", syfrack_i18n::tr!("Rayon du pinceau"), &mut radius, 1.0, 15.0, false) { panel.brush_radius = radius / 100.0; }
                    ui.checkbox(&mut panel.show_priority, syfrack_i18n::tr!("Voir les priorités"));
                    ui.label(RichText::new(syfrack_i18n::tr!("Peindre les traits à préserver, puis actualiser. Ctrl+Z annule un trait.")).small().color(theme::TEXT_DIM));
                });
                ui.add_space(8.0);
                egui::CollapsingHeader::new(syfrack_i18n::tr!("Découpage adaptatif")).default_open(true).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (label, size, budget, detail) in [
                            (syfrack_i18n::tr!("Rapide"), 512, 1024, 0.065),
                            (syfrack_i18n::tr!("Équilibré"), 768, 4096, 0.05),
                            (syfrack_i18n::tr!("Détaillé"), 1024, 8192, 0.03),
                        ] { if ui.small_button(label).clicked() { panel.settings.size = size; panel.settings.config = syfrack_render::mosaic::MosaicConfig { max_regions: budget, detail, ..Default::default() }; } }
                    });
                    let mut size = panel.settings.size as f32;
                    if slider(ui, "size", syfrack_i18n::tr!("Définition d’analyse"), &mut size, 128.0, 2048.0, true) { panel.settings.size = size as u32; }
                    let mut fineness = ((0.15 - panel.settings.config.detail) / 0.145 * 100.0).clamp(0.0, 100.0);
                    if slider(ui, "detail", syfrack_i18n::tr!("Finesse"), &mut fineness, 0.0, 100.0, false) { panel.settings.config.detail = 0.15 - fineness / 100.0 * 0.145; }
                    let mut min = panel.settings.config.min_size as f32;
                    if slider(ui, "min", syfrack_i18n::tr!("Petites régions"), &mut min, 2.0, 64.0, true) { panel.settings.config.min_size = min as u32; panel.settings.config.max_size = panel.settings.config.max_size.max(min as u32); }
                    let mut max = panel.settings.config.max_size as f32;
                    let low = panel.settings.config.min_size as f32;
                    if slider(ui, "max", syfrack_i18n::tr!("Grandes régions"), &mut max, low, 512.0, true) { panel.settings.config.max_size = max as u32; }
                    let mut budget = panel.settings.config.max_regions as f32;
                    if slider(ui, "regions", syfrack_i18n::tr!("Budget de régions"), &mut budget, 64.0, 16384.0, true) { panel.settings.config.max_regions = budget as usize; }
                    ui.label(RichText::new(syfrack_i18n::tr!("Plus de finesse et de régions conserve davantage de détails.")).small().color(theme::TEXT_DIM));
                });
                ui.add_space(8.0);
                egui::CollapsingHeader::new(syfrack_i18n::tr!("Destination et agrandissement")).show(ui, |ui| {
                    ui.label(syfrack_i18n::tr!("Nom de l’export"));
                    ui.text_edit_singleline(&mut panel.name);
                    ui.label(syfrack_i18n::tr!("Dossier de sortie"));
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut panel.directory).desired_width((ui.available_width() - 36.0).max(40.0)));
                        if ui.button("…").on_hover_text(syfrack_i18n::tr!("Choisir le dossier de sortie")).clicked() {
                            file_browser::open_folder_dialog(ui, state, Path::new(&panel.directory), folder_id());
                        }
                    });
                    let pixels = panel.product.as_ref().map(|p| p.image.data.len()).unwrap_or(1);
                    let limit = (1..=4).filter(|s| pixels as u64 * (*s as u64).pow(2) <= syfrack_core::mosaic::MAX_MOSAIC_PIXELS).max().unwrap_or(1);
                    panel.scale = panel.scale.min(limit);
                    ui.horizontal(|ui| {
                        ui.label(syfrack_i18n::tr!("Agrandissement"));
                        for scale in 1..=limit { ui.selectable_value(&mut panel.scale, scale, format!("×{scale}")); }
                    });
                    if let Some(product) = &panel.product { ui.label(format!("{} × {} px", product.image.width * panel.scale, product.image.height * panel.scale)); }
                    ui.label(RichText::new(syfrack_i18n::tr!("PNG 16 bits, recettes et projet réouvrable. Les fichiers existants sont conservés.")).small().color(theme::TEXT_DIM));
                });
            });
        });
        ui.separator();
        let busy = panel.task.is_some();
        ui.horizontal(|ui| {
            let generate = ui.add_enabled(panel.raw.is_some() && !busy, egui::Button::new(RichText::new(syfrack_i18n::tr!("Générer / Actualiser")).color(theme::BG)).fill(theme::ACCENT).min_size(Vec2::new((ui.available_width() - if busy { 82.0 } else { 0.0 }).max(80.0), 32.0)));
            button_id(ui, "generate", &generate);
            if generate.clicked() { panel.generate(ui.ctx()); }
            if busy && ui.button(syfrack_i18n::tr!("Annuler")).clicked() { if let Some(task) = &panel.task { task.control.cancel(); } }
        });
        let export = ui.add_enabled(!busy && panel.current(), egui::Button::new(syfrack_i18n::tr!("Exporter la mosaïque…")).min_size(Vec2::new(ui.available_width(), 28.0)));
        button_id(ui, "export", &export);
        if export.clicked() { panel.export(ui.ctx()); }
        if let Some(task) = &panel.task {
            let (phase, done, total) = task.control.progress();
            let label = if task.control.is_cancelled() { syfrack_i18n::tr!("Annulation…") } else { match task.kind {
                Kind::Import => syfrack_i18n::tr!("Chargement de l’image…"),
                Kind::Export => syfrack_i18n::tr!("Export de la mosaïque…"),
                _ => match phase { 1 => syfrack_i18n::tr!("Recherche des fractales…"), 2 => syfrack_i18n::tr!("Reconstruction de l’image…"), 3 => syfrack_i18n::tr!("Raffinement des régions…"), _ => syfrack_i18n::tr!("Découpage de l’image…") },
            }};
            ui.add(egui::ProgressBar::new(if total > 0 { done.min(total) as f32 / total as f32 } else { 0.0 }).animate(total == 0).text(label));
        } else if panel.preview.is_some() || panel.pending_preview {
            ui.label(syfrack_i18n::tr!("Actualisation de l’aperçu…"));
        } else if panel.raw.is_some() && !panel.current() {
            ui.label(RichText::new(syfrack_i18n::tr!("Réglages prêts à générer")).small().color(theme::ACCENT));
        }
    });
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(theme::BG).inner_margin(12))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (view, label) in [
                    (View::Source, syfrack_i18n::tr!("Image ajustée")),
                    (View::Layout, syfrack_i18n::tr!("Découpage")),
                    (View::Mosaic, syfrack_i18n::tr!("Mosaïque")),
                    (View::Compare, syfrack_i18n::tr!("Comparer")),
                ] {
                    let enabled = match view {
                        View::Mosaic => panel.product.is_some(),
                        View::Compare => {
                            panel.current()
                                && panel.preview_settings.as_ref() == Some(&panel.settings)
                        }
                        _ => true,
                    };
                    let response = ui.add_enabled(
                        enabled,
                        egui::Button::new(label).selected(panel.view == view),
                    );
                    if response.clicked() {
                        panel.view = view;
                        panel.crop_mode = false;
                        panel.brush_mode = false;
                        panel.brush_active = false;
                        panel.zoom = 1.0;
                        panel.pan = Vec2::ZERO;
                    }
                }
                ui.separator();
                if ui
                    .small_button(syfrack_i18n::tr!("Ajuster à la vue"))
                    .clicked()
                {
                    panel.zoom = 1.0;
                    panel.pan = Vec2::ZERO;
                }
                ui.label(format!("{:.0}%", panel.zoom * 100.0));
            });
            if panel.crop_mode {
                ui.label(
                    RichText::new(syfrack_i18n::tr!(
                        "Tracer un rectangle sur l’image pour définir le recadrage."
                    ))
                    .color(theme::ACCENT),
                );
            } else if panel.brush_mode {
                ui.label(
                    RichText::new(syfrack_i18n::tr!(
                        "Glisser pour donner la priorité aux détails · molette : zoom"
                    ))
                    .color(theme::ACCENT),
                );
            } else if panel.view == View::Layout && !panel.current() {
                ui.label(
                    RichText::new(syfrack_i18n::tr!(
                        "Découpage estimé · générer pour appliquer le raffinement"
                    ))
                    .small()
                    .color(theme::TEXT_DIM),
                );
            } else if !panel.current() && panel.product.is_some() {
                ui.label(
                    RichText::new(syfrack_i18n::tr!(
                        "Résultat précédent · actualiser pour appliquer les nouveaux réglages"
                    ))
                    .color(theme::ACCENT),
                );
            } else {
                ui.label(
                    RichText::new(syfrack_i18n::tr!(
                        "Molette : zoom · glisser : déplacer · double-clic : ajuster"
                    ))
                    .small()
                    .color(theme::TEXT_DIM),
                );
            }
            if let Some((message, error)) = &panel.message {
                ui.add(
                    egui::Label::new(RichText::new(message).color(if *error {
                        theme::ERROR
                    } else {
                        theme::TEXT_DIM
                    }))
                    .truncate(),
                )
                .on_hover_text(message);
            }
            if panel.view == View::Compare {
                ui.add(
                    egui::Slider::new(&mut panel.compare, 0.0..=1.0)
                        .text(syfrack_i18n::tr!("Avant / après")),
                );
            }
            panel.canvas(ui);
        });
    panel.track(before, ui.input(|i| i.pointer.any_down()));
    if panel.pending_preview {
        ui.ctx().request_repaint_after(Duration::from_millis(200));
    }
}

impl MosaicPanel {
    fn paint_priority(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rect: Rect,
        painter: &egui::Painter,
    ) {
        let Some(raw) = &self.raw else {
            return;
        };
        // Editing requires the displayed crop/rotation to match the brush geometry.
        if !self
            .preview_settings
            .as_ref()
            .is_some_and(|s| s.image == self.settings.image)
        {
            return;
        }
        let raw_size = [raw.width, raw.height];
        let geometry = priority::Geometry::new(raw_size, &self.settings.image);
        let held = response.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_down());
        if self.brush_mode
            && (held || response.clicked() || (self.brush_active && response.drag_stopped()))
        {
            if let Some(pointer) = response
                .interact_pointer_pos()
                .filter(|p| rect.contains(*p))
            {
                let point = geometry.to_original([
                    (pointer.x - rect.left()) / rect.width(),
                    (pointer.y - rect.top()) / rect.height(),
                ]);
                let total: usize = self.settings.priority.iter().map(|s| s.points.len()).sum();
                if !self.brush_active
                    && self.settings.priority.len() < priority::MAX_STROKES
                    && total < priority::MAX_POINTS
                {
                    let radius = self.brush_radius
                        * geometry.adjusted_size[0].min(geometry.adjusted_size[1])
                        / geometry.original_short;
                    self.settings.priority.push(priority::BrushStroke {
                        points: vec![point],
                        radius,
                        erase: self.brush_erase,
                    });
                    self.brush_active = true;
                    self.priority_dirty = true;
                } else if self.brush_active && total < priority::MAX_POINTS {
                    let stroke = self.settings.priority.last_mut().unwrap();
                    let last = *stroke.points.last().unwrap();
                    let a = geometry.to_adjusted(last);
                    let b = geometry.to_adjusted(point);
                    let distance =
                        Vec2::new((a[0] - b[0]) * rect.width(), (a[1] - b[1]) * rect.height())
                            .length();
                    if distance > 2.0 {
                        stroke.points.push(point);
                        self.priority_dirty = true;
                    }
                }
            }
        }
        if !held {
            self.brush_active = false;
        }
        if self.priority_dirty {
            let size = geometry.adjusted_size;
            let scale = (256.0 / size[0].max(size[1])).min(1.0);
            let size = [
                (size[0] * scale).round().max(1.0) as u32,
                (size[1] * scale).round().max(1.0) as u32,
            ];
            let mask = priority::rasterize(
                raw_size,
                &self.settings.image,
                size,
                &self.settings.priority,
            );
            self.priority_texture = Some(Self::texture(
                ui.ctx(),
                "mosaic-priority",
                priority::overlay(&mask, size.map(|v| v as usize)),
            ));
            self.priority_dirty = false;
        }
        if let Some(texture) = &self.priority_texture {
            painter.image(
                texture.id(),
                rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        if self.brush_mode {
            if let Some(pointer) = response.hover_pos().filter(|p| rect.contains(*p)) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                painter.circle_stroke(
                    pointer,
                    rect.width().min(rect.height()) * self.brush_radius,
                    Stroke::new(
                        1.5,
                        if self.brush_erase {
                            Color32::WHITE
                        } else {
                            theme::ACCENT
                        },
                    ),
                );
            }
            if self.settings.priority.len() >= priority::MAX_STROKES
                || self
                    .settings
                    .priority
                    .iter()
                    .map(|s| s.points.len())
                    .sum::<usize>()
                    >= priority::MAX_POINTS
            {
                painter.text(
                    rect.left_top() + Vec2::splat(12.0),
                    egui::Align2::LEFT_TOP,
                    syfrack_i18n::tr!(
                        "Limite de traits atteinte · effacer les priorités pour recommencer"
                    ),
                    egui::FontId::proportional(14.0),
                    theme::ACCENT,
                );
            }
        }
    }

    fn canvas(&mut self, ui: &mut egui::Ui) {
        let (area, response) = ui.allocate_exact_size(
            ui.available_size().max(Vec2::splat(1.0)),
            Sense::click_and_drag(),
        );
        #[cfg(test)]
        ui.data_mut(|d| d.insert_temp(Id::new("mosaic-image-area"), area));
        let texture = if self.crop_mode {
            self.raw_texture.as_ref()
        } else {
            match self.view {
                View::Source => self.source_texture.as_ref().or(self.raw_texture.as_ref()),
                View::Layout => self.layout_texture.as_ref().or(self.raw_texture.as_ref()),
                View::Mosaic | View::Compare => self.mosaic_texture.as_ref(),
            }
        };
        let Some(texture) = texture.cloned() else {
            let text = if self.raw.is_some() {
                syfrack_i18n::tr!("Générer la mosaïque pour découvrir le résultat")
            } else {
                syfrack_i18n::tr!("Ouvrir une image pour commencer")
            };
            ui.painter().text(
                area.center(),
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(17.0),
                theme::TEXT_DIM,
            );
            return;
        };
        if response.double_clicked() && !self.brush_mode {
            self.zoom = 1.0;
            self.pan = Vec2::ZERO;
        }
        let fit = (area.width() / texture.size_vec2().x).min(area.height() / texture.size_vec2().y);
        if response.hovered() && !self.crop_mode {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let old = self.zoom;
                self.zoom = (self.zoom * (scroll * 0.003).exp()).clamp(1.0, 16.0);
                if let Some(pointer) = response.hover_pos() {
                    let offset = pointer - area.center();
                    self.pan = (self.pan - offset) * (self.zoom / old) + offset;
                }
            }
        }
        if response.dragged() && !self.crop_mode && !self.brush_mode {
            self.pan += ui.input(|i| i.pointer.delta());
        }
        if self.zoom == 1.0 {
            self.pan = Vec2::ZERO;
        }
        let rect = Rect::from_center_size(
            area.center() + self.pan,
            texture.size_vec2() * fit * self.zoom,
        );
        let painter = ui.painter().with_clip_rect(area);
        // Checkerboard shows alpha without modifying either image or export.
        painter.rect_filled(area, 4.0, theme::PANEL);
        let checker = rect.intersect(area);
        let step = 20.0;
        for y in 0..(checker.height() / step).ceil().max(0.0) as i32 {
            for x in 0..(checker.width() / step).ceil().max(0.0) as i32 {
                let r = Rect::from_min_size(
                    checker.min + Vec2::new(x as f32 * step, y as f32 * step),
                    Vec2::splat(step),
                )
                .intersect(checker);
                painter.rect_filled(
                    r,
                    0.0,
                    if (x + y) % 2 == 0 {
                        Color32::from_gray(28)
                    } else {
                        Color32::from_gray(40)
                    },
                );
            }
        }
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        painter.image(texture.id(), rect, uv, Color32::WHITE);
        if !self.crop_mode
            && (self.brush_mode
                || (self.show_priority && matches!(self.view, View::Source | View::Layout)))
        {
            self.paint_priority(ui, &response, rect, &painter);
        }
        if self.view == View::Compare && !self.crop_mode {
            if let Some(source) = &self.source_texture {
                let split = rect.left() + rect.width() * self.compare;
                painter
                    .with_clip_rect(area.intersect(Rect::from_min_max(
                        area.min,
                        Pos2::new(split, area.bottom()),
                    )))
                    .image(source.id(), rect, uv, Color32::WHITE);
                painter.line_segment(
                    [
                        Pos2::new(split, rect.top()),
                        Pos2::new(split, rect.bottom()),
                    ],
                    Stroke::new(2.0, theme::ACCENT),
                );
            }
        }
        if self.crop_mode {
            let normalise = |p: Pos2| {
                Pos2::new(
                    ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0),
                    ((p.y - rect.top()) / rect.height()).clamp(0.0, 1.0),
                )
            };
            if response.drag_started() {
                self.crop_start = ui
                    .input(|i| i.pointer.press_origin())
                    .filter(|p| rect.contains(*p))
                    .map(normalise);
            }
            let mut crop = self.settings.image.crop;
            if let (Some(start), Some(pointer)) = (self.crop_start, response.interact_pointer_pos())
            {
                let end = normalise(pointer);
                crop = [
                    start.x.min(end.x),
                    start.y.min(end.y),
                    start.x.max(end.x),
                    start.y.max(end.y),
                ];
                if response.drag_stopped() {
                    if let Some(raw) = &self.raw {
                        if (crop[2] - crop[0]) * raw.width as f32 >= 2.0
                            && (crop[3] - crop[1]) * raw.height as f32 >= 2.0
                        {
                            self.settings.image.crop = crop;
                        }
                    }
                    self.crop_start = None;
                }
            }
            let selected = Rect::from_min_max(
                rect.min + Vec2::new(crop[0] * rect.width(), crop[1] * rect.height()),
                rect.min + Vec2::new(crop[2] * rect.width(), crop[3] * rect.height()),
            );
            painter.rect_stroke(
                selected,
                0.0,
                Stroke::new(2.0, theme::ACCENT),
                egui::StrokeKind::Inside,
            );
        }
        painter.rect_stroke(
            rect,
            0.0,
            Stroke::new(1.0, theme::SEPARATOR),
            egui::StrokeKind::Inside,
        );
    }
}
