//! Studio file explorer: directory reads off the UI thread, explicit selection,
//! persistent errors, and the same unsaved-document protection as the editor.
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use egui::{Id, Key, RichText};

use super::{confirm_discard, theme};
use crate::state::AppState;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum FileAction {
    Open,
    SaveAs,
    Hdri(Option<usize>),
    Image(Id),
    Folder(Id),
}

#[derive(Clone, Debug)]
struct Entry {
    path: PathBuf,
    name: String,
    folder: bool,
    size: u64,
}
type Listing = Result<Vec<Entry>, String>;
type Pending = Arc<Mutex<Option<Listing>>>;

fn accepts(action: FileAction, path: &Path) -> bool {
    path.extension().is_some_and(|ext| match action {
        FileAction::Folder(_) => false,
        FileAction::Hdri(_) => ext.eq_ignore_ascii_case("hdr") || ext.eq_ignore_ascii_case("exr"),
        FileAction::Image(_) => {
            ["png", "jpg", "jpeg", "bmp", "tif", "tiff"]
                .iter()
                .any(|v| ext.eq_ignore_ascii_case(v))
                || path.file_name().is_some_and(|s| {
                    s.to_string_lossy()
                        .to_lowercase()
                        .ends_with(".mosaic-project.ron")
                })
        }
        _ => ext.eq_ignore_ascii_case("ron"),
    })
}

