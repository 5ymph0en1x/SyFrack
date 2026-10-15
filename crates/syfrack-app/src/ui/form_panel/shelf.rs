//! The library column: the graphs of `graphs/` as thumbnails to load with a
//! click or to drop on the canvas, and « Enregistrer la forme… ».

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use egui::{LayerId, Order, TextureHandle, Vec2};

use super::*;
use crate::gallery::Thumbnails;
use crate::library::{self, LibraryEntry};
use crate::state::Banner;

/// The folder is scanned again this often (as the gallery).
const RESCAN_PERIOD: Duration = Duration::from_secs(2);
/// Thumbnail box of an entry, 16:9.
const THUMB_W: f32 = 20.0 * theme::GRID;
const THUMB_H: f32 = THUMB_W * 9.0 / 16.0;
/// Height kept under the list for the save button.
const FOOTER_H: f32 = 6.0 * theme::GRID;

/// What a library entry carries while it is dragged onto the canvas.
#[derive(Clone)]
pub struct LibraryDrag {
    pub name: String,
    pub graph: Graph,
}

/// The « Enregistrer la forme » dialog: the name typed, and whether its
/// file exists (then « Remplacer » is offered).
#[derive(Clone, Default)]
struct SaveDialog {
    name: String,
    exists: bool,
    /// First frame: the name field takes the keyboard.
    fresh: bool,
}

pub struct Library {
    pub(super) compact: bool,
    dir: PathBuf,
    scanned: Option<Instant>,
    pub entries: Vec<LibraryEntry>,
    thumbs: Thumbnails,
    textures: HashMap<PathBuf, (Option<SystemTime>, TextureHandle)>,
    /// Entry whose loading waits for « ouvrir quand même ».
    /// The entry itself, not its index: a rescan under the modal cannot
    /// swap it for another form.
    confirm_load: Option<LibraryEntry>,
    save: Option<SaveDialog>,
    /// Thumbnails to render (graph, PNG path): the app has the GPU.
    pub thumbnail_jobs: Vec<(Graph, PathBuf)>,
}

impl Default for Library {
    fn default() -> Self {
        Self::with_dir(library::default_dir())
    }
}

impl Library {
    pub fn with_dir(dir: PathBuf) -> Self {
        Self {
            compact: false,
            dir,
            scanned: None,
            entries: Vec::new(),
            thumbs: Thumbnails::new(),
            textures: HashMap::new(),
            confirm_load: None,
            save: None,
            thumbnail_jobs: Vec::new(),
        }
    }

    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    /// Scans the folder when it is due (every 2 s), or at once after
    /// `rescan_now`.
    pub fn refresh(&mut self) {
        if self.scanned.is_some_and(|at| at.elapsed() < RESCAN_PERIOD) {
            return;
        }
        self.entries = library::scan(&self.dir);
        let pngs: HashSet<PathBuf> = self.entries.iter().filter_map(|e| e.png.clone()).collect();
        for png in &pngs {
            self.thumbs.request(png);
        }
        self.thumbs.forget_except(&pngs);
        self.textures.retain(|p, _| pngs.contains(p));
        self.scanned = Some(Instant::now());
    }

    /// Opens « Enregistrer la forme », its name field focused.
    pub fn open_save_dialog(&mut self) {
        self.save = Some(SaveDialog {
            fresh: true,
            ..Default::default()
        });
    }

    /// The save dialog is open.
    pub fn saving(&self) -> bool {
        self.save.is_some()
    }

