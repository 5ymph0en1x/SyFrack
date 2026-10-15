use super::*;
use syfrack_core::scene::graph::language;

#[derive(Default)]
pub(super) struct SourceEditor {
    pub(super) text: String,
    pub(super) base: Option<Graph>,
    pub(super) dirty: bool,
    pub(super) error: Option<String>,
}

impl SourceEditor {
    fn refresh(&mut self, g: &Graph) {
        self.text = language::format(g);
        self.base = Some(g.clone());
        self.dirty = false;
        self.error = None;
    }

    pub(super) fn apply(&mut self, state: &mut AppState) -> bool {
        let g = state.scene.fractal.to_graph();
        if self.base.as_ref() != Some(&g) {
            self.error = Some(syfrack_i18n::tr!("La forme a changé pendant l’écriture. Copier le brouillon, puis Actualiser depuis la forme avant de l’appliquer.").into());
            return false;
        }
        match language::compile(&self.text, Some(&g)) {
            Ok(next) => {
                if state.edit_graph(|g| {
                    *g = next;
                    Ok(())
                }) {
                    self.refresh(&state.scene.fractal.to_graph());
                    state.banner = Some(crate::state::Banner::info(
                        syfrack_i18n::tr!("Langage appliqué · une étape dans l’historique"),
                    ));
                    true
                } else {
                    self.error = state.banner.as_ref().map(|b| b.text.clone());
                    false
                }
            }
            Err(e) => {
                self.error = Some(e.to_string());
                false
            }
        }
    }

    pub(super) fn dialog(&mut self, ui: &mut Ui, state: &AppState) {
        let g = state.scene.fractal.to_graph();
        if ui
            .data(|d| d.get_temp::<bool>(Id::new("source-discard")))
            .unwrap_or(false)
        {
            let answer = crate::ui::floating::confirmation(
                ui.ctx(),
                Id::new("source-discard-modal"),
                syfrack_i18n::tr!("Remplacer le brouillon ?"),
                syfrack_i18n::tr!("Remplacer"),
                syfrack_i18n::tr!("Garder le brouillon"),
                |ui| {
                    ui.label(syfrack_i18n::tr!("Les changements non appliqués seront remplacés par le langage de la forme actuelle."));
                },
            );
            if let Some(replace) = answer {
                if replace {
                    self.refresh(&g);
                }
                ui.data_mut(|d| d.insert_temp(Id::new("source-discard"), false));
            }
        }
    }

    pub(super) fn show(&mut self, ui: &mut Ui, state: &mut AppState, g: &Graph) {
        if self.base.is_none() || (!self.dirty && self.base.as_ref() != Some(g)) {
            self.refresh(g);
        }
        let frame = egui::Frame::NONE.inner_margin(egui::Margin::same(16));
        frame.show(ui, |ui| {
            ui.heading(syfrack_i18n::tr!("Forme · un langage pour construire"));
            ui.label(RichText::new(syfrack_i18n::tr!("Nommer une forme, la transformer, puis l’assembler. Les sources sont écrites dans l’ordre ; les paramètres suivent les points-virgules.")).color(theme::TEXT_DIM));
            ui.horizontal(|ui| {
                let apply = ui.add_enabled(self.dirty, egui::Button::new(RichText::new(syfrack_i18n::tr!("Appliquer à la forme")).color(theme::BG)).fill(theme::ACCENT));
                #[cfg(test)] ui.data_mut(|d| d.insert_temp(Id::new("source-apply"), apply.rect));
                if apply.clicked() { self.apply(state); }
                if ui.button(syfrack_i18n::tr!("Copier le texte")).clicked() { ui.ctx().copy_text(self.text.clone()); }
                if ui.button(syfrack_i18n::tr!("Actualiser depuis la forme")).clicked() {
                    if self.dirty { ui.data_mut(|d| d.insert_temp(Id::new("source-discard"), true)); }
                    else { self.refresh(g); }
                }
            });
            egui::CollapsingHeader::new(syfrack_i18n::tr!("Petit guide · vocabulaire et exemple")).show(ui, |ui| {
                ui.label(syfrack_i18n::tr!("départ fournit l’espace ; sphère / boîte / plan créent des surfaces. déplacer / tourner / agrandir changent l’espace avant de créer une surface."));
                ui.label(syfrack_i18n::tr!("unir(a, b) assemble ; creuser(a, b) garde a et retire b ; croiser(a, b) conserve leur intersection. répéter(source; fois=8; étapes=[a, b]) répète une liste d’opérations sans sources propres."));
                ui.label(syfrack_i18n::tr!("Les nombres utilisent un point décimal. taille est une demi-taille de boîte. _ désigne une source encore à relier. actif=non met une étape en pause. # introduit un commentaire."));
                ui.monospace(language::EXAMPLE);
                ui.label(syfrack_i18n::tr_format!("Toutes les opérations : {}", language::operations().iter().map(language::display_verb).collect::<Vec<_>>().join(" · ")));
            });
            if self.dirty { ui.label(RichText::new(syfrack_i18n::tr!("Brouillon · l’aperçu utilise la dernière forme appliquée")).color(theme::ACCENT)); }
            if self.dirty && self.base.as_ref() != Some(g) { ui.label(RichText::new(syfrack_i18n::tr!("La forme a changé. Le brouillon est conservé ; actualiser avant de l’appliquer.")).color(theme::ERROR)); }
            if let Some(error) = &self.error { ui.label(RichText::new(error).color(theme::ERROR)); }
            ui.separator();
            egui::ScrollArea::both().id_salt("form-source").auto_shrink([false, false]).show(ui, |ui| {
                let response = ui.add(egui::TextEdit::multiline(&mut self.text).id(Id::new("form-source-input")).font(egui::TextStyle::Monospace).code_editor().desired_width(ui.available_width()).desired_rows(32));
                if response.changed() { self.dirty = true; self.error = None; }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn applying_a_script_is_atomic_undoable_and_preserves_material() {
        let mut state = AppState::new(syfrack_core::Scene::example(), None);
        let before = state.scene.clone();
        let mut editor = SourceEditor::default();
        editor.refresh(&state.scene.fractal.to_graph());
        editor.text = editor.text.replace("rayon=1", "rayon=2");
        editor.dirty = true;
        assert!(editor.apply(&mut state));
        assert_eq!(state.scene.material, before.material);
        assert_ne!(state.scene.fractal, before.fractal);
        assert!(state.undo());
        assert_eq!(state.scene, before);
        editor.refresh(&state.scene.fractal.to_graph());
        editor.text.push_str("\ninvalide = dragon()\n");
        assert!(!editor.apply(&mut state));
        assert_eq!(state.scene, before);
        assert!(!state.can_undo());
    }
    #[test]
    fn external_edits_do_not_get_overwritten_by_a_stale_draft() {
        let mut state = AppState::new(syfrack_core::Scene::example(), None);
        let mut editor = SourceEditor::default();
        editor.refresh(&state.scene.fractal.to_graph());
        let id = state
            .scene
            .fractal
            .to_graph()
            .nodes
            .iter()
            .find(|n| matches!(n.kind, NodeKind::Sphere { .. }))
            .unwrap()
            .id;
        assert!(state.set_param(&format!("fractal.nodes[{id}].radius"), 2.5));
        let before = state.scene.clone();
        assert!(!editor.apply(&mut state));
        assert_eq!(state.scene, before);
    }
}
