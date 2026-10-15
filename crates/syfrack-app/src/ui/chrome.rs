//! A compact command bar: document, workspace and output in one visual line.
use super::{
    file_browser::{open_dialog, FileAction},
    render_panel, theme,
};
use crate::state::{AppState, Tab};
use egui::{Align, Id, Layout, RichText, Stroke, Ui, Vec2};
use syfrack_core::Dimension;

pub(super) fn show(ui: &mut Ui, state: &mut AppState) {
    egui::Panel::top("studio-command-bar")
        .frame(
            egui::Frame::NONE
                .fill(theme::PANEL)
                .inner_margin(egui::Margin::symmetric(12, 8)),
        )
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.spacing_mut().button_padding = Vec2::new(10.0, 6.0);
            let wide = ui.available_width() >= 1160.0;
            ui.horizontal(|ui| {
                document_actions(ui, state);
                ui.separator();
                if wide {
                    navigation(ui, state);
                    ui.separator();
                }
                document_title(ui, state, (ui.available_width() - 300.0).max(60.0));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    actions(ui, state)
                });
            });
            if !wide {
                ui.add_space(2.0);
                ui.horizontal(|ui| navigation(ui, state));
            }
            let line = ui.min_rect();
            ui.painter().hline(
                ui.max_rect().x_range(),
                line.bottom() + 7.5,
                Stroke::new(1.0, theme::SEPARATOR),
            );
        });
}

fn document_actions(ui: &mut Ui, state: &mut AppState) {
    ui.label(
        RichText::new("SYFRACK")
            .size(16.0)
            .strong()
            .color(theme::ACCENT),
    );
    ui.label(RichText::new("STUDIO").size(10.0).color(theme::TEXT_DIM));
    ui.add_space(6.0);
    ui.menu_button(syfrack_i18n::tr!("Fichier"), |ui| {
        if ui
            .button(syfrack_i18n::tr!("Ouvrir…                 Ctrl+O"))
            .clicked()
        {
            open_dialog(ui, state, FileAction::Open);
            ui.close();
        }
        if ui
            .button(syfrack_i18n::tr!("Enregistrer             Ctrl+S"))
            .clicked()
        {
            if state.path.is_some() {
                state.save();
            } else {
                open_dialog(ui, state, FileAction::SaveAs);
            }
            ui.close();
        }
        if ui
            .button(syfrack_i18n::tr!("Enregistrer sous…   Ctrl+Maj+S"))
            .clicked()
        {
            open_dialog(ui, state, FileAction::SaveAs);
            ui.close();
        }
    });
    if history_button(ui, state.can_undo(), false).clicked() {
        state.undo();
    }
    if history_button(ui, state.can_redo(), true).clicked() {
        state.redo();
    }
}

fn history_button(ui: &mut Ui, enabled: bool, redo: bool) -> egui::Response {
    let label = if redo {
        syfrack_i18n::tr!("Rétablir")
    } else {
        syfrack_i18n::tr!("Undo")
    };
    let response = ui.add_enabled(
        enabled,
        egui::Button::new("")
            .min_size(Vec2::splat(28.0))
            .frame(false),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled && ui.is_enabled(), label)
    });
    let color = if response.enabled() {
        ui.style().interact(&response).fg_stroke.color
    } else {
        theme::TEXT_DIM.gamma_multiply(0.4)
    };
    let center = response.rect.center();
    let point = |x: f32, y: f32| center + Vec2::new(if redo { -x } else { x }, y);
    ui.painter().add(egui::Shape::line(
        vec![
            point(6.0, 5.0),
            point(6.0, -1.0),
            point(3.0, -4.0),
            point(-3.0, -4.0),
            point(-7.0, 0.0),
        ],
        Stroke::new(1.5, color),
    ));
    ui.painter().add(egui::Shape::line(
        vec![point(-7.0, -5.0), point(-7.0, 0.0), point(-2.0, 0.0)],
        Stroke::new(1.5, color),
    ));
    #[cfg(test)]
    ui.data_mut(|d| d.insert_temp(Id::new(("history", label)), response.rect));
    response.on_hover_text(if redo {
        syfrack_i18n::tr!("Rétablir · Ctrl+Y / Ctrl+Maj+Z")
    } else {
        syfrack_i18n::tr!("Annuler la dernière modification · Ctrl+Z")
    })
}

fn navigation(ui: &mut Ui, state: &mut AppState) {
    ui.push_id("workspace-mode", |ui| {
        egui::Frame::NONE
            .fill(theme::BG)
            .corner_radius(5.0)
            .inner_margin(2)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 1.0;
                    for (dimension, label) in [(Dimension::TwoD, "2D"), (Dimension::ThreeD, "3D")] {
                        let text = if state.workshop_modified(dimension) {
                            format!("{label} ●")
                        } else {
                            label.into()
                        };
                        let response = ui
                            .add(
                                egui::Button::new(text)
                                    .min_size(Vec2::new(38.0, 28.0))
                                    .selected(state.scene.dimension == dimension)
                                    .frame(state.scene.dimension == dimension),
                            )
                            .on_hover_text(syfrack_i18n::tr_format!(
                                "Atelier {label} · chaque atelier conserve sa composition",
                                label = label
                            ));
                        #[cfg(test)]
                        ui.data_mut(|d| {
                            d.insert_temp(Id::new(("dimension", label)), response.rect)
                        });
                        if response.clicked() {
                            state.switch_dimension(dimension);
                        }
                    }
                });
            });
    });
    ui.add_space(4.0);
    for (tab, label, hint) in [
        (
            Tab::Atelier,
            syfrack_i18n::tr!("Atelier"),
            syfrack_i18n::tr!("Composer la scène et développer l’image"),
        ),
        (
            Tab::Forme,
            syfrack_i18n::tr!("Forme"),
            syfrack_i18n::tr!("Construire par étapes ou relier les nœuds"),
        ),
        (
            Tab::Galerie,
            syfrack_i18n::tr!("Galerie"),
            syfrack_i18n::tr!("Retrouver les rendus et reprendre les calculs"),
        ),
    ] {
        if state.scene.dimension == Dimension::TwoD && tab == Tab::Forme {
            continue;
        }
        let selected = state.tab == tab;
        let response = ui
            .add(
                egui::Button::new(RichText::new(label).color(if selected {
                    theme::ACCENT
                } else {
                    theme::TEXT_DIM
                }))
                .min_size(Vec2::new(66.0, 28.0))
                .selected(selected)
                .frame(selected),
            )
            .on_hover_text(hint);
        if selected {
            ui.painter().hline(
                response.rect.x_range(),
                response.rect.bottom() + 2.0,
                Stroke::new(2.0, theme::ACCENT),
            );
        }
        #[cfg(test)]
        ui.data_mut(|d| d.insert_temp(Id::new(("workspace-tab", label)), response.rect));
        if response.clicked() {
            state.tab = tab;
        }
    }
}