    /// The next `refresh` scans (a file was written).
    pub fn rescan_now(&mut self) {
        self.scanned = None;
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
                        format!("library-{}", png.display()),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    self.textures.insert(png.clone(), (stamp, tex));
                }
            }
        }
    }

    /// The column: entries, then the save button. Clicks load, drags carry
    /// a `LibraryDrag` to the canvas.
    pub fn show(&mut self, ui: &mut Ui, state: &mut AppState) {
        self.refresh();
        self.sync_textures(ui.ctx());
        ui.heading(syfrack_i18n::tr!("Bibliothèque"));
        ui.add_space(theme::GRID);
        let list_h = (ui.available_height() - FOOTER_H).max(THUMB_H);
        let mut load = None;
        egui::ScrollArea::vertical()
            .id_salt("library-list")
            .max_height(list_h)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                if self.entries.is_empty() {
                    ui.label(
                        RichText::new(syfrack_i18n::tr_format!(
                            "Aucune forme dans {}",
                            self.dir.display()
                        ))
                        .color(theme::TEXT_DIM),
                    );
                }
                for (i, e) in self.entries.iter().enumerate() {
                    if self.entry(ui, e) {
                        load = Some(i);
                    }
                    ui.add_space(theme::GRID);
                }
            });
        if let Some(e) = load.and_then(|i| self.entries.get(i)).cloned() {
            if state.is_modified() {
                self.confirm_load = Some(e);
            } else {
                load_entry(state, &e);
            }
        }
        ui.add_space(theme::GRID);
        if ui
            .button(syfrack_i18n::tr!("Enregistrer la forme…"))
            .clicked()
        {
            self.open_save_dialog();
        }
        drag_ghost(ui);
    }

    pub fn dialogs(&mut self, ui: &mut Ui, state: &mut AppState) {
        if self.confirm_load.is_some() {
            match crate::ui::confirm_discard(ui.ctx(), Id::new("library-load-confirm")) {
                Some(true) => {
                    if let Some(e) = self.confirm_load.take() {
                        load_entry(state, &e);
                    }
                }
                Some(false) => self.confirm_load = None,
                None => {}
            }
        }
        self.save_dialog(ui, state);
    }

    /// One entry: thumbnail, name, node count. Returns true when clicked.
    fn entry(&self, ui: &mut Ui, e: &LibraryEntry) -> bool {
        let w = ui.available_width();
        let thumb_w = if self.compact { 80.0 } else { THUMB_W };
        let thumb_h = if self.compact { 56.0 } else { THUMB_H };
        let (rect, resp) = ui.allocate_exact_size(
            Vec2::new(w, thumb_h),
            if self.compact {
                Sense::click()
            } else {
                Sense::click_and_drag()
            },
        );
        let resp = resp.on_hover_text(format!("{}\n{}", e.localized_name(), e.path.display()));
        #[cfg(test)]
        ui.data_mut(|d| d.insert_temp(Id::new(("library-entry", &e.name)), rect));
        let painter = ui.painter_at(rect);
        let thumb = Rect::from_min_size(rect.min, Vec2::new(thumb_w, thumb_w * 9.0 / 16.0));
        let texture = e.png.as_ref().and_then(|p| self.textures.get(p));
        match texture {
            Some((_, tex)) => {
                let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                painter.image(tex.id(), thumb, uv, Color32::WHITE);
            }
            None => {
                painter.rect_filled(thumb, 2.0, theme::PANEL);
                painter.text(
                    thumb.center(),
                    Align2::CENTER_CENTER,
                    e.localized_name(),
                    FontId::proportional(11.0),
                    theme::TEXT_DIM,
                );
            }
        }
        let border = if resp.hovered() {
            theme::ACCENT
        } else {
            theme::SEPARATOR
        };
        painter.rect_stroke(thumb, 2.0, Stroke::new(1.0, border), StrokeKind::Inside);
        let x = thumb.right() + theme::GRID;
        painter.text(
            Pos2::new(x, rect.top() + 2.0 * theme::GRID),
            Align2::LEFT_CENTER,
            e.localized_name(),
            FontId::proportional(13.0),
            theme::TEXT,
        );
        painter.text(
            Pos2::new(x, rect.top() + 4.5 * theme::GRID),
            Align2::LEFT_CENTER,
            syfrack_i18n::tr_format!("{} nœuds", e.nodes),
            FontId::proportional(11.0),
            theme::TEXT_DIM,
        );
        if !self.compact && resp.drag_started() {
            egui::DragAndDrop::set_payload(
                ui.ctx(),
                LibraryDrag {
                    name: e.localized_name().to_owned(),
                    graph: e.graph.clone(),
                },
            );
        }
        resp.clicked()
    }

    /// The entry waiting for « ouvrir quand même ».
    pub fn confirming(&self) -> Option<&LibraryEntry> {
        self.confirm_load.as_ref()
    }

    fn save_dialog(&mut self, ui: &mut Ui, state: &mut AppState) {
        let Some(mut dialog) = self.save.take() else {
            return;
        };
        let mut close = false;
        let mut open = true;
        let mut write = None;
        crate::ui::floating::Window::new(syfrack_i18n::tr!("Enregistrer la forme"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ui.ctx(), |ui| {
                let field = ui.add(
                    egui::TextEdit::singleline(&mut dialog.name)
                        .hint_text(syfrack_i18n::tr!("nom de la forme"))
                        .desired_width(40.0 * theme::GRID),
                );
                if std::mem::take(&mut dialog.fresh) {
                    field.request_focus();
                }
                if field.changed() {
                    dialog.exists = false;
                }
                if ui.input(|i| i.key_pressed(Key::Escape)) {
                    close = true;
                }
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                let file = self
                    .dir
                    .join(format!("{}.ron", library::slug(&dialog.name)));
                ui.label(RichText::new(file.display().to_string()).color(theme::TEXT_DIM));
                // A shipped form's name is refused outright: no « Remplacer ».
                let shipped = library::refuse_bundled(&dialog.name).err();
                if let Some(e) = &shipped {
                    ui.label(RichText::new(e.to_string()).color(theme::ERROR));
                } else if dialog.exists {
                    ui.label(
                        RichText::new(syfrack_i18n::tr!("Ce fichier existe déjà."))
                            .color(theme::ERROR),
                    );
                }
                ui.horizontal(|ui| {
                    let named = !dialog.name.trim().is_empty() && shipped.is_none();
                    if (ui.button(syfrack_i18n::tr!("Enregistrer")).clicked() || enter) && named {
                        write = Some(false);
                    }
                    if dialog.exists
                        && shipped.is_none()
                        && ui.button(syfrack_i18n::tr!("Remplacer")).clicked()
                    {
                        write = Some(true);
                    }
                    if ui.button(syfrack_i18n::tr!("Annuler")).clicked() {
                        close = true;
                    }
                });
            });
        if let Some(overwrite) = write {
            let g = state.scene.fractal.to_graph();
            let name = dialog.name.trim().to_string();
            let file = self.dir.join(format!("{}.ron", library::slug(&name)));
            if !overwrite && file.exists() {
                dialog.exists = true;
            } else {
                let saved = if overwrite {
                    library::overwrite(&self.dir, &name, &g)
                } else {
                    library::save(&self.dir, &name, &g)
                };
                match saved {
                    Ok(path) => {
                        // An incomplete form is kept without a thumbnail.
                        if g.validate().is_ok() {
                            self.thumbnail_jobs.push((g, path.with_extension("png")));
                        }
                        state.banner = Some(Banner::info(syfrack_i18n::tr_format!(
                            "forme enregistrée {}",
                            path.display()
                        )));
                        self.rescan_now();
                        close = true;
                    }
                    Err(e) => state.banner = Some(Banner::error(format!("{e:#}"))),
                }
            }
        }
        if !close && open {
            self.save = Some(dialog);
        }
    }
}

