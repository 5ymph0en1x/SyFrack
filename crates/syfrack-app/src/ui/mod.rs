mod chrome;
pub mod creation;
mod file_browser;
pub mod floating;
pub mod form_panel;
pub mod gallery_panel;
pub mod inspector;
pub mod mosaic;
pub mod panels;
pub mod phenomena;
pub mod render_panel;
mod simple;
pub mod texture_preview;
pub mod theme;
pub mod viewport;
pub mod widgets;
pub mod workshops;

use egui::{Key, Modifiers};

use crate::state::{AppState, Tab};
#[cfg(test)]
use file_browser::{dialog_id, FileDialog};
use file_browser::{file_dialog, open_dialog, FileAction};
use form_panel::FormPanel;
use gallery_panel::GalleryPanel;

/// What the central panel shows, and what it reports back.
pub struct Viewport {
    pub texture: Option<egui::TextureId>,
    /// Size in pixels of the preview texture.
    pub size: (u32, u32),
    /// Small status line, bottom left of the viewport.
    pub caption: String,
    /// Output: size of the central area in physical pixels.
    pub available_px: (u32, u32),
    /// Distance to the surface under the cursor; scales free-camera speed.
    pub depth_hint: f32,
    /// Last picked surface point (world space) and its readback.
    pub pick: Option<(glam::Vec3, crate::preview::Pick)>,
    /// Output size to mark over the image (letterbox guides), while the
    /// « Rendre » size fields are being edited.
    pub frame_guide: Option<(u32, u32)>,
    /// Output: what the viewport wants the app to do.
    pub output: viewport::ViewportOutput,
}

/// The « Modifications non enregistrées » modal shared by every action that
/// replaces the scene. `Some(true)`: open anyway; `Some(false)`: cancelled.
pub fn confirm_discard(ctx: &egui::Context, id: egui::Id) -> Option<bool> {
    confirm_unsaved(
        ctx,
        id,
        syfrack_i18n::tr!("Ouvrir un autre projet ?"),
        syfrack_i18n::tr!("Ouvrir sans enregistrer"),
    )
}

/// The same modal for closing the window.
pub fn confirm_quit(ctx: &egui::Context, id: egui::Id) -> Option<bool> {
    confirm_unsaved(
        ctx,
        id,
        syfrack_i18n::tr!("Quitter sans enregistrer ?"),
        syfrack_i18n::tr!("Quitter sans enregistrer"),
    )
}

fn confirm_unsaved(ctx: &egui::Context, id: egui::Id, question: &str, go: &str) -> Option<bool> {
    floating::confirmation(ctx, id, question, go, syfrack_i18n::tr!("Annuler"), |ui| {
        ui.label(syfrack_i18n::tr!("Un atelier contient des modifications non enregistrées. Elles seront perdues si vous continuez."));
    })
}

/// Edits made while a pointer button is held, or while the viewport is being
/// navigated by wheel or keys, form one undo step: a slider drag, a colour
/// picked in a popup, an orbit, a zoom gesture.
fn coalesce_gestures(ui: &egui::Ui, state: &mut AppState) {
    // A new press always starts a new step, even inside the tail of a wheel
    // or key gesture.
    if ui.input(|i| i.pointer.any_pressed()) {
        state.end_drag();
        ui.data_mut(|d| d.remove::<f64>(viewport::nav_id()));
    }
    // The release frame still belongs to the gesture: an edit it makes (the
    // last delta of a card move) joins the step; the next frame closes it.
    let held = ui.input(|i| i.pointer.any_down() || i.pointer.any_released());
    if held || viewport::navigating(ui) {
        state.begin_drag();
    } else {
        state.end_drag();
    }
}