fn document_title(ui: &mut Ui, state: &AppState, width: f32) {
    ui.allocate_ui_with_layout(
        Vec2::new(width, 28.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            let name = state
                .path
                .as_ref()
                .and_then(|p| p.file_stem())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| syfrack_i18n::tr!("Scène sans nom").into());
            let modified = state.is_modified();
            let title = ui
                .scope(|ui| {
                    ui.set_max_width((width - if modified { 66.0 } else { 0.0 }).max(16.0));
                    ui.add(
                        egui::Label::new(RichText::new(&name).color(theme::TEXT_DIM))
                            .truncate()
                            .halign(Align::Min),
                    )
                })
                .inner
                .on_hover_text(
                    state
                        .path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or(name),
                );
            #[cfg(test)]
            ui.data_mut(|d| d.insert_temp(Id::new("document-title"), title.rect));
            let _ = title;
            if modified {
                ui.label(
                    RichText::new(syfrack_i18n::tr!("Modifié"))
                        .size(11.0)
                        .color(theme::ACCENT),
                )
                .on_hover_text(syfrack_i18n::tr!("Modifications non enregistrées"));
            }
        },
    );
}

fn actions(ui: &mut Ui, state: &mut AppState) {
    let response = ui
        .add(
            egui::Button::new(
                RichText::new(syfrack_i18n::tr!("Rendre / Exporter")).color(theme::BG),
            )
            .fill(theme::ACCENT)
            .min_size(Vec2::new(138.0, 30.0)),
        )
        .on_hover_text(syfrack_i18n::tr!("Préparer le rendu final · Ctrl+Entrée"));
    #[cfg(test)]
    ui.data_mut(|d| d.insert_temp(Id::new("render-open-button"), response.rect));
    if response.clicked() {
        ui.data_mut(|d| d.insert_temp(render_panel::toggle_id(), true));
    }
    ui.separator();
    let display = ui.menu_button(syfrack_i18n::tr!("Affichage"), |ui| {
        #[cfg(test)]
        ui.data_mut(|d| d.insert_temp(Id::new("display-menu-open"), true));
        let response = ui.checkbox(
            &mut state.image_only,
            syfrack_i18n::tr!("Vue seule                  Tab"),
        );
        #[cfg(test)]
        ui.data_mut(|d| d.insert_temp(Id::new("display-image-only"), response.rect));
        let _ = response;
        if state.scene.dimension == Dimension::ThreeD {
            ui.checkbox(
                &mut state.show_gizmos,
                syfrack_i18n::tr!("Poignées de lumière       H"),
            );
        }
        ui.separator();
        ui.label(
            RichText::new(syfrack_i18n::tr!("Échelle de l’interface"))
                .small()
                .color(theme::TEXT_DIM),
        );
        let scale = ui.ctx().zoom_factor();
        for (label, zoom) in [
            ("100 %", 1.0),
            ("115 %", 1.15),
            ("130 %", 1.3),
            ("150 %", 1.5),
        ] {
            if super::widgets::menu_choice(ui, (scale - zoom).abs() < 0.01, label).clicked() {
                ui.ctx().set_zoom_factor(zoom);
                ui.close();
            }
        }
        ui.separator();
        ui.label(syfrack_i18n::tr!("Language"));
        for language in syfrack_i18n::Language::ALL {
            let response = super::widgets::menu_choice(
                ui,
                syfrack_i18n::language() == language,
                language.native_name(),
            );
            #[cfg(test)]
            ui.data_mut(|d| d.insert_temp(Id::new(("language", language.code())), response.rect));
            if response.clicked() {
                if let Err(error) = syfrack_i18n::select(language) {
                    state.banner = Some(crate::state::Banner::error(syfrack_i18n::tr_format!(
                        "Language preference could not be saved: {}",
                        error
                    )));
                }
                ui.ctx().request_repaint();
                ui.close();
            }
        }
    });
    #[cfg(test)]
    ui.data_mut(|d| d.insert_temp(Id::new("display-menu"), display.response.rect));
    let _ = display;
    let help = ui
        .add(egui::Button::new(syfrack_i18n::tr!("Aide")).frame(false))
        .on_hover_text(syfrack_i18n::tr!("Prendre en main le Studio · F1"));
    if help.clicked() {
        ui.data_mut(|d| d.insert_temp(Id::new("studio-help"), true));
    }
}