/// Replaces the form with `e` (one undo step) and frames it.
fn load_entry(state: &mut AppState, e: &LibraryEntry) {
    let g = e.graph.clone();
    let view = view_for(&g);
    state.edit(|s| s.fractal = Fractal::Graph(g));
    state.form.selected.clear();
    state.form.view = view;
    state.banner = Some(Banner::info(syfrack_i18n::tr_format!(
        "forme « {} » chargée",
        e.localized_name()
    )));
}

/// The name of a dragged entry follows the pointer.
fn drag_ghost(ui: &Ui) {
    let Some(drag) = egui::DragAndDrop::payload::<LibraryDrag>(ui.ctx()) else {
        return;
    };
    let Some(p) = ui.input(|i| i.pointer.latest_pos()) else {
        return;
    };
    let painter = ui
        .ctx()
        .layer_painter(LayerId::new(Order::Tooltip, Id::new("library-ghost")));
    let galley = painter.layout_no_wrap(drag.name.clone(), FontId::proportional(13.0), theme::TEXT);
    let rect = Rect::from_min_size(p + egui::vec2(12.0, 12.0), galley.size()).expand(4.0);
    painter.rect_filled(rect, 2.0, theme::PANEL);
    painter.rect_stroke(
        rect,
        2.0,
        Stroke::new(1.0, theme::ACCENT),
        StrokeKind::Inside,
    );
    painter.galley(rect.min + egui::vec2(4.0, 4.0), galley, theme::TEXT);
}