fn scan(dir: &Path, action: FileAction) -> Listing {
    let mut entries = vec![];
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_dir() && !meta.is_file() {
            continue;
        }
        let path = entry.path();
        if !meta.is_dir() && !accepts(action, &path) {
            continue;
        }
        entries.push(Entry {
            name: entry.file_name().to_string_lossy().into(),
            path,
            folder: meta.is_dir(),
            size: meta.len(),
        });
    }
    entries.sort_by(|a, b| {
        b.folder
            .cmp(&a.folder)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

#[derive(Clone)]
pub(super) struct FileDialog {
    pub(super) action: FileAction,
    dir: PathBuf,
    root: PathBuf,
    location: String,
    editing_location: bool,
    focus_location: bool,
    pub(super) filename: String,
    selected: Option<PathBuf>,
    query: String,
    entries: Vec<Entry>,
    pending: Option<Pending>,
    error: Option<String>,
    back: Vec<PathBuf>,
    confirming: bool,
    overwrite: bool,
    /// Capture the target while a confirmation is open.
    target: Option<PathBuf>,
}

pub(super) fn dialog_id() -> Id {
    Id::new("file-dialog")
}

impl FileDialog {
    fn new(state: &AppState, action: FileAction) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let root = crate::paths::content_root();
        let path = state.path.as_ref().map(|p| {
            if p.is_absolute() {
                p.clone()
            } else {
                cwd.join(p)
            }
        });
        let dir = path
            .as_ref()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf)
            .filter(|p| p.is_dir())
            .unwrap_or_else(|| {
                if root.join("scenes").is_dir() {
                    root.join("scenes")
                } else {
                    root.clone()
                }
            });
        let filename = if action == FileAction::SaveAs {
            path.as_ref()
                .and_then(|p| p.file_name())
                .map(|s| s.to_string_lossy().into())
                .unwrap_or_else(|| syfrack_i18n::tr!("ma-scène.ron").into())
        } else {
            String::new()
        };
        let mut this = Self {
            action,
            dir: dir.clone(),
            root,
            location: String::new(),
            editing_location: false,
            focus_location: false,
            filename,
            selected: None,
            query: String::new(),
            entries: vec![],
            pending: None,
            error: None,
            back: vec![],
            confirming: false,
            overwrite: false,
            target: None,
        };
        this.navigate(dir, false);
        this
    }

    fn navigate(&mut self, dir: PathBuf, remember: bool) {
        if !dir.is_dir() {
            self.error = Some(
                syfrack_i18n::tr!(
                    "Ce dossier est introuvable ou inaccessible. Vérifiez le chemin."
                )
                .into(),
            );
            return;
        }
        if remember && self.dir != dir {
            self.back.push(self.dir.clone());
        }
        self.dir = dir.clone();
        self.location = dir.display().to_string();
        self.editing_location = false;
        self.query.clear();
        self.selected = None;
        self.entries.clear();
        self.error = None;
        if self.action != FileAction::SaveAs {
            self.filename.clear();
        }
        let result = Arc::new(Mutex::new(None));
        self.pending = Some(result.clone());
        let action = self.action;
        std::thread::spawn(move || {
            let listing = scan(&dir, action);
            if let Ok(mut slot) = result.lock() {
                *slot = Some(listing);
            }
        });
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let result = self.pending.as_ref().and_then(|p| p.lock().ok()?.take());
        if let Some(result) = result {
            self.pending = None;
            match result {
                Ok(entries) => self.entries = entries,
                Err(e) => {
                    self.error = Some(syfrack_i18n::tr_format!(
                        "Impossible de lire ce dossier · {e}",
                        e = e
                    ))
                }
            }
        }
        if self.pending.is_some() {
            ctx.request_repaint_after(Duration::from_millis(40));
        }
    }

    fn resolve(&self) -> Result<PathBuf, String> {
        if matches!(self.action, FileAction::Folder(_)) {
            let path = self.selected.as_ref().unwrap_or(&self.dir);
            return if path.is_dir() {
                Ok(path.clone())
            } else {
                Err(syfrack_i18n::tr!("Ce dossier n’existe plus.").into())
            };
        }
        let name = self.filename.trim();
        if name.is_empty() {
            return Err(syfrack_i18n::tr!("Sélectionner un fichier ou saisir son nom").into());
        }
        let mut path = self.dir.join(name);
        if path.is_dir() {
            return Err(syfrack_i18n::tr!("Ce nom désigne un dossier").into());
        }
        if self.action == FileAction::SaveAs && path.extension().is_none() {
            path.set_extension("ron");
        }
        if !accepts(self.action, &path) {
            return Err(match self.action {
                FileAction::Image(_) => {
                    syfrack_i18n::tr!("Choisir une image PNG, JPEG, BMP ou TIFF")
                }
                FileAction::Hdri(_) => {
                    syfrack_i18n::tr!("Un panorama HDRI utilise l’extension .hdr ou .exr")
                }
                _ => syfrack_i18n::tr!("Une scène SyFrack utilise l’extension .ron"),
            }
            .into());
        }
        if self.action != FileAction::SaveAs && !path.is_file() {
            return Err(syfrack_i18n::tr!("Le fichier sélectionné n’existe pas").into());
        }
        Ok(path)
    }

    fn run(&mut self, state: &mut AppState, ctx: &egui::Context) -> bool {
        let Some(path) = &self.target else {
            return false;
        };
        let done = match self.action {
            FileAction::Image(result_id) => {
                ctx.data_mut(|d| d.insert_temp(result_id, path.clone()));
                true
            }
            FileAction::Folder(result_id) => {
                ctx.data_mut(|d| d.insert_temp(result_id, path.clone()));
                true
            }
            FileAction::Open => state.load(path),
            FileAction::SaveAs => state.save_as(path),
            FileAction::Hdri(index) => {
                if index.is_none() && state.scene.lights.len() >= syfrack_core::MAX_LIGHTS {
                    self.error =
                        Some(syfrack_i18n::tr!("Nombre maximal de lumières atteint").into());
                    return false;
                }
                if index.is_some_and(|i| {
                    !matches!(
                        state.scene.lights.get(i),
                        Some(syfrack_core::Light::Hdri { .. })
                    )
                }) {
                    self.error = Some(syfrack_i18n::tr!("Cette lumière HDRI n’existe plus").into());
                    return false;
                }
                let path = path.clone();
                state.edit(|scene| {
                    if let Some(i) = index {
                        if let syfrack_core::Light::Hdri { path: current, .. } =
                            &mut scene.lights[i]
                        {
                            *current = path;
                        }
                    } else {
                        scene.lights.push(syfrack_core::Light::Hdri {
                            path,
                            intensity: 1.0,
                            rotation: 0.0,
                        });
                    }
                });
                state.refresh_preview();
                true
            }
        };
        if !done {
            self.error = state.banner.as_ref().map(|b| b.text.clone());
        }
        done
    }
}

