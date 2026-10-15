//! The Atelier's « Forme » summary: which form is rendered, its size, the
//! step safety, and the way to the workshop. The form itself is edited in
//! the « Forme » tab.

use egui::RichText;

use super::{labeled, param};
use crate::library::{self, LibraryEntry};
use crate::state::{AppState, Tab};
use crate::ui::theme;

/// What the summary says the form is: the library entry it matches, or a
/// custom form.
pub fn form_name(entries: &[LibraryEntry], g: &syfrack_core::Graph) -> String {
    library::matching(entries, g).map_or_else(
        || syfrack_i18n::tr!("forme personnalisée").to_string(),
        |e| e.localized_name().to_owned(),
    )
}

pub fn show(ui: &mut egui::Ui, state: &mut AppState, library: &[LibraryEntry]) {
    let g = state.scene.fractal.to_graph();
    labeled(ui, syfrack_i18n::tr!("Nom"), |ui| {
        ui.label(form_name(library, &g));
    });
    labeled(ui, syfrack_i18n::tr!("Taille"), |ui| {
        ui.label(
            RichText::new(syfrack_i18n::tr_format!(
                "{} nœuds · profondeur {}",
                g.nodes.len(),
                g.iterate_depth()
            ))
            .color(theme::TEXT_DIM),
        );
    });
    // Only a form graph has a safety factor of its own.
    param("fractal.safety", syfrack_i18n::tr!("Sûreté du pas"))
        .decimals(2)
        .show(ui, state);
    ui.add_space(theme::GRID);
    let edit = ui.button(syfrack_i18n::tr!("Éditer la forme"));
    #[cfg(test)]
    ui.data_mut(|d| d.insert_temp(egui::Id::new("edit-form-button"), edit.rect));
    if edit.clicked() {
        state.tab = Tab::Forme;
    }
}