/// Ctrl+Z undo, Ctrl+Y or Ctrl+Shift+Z redo, Ctrl+S save (save as for a new
/// scene), Ctrl+O open; not
/// while a text field has the keyboard.
fn shortcuts(ui: &egui::Ui, state: &mut AppState) {
    if ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    let shift = Modifiers::COMMAND | Modifiers::SHIFT;
    let (redo_shift, undo, redo, save, open) = ui.input_mut(|i| {
        (
            i.consume_key(shift, Key::Z),
            i.consume_key(Modifiers::COMMAND, Key::Z),
            i.consume_key(Modifiers::COMMAND, Key::Y),
            i.consume_key(Modifiers::COMMAND, Key::S),
            i.consume_key(Modifiers::COMMAND, Key::O),
        )
    });
    if undo {
        if state.tab == Tab::Mosaic {
            mosaic::history(ui.ctx(), false, true);
        } else {
            state.undo();
        }
    }
    if redo || redo_shift {
        if state.tab == Tab::Mosaic {
            mosaic::history(ui.ctx(), true, true);
        } else {
            state.redo();
        }
    }
    if save {
        if state.tab == Tab::Mosaic {
            mosaic::request_export(ui.ctx());
        } else if state.path.is_some() {
            state.save();
        } else {
            open_dialog(ui, state, FileAction::SaveAs);
        }
    }
    if open {
        if state.tab == Tab::Mosaic {
            mosaic::choose_image(ui, state);
        } else {
            open_dialog(ui, state, FileAction::Open);
        }
    }
    if ui.input_mut(|i| i.consume_key(shift, Key::S)) {
        if state.tab == Tab::Mosaic {
            mosaic::request_export(ui.ctx());
        } else {
            open_dialog(ui, state, FileAction::SaveAs);
        }
    }
    if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter)) {
        if state.tab == Tab::Mosaic {
            mosaic::request_generate(ui.ctx());
        } else {
            ui.data_mut(|d| d.insert_temp(render_panel::toggle_id(), true));
        }
    }
    if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab)) {
        state.image_only = !state.image_only;
    }
    if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::H)) {
        state.show_gizmos = !state.show_gizmos;
    }
    if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F1)) {
        ui.data_mut(|d| d.insert_temp(egui::Id::new("studio-help"), true));
    }
}

fn help(ui: &egui::Ui) {
    let id = egui::Id::new("studio-help");
    let mut open = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    if open {
        crate::ui::floating::Window::new(syfrack_i18n::tr!("Prendre en main le Studio")).open(&mut open).resizable(false)
            .default_width(460.0).max_height((ui.ctx().content_rect().height()-100.0).max(280.0)).vscroll(true).show(ui.ctx(), |ui| {
                ui.heading(syfrack_i18n::tr!("Interface"));
                for mode in crate::interface::InterfaceMode::ALL {
                    ui.label(egui::RichText::new(mode.label()).strong());
                    ui.label(mode.description());
                }
                ui.add_space(8.0);
                ui.heading(syfrack_i18n::tr!("De la forme à l’image"));
                for (title, body) in [
                    (syfrack_i18n::tr!("1 · Choisir l'atelier"), syfrack_i18n::tr!("2D dessine une image fractale : motif, couleurs et cadrage. 3D compose un volume avec une matière, des lumières et une caméra. Chaque atelier conserve votre composition quand vous passez à l'autre.")),
                    (syfrack_i18n::tr!("2 · Donner une matière"), syfrack_i18n::tr!("Matière règle la surface. Texture génère un motif indépendant : couleur, rugosité, relief fin, profondeur et éventails.")),
                    (syfrack_i18n::tr!("3 · Éclairer et développer"), syfrack_i18n::tr!("Lumières place les sources. Atmosphère ajoute brouillard, mer et ciel. Image règle l’exposition et les couleurs.")),
                    (syfrack_i18n::tr!("4 · Rendre"), syfrack_i18n::tr!("Rendre / Exporter prépare la définition, la qualité et le débruitage. Chaque rendu conserve le PNG, l’EXR brut et sa scène.")),
                ] {
                    ui.label(egui::RichText::new(title).strong()); ui.label(body); ui.add_space(8.0);
                }
                ui.separator();
                ui.label(syfrack_i18n::tr!("2D : glisser pour déplacer ; molette pour zoomer autour du pointeur. 3D : glisser pour tourner, molette pour zoomer, bouton milieu pour déplacer."));
                ui.label(syfrack_i18n::tr!("Caméra libre : clic droit pour regarder, ZQSD pour se déplacer, E/C pour monter ou descendre."));
                ui.label(syfrack_i18n::tr!("Clic droit dans l’image : mise au point, point d’orbite ou nouvelle lumière."));
                ui.label(syfrack_i18n::tr!("Double-clic sur une valeur : saisie précise. Ctrl+Z / Ctrl+Y : annuler / rétablir. Tab : vue seule. H : poignées."));
            });
    }
    ui.data_mut(|d| d.insert_temp(id, open));
}