pub(super) fn open_dialog(ui: &egui::Ui, state: &AppState, action: FileAction) {
    ui.data_mut(|d| d.insert_temp(dialog_id(), FileDialog::new(state, action)));
}

pub(super) fn open_folder_dialog(ui: &egui::Ui, state: &AppState, initial: &Path, result_id: Id) {
    let mut dialog = FileDialog::new(state, FileAction::Folder(result_id));
    let initial = std::path::absolute(initial).unwrap_or_else(|_| dialog.root.clone());
    if let Some(existing) = initial.ancestors().find(|p| p.is_dir()) {
        dialog.navigate(existing.to_path_buf(), false);
    }
    ui.data_mut(|d| {
        d.remove::<PathBuf>(result_id);
        d.insert_temp(dialog_id(), dialog);
    });
}

pub(super) fn open_image_dialog(ui: &egui::Ui, state: &AppState, initial: &Path, result_id: Id) {
    let mut dialog = FileDialog::new(state, FileAction::Image(result_id));
    let directory = if initial.is_file() {
        initial.parent().unwrap_or(initial)
    } else {
        initial
    };
    if directory.is_dir() {
        dialog.navigate(directory.to_path_buf(), false);
    }
    ui.data_mut(|d| {
        d.remove::<PathBuf>(result_id);
        d.insert_temp(dialog_id(), dialog);
    });
}

fn places(root: &Path) -> Vec<(&'static str, PathBuf)> {
    let mut places = vec![
        ("SyFrack", root.to_path_buf()),
        (syfrack_i18n::tr!("Scènes"), root.join("scenes")),
        (syfrack_i18n::tr!("Rendus"), root.join("renders")),
    ];
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        let home = PathBuf::from(home);
        places.push((syfrack_i18n::tr!("Personnel"), home.clone()));
        places.push(("Documents", home.join("Documents")));
        places.push((syfrack_i18n::tr!("Bureau"), home.join("Desktop")));
    }
    if let Some(drive) = root.ancestors().last() {
        places.push((syfrack_i18n::tr!("Lecteur"), drive.to_path_buf()));
    }
    places
}

/// A workspace-relative trail; outside it, retain the drive or network share.
fn breadcrumbs(dir: &Path, root: &Path) -> Vec<(String, PathBuf)> {
    let mut paths: Vec<_> = dir.ancestors().collect();
    if let Some(index) = paths.iter().position(|p| *p == root) {
        paths.truncate(index + 1);
    }
    paths
        .into_iter()
        .rev()
        .map(|p| {
            let name = if p == root {
                "SyFrack".into()
            } else {
                p.file_name().map_or_else(
                    || p.display().to_string(),
                    |s| s.to_string_lossy().into_owned(),
                )
            };
            (name, p.to_path_buf())
        })
        .collect()
}

