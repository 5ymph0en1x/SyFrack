//! One focused inspector beside the image, with independent scroll positions.
use egui::{RichText, Ui};

use crate::state::{AppState, Inspector};
use crate::ui::{panels, theme, workshops};

pub const WIDTH: f32 = 420.0;

pub fn show(ui: &mut Ui, state: &mut AppState, _library: &[crate::library::LibraryEntry]) {
    ui.add_space(4.0);
    let pages = workshops::pages(state.scene.dimension);
    if !pages.contains(&state.inspector) {
        state.inspector = Inspector::Scene;
    }
    for row in pages.chunks(3) {
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 16.0) / 3.0;
            for &page in row {
                let selected = state.inspector == page;
                let r = ui
                    .add_sized(
                        [width, 30.0],
                        egui::Button::new(
                            RichText::new(workshops::label(page, state.scene.dimension)).color(
                                if selected {
                                    theme::ACCENT
                                } else {
                                    theme::TEXT_DIM
                                },
                            ),
                        )
                        .selected(selected)
                        .fill(if selected {
                            theme::SELECTED
                        } else {
                            theme::PANEL
                        })
                        .stroke(egui::Stroke::new(
                            1.0,
                            if selected {
                                theme::ACCENT
                            } else {
                                theme::SEPARATOR
                            },
                        )),
                    )
                    .on_hover_text(page.description());
                if r.clicked() {
                    state.inspector = page;
                }
                #[cfg(test)]
                ui.data_mut(|d| {
                    d.insert_temp(egui::Id::new(("inspector-page", page.label())), r.rect)
                });
            }
        });
    }
    ui.add_space(12.0);
    ui.heading(workshops::label(state.inspector, state.scene.dimension));
    ui.add_space(8.0);
    ui.separator();
    ui.add_space(8.0);
    ui.push_id(state.inspector.label(), |ui| {
        panels::column(ui, |ui| workshops::show(ui, state));
    });
}

pub fn group(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    ui.label(RichText::new(title).strong().color(theme::TEXT));
    egui::Frame::NONE
        .fill(theme::BG)
        .corner_radius(6.0)
        .inner_margin(egui::Margin::same(12))
        .show(ui, body);
    ui.add_space(8.0);
}