fn banner(ui: &mut egui::Ui, state: &mut AppState) {
    let Some(b) = &state.banner else {
        return;
    };
    let color = if b.error {
        theme::ERROR
    } else {
        theme::TEXT_DIM
    };
    let text = b.text.clone();
    egui::Panel::top("banner").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(text).color(color));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("×").clicked() {
                    state.banner = None;
                }
            });
        });
    });
}

/// Document actions, workspace navigation, then image + focused inspector,
/// the node canvas or the render gallery.
pub fn draw(
    ui: &mut egui::Ui,
    state: &mut AppState,
    view: &mut Viewport,
    gallery: &mut GalleryPanel,
    form: &mut FormPanel,
) {
    if floating::is_secondary(ui.ctx()) {
        dialogs(ui, state, gallery, form);
        return;
    }
    mosaic::poll(ui.ctx());
    coalesce_gestures(ui, state);
    if !floating::blocked(ui.ctx()) {
        shortcuts(ui, state);
    }
    if floating::blocked(ui.ctx()) {
        ui.disable();
    }
    chrome::show(ui, state);
    dialogs(ui, state, gallery, form);
    banner(ui, state);
    if state.tab == Tab::Mosaic {
        mosaic::show(ui, state);
        return;
    }
    if state.tab == Tab::Forme {
        form.show(ui, state);
        return;
    }
    if state.tab == Tab::Galerie {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::same(2 * theme::GRID as i8)),
            )
            .show(ui, |ui| gallery.show(ui, state));
        return;
    }
    egui::Panel::bottom("status").show(ui, |ui| {
        ui.horizontal(|ui| {
            form.preview_controls(ui);
            ui.label(
                egui::RichText::new(&view.caption)
                    .small()
                    .monospace()
                    .color(theme::TEXT_DIM),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let activity = ui
                    .data(|d| d.get_temp::<Option<String>>(render_panel::activity_id()))
                    .flatten();
                if let Some(activity) = activity {
                    if ui
                        .small_button(egui::RichText::new(activity).color(theme::ACCENT))
                        .clicked()
                    {
                        ui.data_mut(|d| d.insert_temp(render_panel::toggle_id(), true));
                    }
                } else {
                    ui.label(
                        egui::RichText::new(syfrack_i18n::tr!("Aperçu progressif"))
                            .small()
                            .color(theme::TEXT_DIM),
                    );
                }
            });
        });
    });
    if !state.image_only {
        egui::Panel::right("workspace-inspector")
            .resizable(true)
            .default_size(inspector::WIDTH)
            .size_range(360.0..=(ui.ctx().content_rect().width() * 0.45).clamp(360.0, 480.0))
            .show(ui, |ui| {
                form.library.refresh();
                inspector::show(ui, state, &form.library.entries);
            });
    }
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(theme::BG))
        .show(ui, |ui| {
            let available = ui.available_size();
            let ppp = ui.pixels_per_point();
            view.available_px = (
                (available.x * ppp).round().max(0.0) as u32,
                (available.y * ppp).round().max(0.0) as u32,
            );
            viewport::show(ui, state, view);
        });
}

#[cfg(test)]
mod tests;

pub fn dialogs(
    ui: &mut egui::Ui,
    state: &mut AppState,
    gallery: &mut GalleryPanel,
    form: &mut FormPanel,
) {
    help(ui);
    file_dialog(ui, state);
    gallery.dialogs(ui, state);
    form.dialogs(ui, state);
}