fn address_bar(ui: &mut egui::Ui, dialog: &mut FileDialog) -> Option<(PathBuf, bool)> {
    let mut destination = None;
    ui.horizontal(|ui| {
        let width = ui.available_width() - 104.0;
        if dialog.editing_location {
            let field = ui.add_sized(
                [width, 32.0],
                egui::TextEdit::singleline(&mut dialog.location)
                    .id(Id::new("file-location"))
                    .margin(egui::vec2(8.0, 6.0))
                    .hint_text(syfrack_i18n::tr!("Saisir ou coller un chemin…")),
            );
            if dialog.focus_location {
                field.request_focus();
                if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), field.id) {
                    state
                        .cursor
                        .set_char_range(Some(egui::text::CCursorRange::two(
                            egui::text::CCursor::new(0),
                            egui::text::CCursor::new(dialog.location.chars().count()),
                        )));
                    state.store(ui.ctx(), field.id);
                }
                dialog.focus_location = false;
            }
            if ui
                .add_sized([96.0, 32.0], egui::Button::new(syfrack_i18n::tr!("Aller")))
                .clicked()
                || (field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)))
            {
                destination = Some((
                    dialog.dir.join(dialog.location.trim().trim_matches('"')),
                    true,
                ));
            }
        } else {
            let trail = breadcrumbs(&dialog.dir, &dialog.root);
            egui::Frame::NONE
                .fill(theme::BG)
                .stroke(egui::Stroke::new(1.0, theme::SEPARATOR))
                .corner_radius(4.0)
                .inner_margin(egui::Margin::symmetric(8, 2))
                .show(ui, |ui| {
                    ui.set_width((width - 18.0).max(100.0));
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let font = egui::TextStyle::Button.resolve(ui.style());
                        let widths: Vec<f32> = trail
                            .iter()
                            .map(|(name, _)| {
                                ui.fonts_mut(|f| {
                                    f.layout_no_wrap(name.clone(), font.clone(), theme::TEXT)
                                        .size()
                                        .x
                                }) + 40.0
                            })
                            .collect();
                        let mut start = 0;
                        while start + 1 < trail.len()
                            && widths[start..].iter().sum::<f32>()
                                + if start > 0 { 48.0 } else { 0.0 }
                                > ui.available_width()
                        {
                            start += 1;
                        }
                        if start > 0 {
                            ui.menu_button("…", |ui| {
                                for (name, path) in &trail[..start] {
                                    if ui
                                        .button(name)
                                        .on_hover_text(path.display().to_string())
                                        .clicked()
                                    {
                                        destination = Some((path.clone(), true));
                                        ui.close();
                                    }
                                }
                            })
                            .response
                            .on_hover_text(syfrack_i18n::tr!("Dossiers parents"));
                            breadcrumb_separator(ui);
                        }
                        for (index, (name, path)) in trail.iter().enumerate().skip(start) {
                            if index > start {
                                breadcrumb_separator(ui);
                            }
                            let current = index + 1 == trail.len();
                            let button = egui::Button::new(RichText::new(name).color(if current {
                                theme::TEXT
                            } else {
                                theme::TEXT_DIM
                            }))
                            .frame(false)
                            .truncate();
                            if ui
                                .add(button)
                                .on_hover_text(path.display().to_string())
                                .clicked()
                            {
                                destination = Some((path.clone(), true));
                            }
                        }
                    });
                })
                .response
                .on_hover_text(dialog.dir.display().to_string());
            if ui
                .add_sized(
                    [96.0, 32.0],
                    egui::Button::new(syfrack_i18n::tr!("Chemin…")),
                )
                .on_hover_text(syfrack_i18n::tr!(
                    "Saisir ou copier le chemin complet · Ctrl+L"
                ))
                .clicked()
            {
                dialog.editing_location = true;
                dialog.focus_location = true;
            }
        }
    });
    destination
}

fn breadcrumb_separator(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 26.0), egui::Sense::hover());
    let center = rect.center();
    ui.painter().add(egui::Shape::line(
        vec![
            center + egui::vec2(-2.0, -4.0),
            center + egui::vec2(2.0, 0.0),
            center + egui::vec2(-2.0, 4.0),
        ],
        egui::Stroke::new(1.2, theme::TEXT_DIM),
    ));
}

fn size(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        syfrack_i18n::tr_format!("{:.1} Mio", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        syfrack_i18n::tr_format!("{:.1} Kio", bytes as f64 / 1024.0)
    } else {
        syfrack_i18n::tr_format!("{bytes} o", bytes = bytes)
    }
}

pub(super) fn file_dialog(ui: &mut egui::Ui, state: &mut AppState) {
    let Some(mut dialog) = ui.data(|d| d.get_temp::<FileDialog>(dialog_id())) else {
        return;
    };
    dialog.poll(ui.ctx());
    // Consume Escape before the native window does: it first cancels address
    // editing, and a second press closes the explorer.
    if !dialog.confirming && !dialog.overwrite && !super::floating::blocked(ui.ctx()) {
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::L)) {
            dialog.editing_location = true;
            dialog.focus_location = true;
        }
        if dialog.editing_location
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape))
        {
            dialog.editing_location = false;
            dialog.location = dialog.dir.display().to_string();
        }
    }
    let (title, verb) = match dialog.action {
        FileAction::Image(_) => (
            syfrack_i18n::tr!("Choisir une image"),
            syfrack_i18n::tr!("Utiliser"),
        ),
        FileAction::Open => (
            syfrack_i18n::tr!("Ouvrir un projet"),
            syfrack_i18n::tr!("Ouvrir"),
        ),
        FileAction::SaveAs => (
            syfrack_i18n::tr!("Enregistrer le projet sous"),
            syfrack_i18n::tr!("Enregistrer"),
        ),
        FileAction::Hdri(_) => (
            syfrack_i18n::tr!("Choisir un panorama HDRI"),
            syfrack_i18n::tr!("Utiliser"),
        ),
        FileAction::Folder(_) => (
            syfrack_i18n::tr!("Choisir le dossier de sortie"),
            syfrack_i18n::tr!("Choisir ce dossier"),
        ),
    };
    let mut open = true;
    let mut close = false;
    let mut go = false;
    let max = ui.ctx().content_rect().size();
    crate::ui::floating::Window::new(title)
        .id(dialog_id())
        .modal(matches!(dialog.action, FileAction::Folder(_) | FileAction::Image(_)))
        .open(&mut open)
        .collapsible(false)
        .default_width(780.0)
        .min_width(580.0)
        .min_height(380.0)
        .max_width((max.x - 40.0).max(580.0))
        .show(ui.ctx(), |ui| {
            ui.add_enabled_ui(!dialog.confirming && !dialog.overwrite, |ui| {
                let mut destination = None;
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!dialog.back.is_empty(), egui::Button::new(syfrack_i18n::tr!("Retour")))
                        .clicked()
                    {
                        destination = dialog.back.pop().map(|p| (p, false));
                    }
                    if ui
                        .add_enabled(dialog.dir.parent().is_some(), egui::Button::new("Parent"))
                        .clicked()
                    {
                        destination = dialog.dir.parent().map(|p| (p.to_path_buf(), true));
                    }
                    if ui.button(syfrack_i18n::tr!("Actualiser")).clicked() {
                        destination = Some((dialog.dir.clone(), false));
                    }
                });
                destination = address_bar(ui, &mut dialog).or(destination);
                ui.add_space(6.0);
                ui.separator();
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(140.0);
                        ui.label(RichText::new(syfrack_i18n::tr!("EMPLACEMENTS")).small().color(theme::TEXT_DIM));
                        for (name, path) in
                            places(&dialog.root).into_iter().filter(|(_, p)| p.is_dir())
                        {
                            if ui
                                .add_sized(
                                    [140.0, 32.0],
                                    egui::Button::new("")
                                        .left_text(name)
                                        .selected(dialog.dir == path)
                                        .frame(dialog.dir == path),
                                )
                                .on_hover_text(path.display().to_string())
                                .clicked()
                            {
                                destination = Some((path, true));
                            }
                        }
                    });
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.set_min_width(ui.available_width());
                        ui.add(
                            egui::TextEdit::singleline(&mut dialog.query)
                                .hint_text(syfrack_i18n::tr!("Filtrer les noms…"))
                                .margin(egui::vec2(8.0, 7.0))
                                .desired_width(ui.available_width()),
                        );
                        ui.horizontal(|ui| {
                            ui.allocate_ui_with_layout(
                                egui::vec2(ui.available_width() - 145.0, 22.0),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.set_min_width(ui.available_width());
                                    ui.label(RichText::new(syfrack_i18n::tr!("Nom")).strong());
                                },
                            );
                            ui.label(RichText::new(syfrack_i18n::tr!("Type · Taille")).color(theme::TEXT_DIM));
                        });
                        let height = if super::floating::is_secondary(ui.ctx()) {
                            (max.y - ui.cursor().min.y - 158.0).max(100.0)
                        } else {
                            (max.y - 350.0).clamp(180.0, 370.0)
                        };
                        egui::ScrollArea::vertical()
                            .id_salt("file-entries")
                            .min_scrolled_height(height)
                            .max_height(height)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                let query = dialog.query.to_lowercase();
                                if dialog.pending.is_some() {
                                    ui.label(syfrack_i18n::tr!("Lecture du dossier…"));
                                }
                                let mut count = 0;
                                for entry in dialog
                                    .entries
                                    .iter()
                                    .filter(|e| e.name.to_lowercase().contains(&query))
                                {
                                    count += 1;
                                    let selected = dialog.selected.as_ref() == Some(&entry.path)
                                        || (!entry.folder && dialog.filename == entry.name);
                                    ui.horizontal(|ui| {
                                        let row = ui
                                            .add_sized(
                                                [ui.available_width() - 145.0, 27.0],
                                                egui::Button::new("")
                                                    .left_text(&entry.name)
                                                    .truncate()
                                                    .selected(selected)
                                                    .frame(selected),
                                            )
                                            .on_hover_text(&entry.name);
                                        #[cfg(test)]
                                        ui.data_mut(|d| {
                                            d.insert_temp(
                                                Id::new(("file-entry", entry.name.as_str())),
                                                row.rect,
                                            )
                                        });
                                        if row.clicked() {
                                            dialog.selected = Some(entry.path.clone());
                                            if entry.folder {
                                                dialog.filename.clear();
                                            } else {
                                                dialog.filename = entry.name.clone();
                                            }
                                        }
                                        if row.double_clicked() {
                                            if entry.folder {
                                                destination = Some((entry.path.clone(), true));
                                            } else {
                                                dialog.filename = entry.name.clone();
                                                go = true;
                                            }
                                        }
                                        ui.label(
                                            RichText::new(if entry.folder {
                                                syfrack_i18n::tr!("Dossier").into()
                                            } else {
                                                format!(
                                                    "{} · {}",
                                                    if matches!(dialog.action, FileAction::Image(_)) {
                                                        syfrack_i18n::tr!("Image")
                                                    } else if matches!(dialog.action, FileAction::Hdri(_))
                                                    {
                                                        "HDRI"
                                                    } else {
                                                        syfrack_i18n::tr!("Scène")
                                                    },
                                                    size(entry.size)
                                                )
                                            })
                                            .small()
                                            .color(theme::TEXT_DIM),
                                        );
                                    });
                                }
                                if count == 0 && dialog.pending.is_none() {
                                    ui.label(if matches!(dialog.action, FileAction::Folder(_)) {
                                        syfrack_i18n::tr!("Aucun sous-dossier · vous pouvez choisir ce dossier.")
                                    } else if matches!(dialog.action, FileAction::Image(_)) {
                                        syfrack_i18n::tr!("Aucune image correspondante dans ce dossier")
                                    } else if matches!(dialog.action, FileAction::Hdri(_)) {
                                        syfrack_i18n::tr!("Aucun dossier ou panorama .hdr / .exr correspondant")
                                    } else {
                                        syfrack_i18n::tr!("Aucun dossier ou fichier .ron correspondant")
                                    });
                                }
                            });
                    });
                });
                if let Some((path, remember)) = destination {
                    dialog.navigate(path, remember);
                }
                ui.separator();
                ui.add_space(6.0);
                if matches!(dialog.action, FileAction::Folder(_)) {
                    ui.horizontal(|ui| {
                        ui.label(syfrack_i18n::tr!("Dossier choisi"));
                        let path = dialog.selected.as_ref().unwrap_or(&dialog.dir);
                        ui.add(egui::Label::new(path.display().to_string()).truncate())
                            .on_hover_text(path.display().to_string());
                    });
                } else {
                    ui.horizontal(|ui| {
                        ui.label(syfrack_i18n::tr!("Fichier"));
                        let field = ui.add(
                            egui::TextEdit::singleline(&mut dialog.filename)
                                .margin(egui::vec2(8.0, 7.0))
                                .hint_text(if matches!(dialog.action, FileAction::Image(_)) {
                                    syfrack_i18n::tr!("Choisir une image PNG, JPEG, BMP ou TIFF")
                                } else if matches!(dialog.action, FileAction::Hdri(_)) {
                                    syfrack_i18n::tr!("Sélectionner un panorama .hdr ou .exr")
                                } else {
                                    syfrack_i18n::tr!("Sélectionner une scène .ron")
                                })
                                .desired_width(ui.available_width()),
                        );
                        go |= field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                        if field.changed() {
                            dialog.selected = None;
                        }
                    });
                }
                ui.label(
                    RichText::new(if matches!(dialog.action, FileAction::Folder(_)) {
                        syfrack_i18n::tr!("Double-cliquez pour entrer dans un dossier, puis confirmez votre choix.")
                    } else if matches!(dialog.action, FileAction::Image(_)) {
                        syfrack_i18n::tr!("Images sRGB · PNG / JPEG / BMP / TIFF · orientation EXIF conservée")
                    } else if matches!(dialog.action, FileAction::Hdri(_)) {
                        syfrack_i18n::tr!("Panoramas linéaires · .hdr / .exr · projection équirectangulaire")
                    } else {
                        syfrack_i18n::tr!("Projets SyFrack · .ron · double-clic pour ouvrir un dossier ou une scène")
                    })
                    .small()
                    .color(theme::TEXT_DIM),
                );
                if let Some(error) = &dialog.error {
                    ui.label(RichText::new(error).color(theme::ERROR));
                }
                ui.add_space(6.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let button = ui.add_enabled(
                        matches!(dialog.action, FileAction::Folder(_))
                            || !dialog.filename.trim().is_empty(),
                        egui::Button::new(RichText::new(verb).color(theme::BG))
                            .fill(theme::ACCENT)
                            .min_size(egui::vec2(112.0, 32.0)),
                    );
                    #[cfg(test)]
                    ui.data_mut(|d| d.insert_temp(Id::new("file-go"), button.rect));
                    go |= button.clicked();
                    if ui
                        .add(egui::Button::new(syfrack_i18n::tr!("Annuler")).min_size(egui::vec2(100.0, 32.0)))
                        .clicked()
                    {
                        close = true;
                    }
                });
            });
        });
    if go {
        match dialog.resolve() {
            Ok(target) => {
                dialog.target = Some(target);
                if (dialog.action == FileAction::Open && state.has_unsaved_workspaces())
                    || (matches!(dialog.action, FileAction::Image(_))
                        && super::mosaic::has_unsaved_work(ui.ctx()))
                {
                    dialog.confirming = true;
                } else if dialog.action == FileAction::SaveAs
                    && dialog.target.as_ref().is_some_and(|p| p.exists())
                {
                    dialog.overwrite = true;
                } else {
                    close = dialog.run(state, ui.ctx());
                }
            }
            Err(e) => dialog.error = Some(e),
        }
    }
    if dialog.confirming {
        match confirm_discard(ui.ctx(), Id::new("file-open-confirm")) {
            Some(true) => {
                dialog.confirming = false;
                close = dialog.run(state, ui.ctx());
            }
            Some(false) => dialog.confirming = false,
            None => {}
        }
    }
    if dialog.overwrite {
        let answer = crate::ui::floating::confirmation(
            ui.ctx(),
            Id::new("file-overwrite-confirm"),
            syfrack_i18n::tr!("Remplacer ce fichier ?"),
            syfrack_i18n::tr!("Remplacer"),
            syfrack_i18n::tr!("Annuler"),
            |ui| {
                let target = dialog.target.as_ref().unwrap();
                let name = target.file_name().unwrap_or_default().to_string_lossy();
                ui.add(egui::Label::new(RichText::new(name).strong()).truncate())
                    .on_hover_text(target.display().to_string());
                ui.label(syfrack_i18n::tr!(
                    "Un fichier porte déjà ce nom. Son contenu sera remplacé par le projet actuel."
                ));
            },
        );
        if let Some(replace) = answer {
            dialog.overwrite = false;
            if replace {
                close = dialog.run(state, ui.ctx());
            }
        }
    }
    if !open || close {
        ui.data_mut(|d| d.remove::<FileDialog>(dialog_id()));
    } else {
        ui.data_mut(|d| d.insert_temp(dialog_id(), dialog));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn folder_selection_returns_only_a_directory_without_touching_the_document() {
        let root =
            std::env::temp_dir().join(format!("syfrack-export-folder-{}", std::process::id()));
        let child = root.join("Images finales");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(root.join("ignore.ron"), "").unwrap();
        let ctx = egui::Context::default();
        let result = Id::new("folder-result-test");
        let mut state = AppState::new(syfrack_core::Scene::example(), None);
        state.set_param("look.exposure_ev", 1.0);
        let before = state.scene.clone();
        let mut dialog = FileDialog::new(&state, FileAction::Folder(result));
        dialog.navigate(root.clone(), false);
        assert_eq!(dialog.resolve().unwrap(), root);
        let entries = scan(&root, dialog.action).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].folder);
        dialog.selected = Some(child.clone());
        dialog.target = Some(dialog.resolve().unwrap());
        assert!(dialog.run(&mut state, &ctx));
        assert_eq!(ctx.data(|d| d.get_temp::<PathBuf>(result)), Some(child));
        assert_eq!(state.scene, before);
        assert!(state.has_unsaved_workspaces());
        assert!(state.path.is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn invalid_address_preserves_directory_selection_and_history() {
        let state = AppState::new(syfrack_core::Scene::example(), None);
        let mut dialog = FileDialog::new(&state, FileAction::SaveAs);
        let dir = dialog.dir.clone();
        dialog.filename = "my-work.ron".into();
        dialog.selected = Some(dir.join("my-work.ron"));
        let before = dialog.selected.clone();
        dialog.navigate(dir.join("nonexistent-directory-for-address-test"), true);
        assert_eq!(dialog.dir, dir);
        assert_eq!(dialog.filename, "my-work.ron");
        assert_eq!(dialog.selected, before);
        assert!(dialog.back.is_empty());
        assert!(dialog.error.is_some());
    }

    #[test]
    fn breadcrumb_targets_keep_the_full_path_including_external_roots() {
        let root = std::env::temp_dir().join("syfrack-breadcrumb-root");
        let dir = root.join("scenes").join("my collection");
        let trail = breadcrumbs(&dir, &root);
        assert_eq!(trail.len(), 3);
        assert_eq!(trail[0], ("SyFrack".into(), root.clone()));
        assert_eq!(trail[2], ("my collection".into(), dir));
        let external = std::env::temp_dir().join("outside-the-project");
        let trail = breadcrumbs(&external, &root);
        assert_eq!(
            trail.first().unwrap().1,
            external.ancestors().last().unwrap()
        );
        assert_eq!(trail.last().unwrap().1, external);
    }

    #[test]
    fn hdri_import_filters_formats_and_edits_the_light_without_replacing_the_scene() {
        let dir = std::env::temp_dir().join(format!("syfrack-hdri-browser-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["studio.EXR", "sun.hdr", "scene.ron", "image.png"] {
            std::fs::write(dir.join(name), "").unwrap();
        }
        let entries = scan(&dir, FileAction::Hdri(None)).unwrap();
        assert_eq!(entries.len(), 2);
        let mut state = AppState::new(syfrack_core::Scene::example(), None);
        let before = state.scene.clone();
        let mut dialog = FileDialog::new(&state, FileAction::Hdri(None));
        dialog.dir = dir.clone();
        dialog.filename = "studio.EXR".into();
        dialog.target = Some(dialog.resolve().unwrap());
        assert!(dialog.run(&mut state, &egui::Context::default()));
        assert_eq!(state.scene.fractal, before.fractal);
        assert_eq!(state.scene.lights.len(), before.lights.len() + 1);
        state.undo();
        assert_eq!(state.scene, before);
        dialog.filename = "image.png".into();
        assert!(dialog.resolve().is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn explorer_orders_folders_and_filters_non_scenes() {
        let dir = std::env::temp_dir().join(format!("syfrack-browser-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Z-folder")).unwrap();
        std::fs::write(dir.join("A.RON"), "").unwrap();
        std::fs::write(dir.join("image.png"), "").unwrap();
        let entries = scan(&dir, FileAction::Open).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries[0].folder);
        assert_eq!(entries[1].name, "A.RON");
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn save_names_gain_extension_and_missing_open_targets_are_refused() {
        let state = AppState::new(syfrack_core::Scene::example(), None);
        let mut dialog = FileDialog::new(&state, FileAction::SaveAs);
        dialog.filename = "test-scene".into();
        assert_eq!(dialog.resolve().unwrap().extension().unwrap(), "ron");
        dialog.filename = "bad.png".into();
        assert!(dialog.resolve().is_err());
        dialog.action = FileAction::Open;
        dialog.filename = "absent-for-test.ron".into();
        assert!(dialog.resolve().is_err());
    }
}
