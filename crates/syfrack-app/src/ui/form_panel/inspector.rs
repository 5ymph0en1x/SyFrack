//! The inspector column.

use super::*;

impl FormPanel {
    /// The inspector column: the selected node, the selection, or the graph
    /// summary. Returns true when it changed the scene.
    pub(super) fn inspector(&mut self, ui: &mut Ui, state: &mut AppState, g: &Graph) -> bool {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.heading(if self.mode == FormMode::Expert {
            syfrack_i18n::tr!("Inspecteur")
        } else {
            syfrack_i18n::tr!("Élément sélectionné")
        });
        ui.add_space(theme::GRID);
        let undo_before = state.can_undo();
        let scene_before = state.scene.fractal.clone();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::NONE
                    .inner_margin(egui::Margin {
                        right: (1.5 * theme::GRID) as i8,
                        ..Default::default()
                    })
                    .show(ui, |ui| match state.form.selected.as_slice() {
                        [] => self.summary(ui, state, g),
                        [id] => match g.node(*id) {
                            Some(n) => self.node_inspector(ui, state, n),
                            None => self.summary(ui, state, g),
                        },
                        many => selection_inspector(ui, state, many.len()),
                    });
            });
        state.can_undo() != undo_before || state.scene.fractal != scene_before
    }

    pub(super) fn node_inspector(&mut self, ui: &mut Ui, state: &mut AppState, n: &Node) {
        // Name: typed in a field, written when the field lets go.
        let field_id = Id::new(("form-name", n.id));
        let focused = ui.memory(|m| m.has_focus(field_id));
        if !focused || self.name_buf.as_ref().is_none_or(|(id, _)| *id != n.id) {
            self.name_buf = Some((n.id, n.name.clone()));
        }
        let mut commit = None;
        panels::labeled(ui, syfrack_i18n::tr!("Nom"), |ui| {
            let buf = &mut self.name_buf.as_mut().expect("set above").1;
            let r = ui.add(
                egui::TextEdit::singleline(buf)
                    .id(field_id)
                    .desired_width(ui.available_width()),
            );
            if std::mem::take(&mut self.focus_name) {
                r.request_focus();
            }
            let cancelled = ui.input(|i| i.key_pressed(Key::Escape));
            if r.lost_focus() && !cancelled && !buf.trim().is_empty() && buf.trim() != n.name {
                commit = Some(buf.trim().to_string());
            }
        });
        if let Some(name) = commit {
            let id = n.id;
            state.edit_graph(|g| {
                if let Some(n) = g.node_mut(id) {
                    n.name = name;
                }
                Ok(())
            });
        }
        panels::labeled(ui, syfrack_i18n::tr!("Nœud"), |ui| {
            ui.label(RichText::new(n.kind.label()).color(theme::TEXT_DIM));
        });
        let io = matches!(n.kind, NodeKind::Input | NodeKind::Output);
        if !io {
            if let Some(on) = panels::toggle(ui, syfrack_i18n::tr!("Actif"), n.enabled) {
                let id = n.id;
                state.edit_graph(|g| {
                    graph_edit::set_enabled(g, &[id], on);
                    Ok(())
                });
            }
        }
        if let Some((_, msg)) = state.form.errors.iter().find(|(e, _)| *e == n.id) {
            ui.add_space(theme::GRID / 2.0);
            ui.label(RichText::new(msg).color(theme::ERROR));
        }
        for note in port_notes(&state.scene.fractal.to_graph(), n.id) {
            ui.add_space(theme::GRID / 2.0);
            ui.label(RichText::new(note).color(theme::ERROR));
        }
        panels::rule(ui);

        if self.mode != FormMode::Expert {
            self.relationships(ui, state, n);
            self.modifier_menu(ui, state, n);
            panels::rule(ui);
        }

        // Choices that are not numbers: the trap measure, the abs axes.
        match &n.kind {
            NodeKind::MaterialTag { index } => {
                let mut selected = *index;
                let finishes = &state.scene.material.finishes;
                let name = if *index == 0 {
                    syfrack_i18n::tr!("Surface globale")
                } else {
                    finishes
                        .get(*index as usize - 1)
                        .map(|f| f.name.as_str())
                        .unwrap_or(syfrack_i18n::tr!("Matériau absent"))
                };
                panels::labeled(ui, syfrack_i18n::tr!("Matériau"), |ui| {
                    egui::ComboBox::from_id_salt(("branch_material", n.id))
                        .selected_text(name)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut selected,
                                0,
                                syfrack_i18n::tr!("Surface globale"),
                            );
                            for (i, f) in finishes.iter().enumerate() {
                                ui.selectable_value(&mut selected, i as u32 + 1, &f.name);
                            }
                        });
                });
                if selected != *index {
                    let id = n.id;
                    state.edit_graph(|g| {
                        if let Some(NodeKind::MaterialTag { index }) =
                            g.node_mut(id).map(|n| &mut n.kind)
                        {
                            *index = selected;
                        }
                        Ok(())
                    });
                }
                ui.small(syfrack_i18n::tr!(
                    "Les matériaux se règlent dans le panneau Surface."
                ));
            }
            NodeKind::RepeatFinite { .. } => {
                ui.small(syfrack_i18n::tr!("Copies centrées, limitées sur chaque axe. Conserver la forme dans une demi-période pour éviter le chevauchement des cellules."));
            }
            NodeKind::Bounded { .. } => {
                ui.small(syfrack_i18n::tr!("Entrée 1 : espace du volume. Entrée 2 : forme détaillée. La forme est découpée aux limites du volume."));
            }
            NodeKind::Instances { placements, .. } => {
                ui.small(syfrack_i18n::tr!("Le modèle est défini autour de l’origine et découpé au volume indiqué. Chaque copie possède sa propre implantation."));
                ui.horizontal(|ui| {
                    ui.label(format!("{} copies", placements.len()));
                    if ui
                        .add_enabled(
                            placements.len() < syfrack_core::graph::MAX_PLACEMENTS,
                            egui::Button::new(syfrack_i18n::tr!("Ajouter")),
                        )
                        .clicked()
                    {
                        let id = n.id;
                        let mut copy = placements.last().cloned().unwrap_or_default();
                        copy.position.x += 2.0;
                        state.edit_graph(|g| {
                            if let Some(NodeKind::Instances { placements, .. }) =
                                g.node_mut(id).map(|n| &mut n.kind)
                            {
                                placements.push(copy);
                            }
                            Ok(())
                        });
                    }
                });
                for (i, _) in placements.iter().enumerate() {
                    egui::CollapsingHeader::new(syfrack_i18n::tr_format!("Copie {:02}", i + 1))
                        .id_salt((n.id, i))
                        .show(ui, |ui| {
                            for (field, label) in [
                                ("position", "Position"),
                                ("angles", "Orientation"),
                                ("scale", "Proportions"),
                            ] {
                                ui.label(RichText::new(label).color(theme::TEXT_DIM));
                                for axis in ["x", "y", "z"] {
                                    let path = format!(
                                        "fractal.nodes[{}].placements[{i}].{field}.{axis}",
                                        n.id
                                    );
                                    panels::param(&path, axis).show(ui, state);
                                }
                            }
                            if ui
                                .add_enabled(
                                    placements.len() > 1,
                                    egui::Button::new(syfrack_i18n::tr!("Supprimer cette copie")),
                                )
                                .clicked()
                            {
                                let id = n.id;
                                state.edit_graph(|g| {
                                    if let Some(NodeKind::Instances { placements, .. }) =
                                        g.node_mut(id).map(|n| &mut n.kind)
                                    {
                                        placements.remove(i);
                                    }
                                    Ok(())
                                });
                            }
                        });
                }
            }
            NodeKind::Trap { measure, mode, .. } => {
                let current = MEASURES.iter().position(|(m, _)| m == measure).unwrap_or(0);
                let names: Vec<&str> = MEASURES
                    .iter()
                    .map(|(_, s)| syfrack_i18n::text(s))
                    .collect();
                if let Some(i) = panels::choice(ui, syfrack_i18n::tr!("⟳ Mesure"), &names, current)
                {
                    let (id, m) = (n.id, MEASURES[i].0);
                    state.edit_graph(|g| {
                        if let Some(NodeKind::Trap { measure, .. }) =
                            g.node_mut(id).map(|n| &mut n.kind)
                        {
                            *measure = m;
                        }
                        Ok(())
                    });
                }
                let modes = [
                    syfrack_core::TrapMode::Minimum,
                    syfrack_core::TrapMode::Maximum,
                    syfrack_core::TrapMode::Last,
                ];
                let current = modes.iter().position(|m| m == mode).unwrap_or(0);
                if let Some(i) = panels::choice(
                    ui,
                    "Accumulation",
                    &["Minimum", "Maximum", syfrack_i18n::tr!("Dernière valeur")],
                    current,
                ) {
                    let id = n.id;
                    state.edit_graph(|g| {
                        if let Some(NodeKind::Trap { mode, .. }) =
                            g.node_mut(id).map(|n| &mut n.kind)
                        {
                            *mode = modes[i];
                        }
                        Ok(())
                    });
                }
            }
            NodeKind::Abs { x, y, z } | NodeKind::BoxFoldAxes { x, y, z, .. } => {
                let mut axes = [*x, *y, *z];
                let mut changed = false;
                panels::labeled(ui, "⟳ Axes", |ui| {
                    for (v, name) in axes.iter_mut().zip(["x", "y", "z"]) {
                        changed |= ui.checkbox(v, name).changed();
                    }
                });
                if changed {
                    let id = n.id;
                    state.edit_graph(|g| {
                        if let Some(
                            NodeKind::Abs { x, y, z } | NodeKind::BoxFoldAxes { x, y, z, .. },
                        ) = g.node_mut(id).map(|n| &mut n.kind)
                        {
                            [*x, *y, *z] = axes;
                        }
                        Ok(())
                    });
                }
            }
            _ => {}
        }
        // Every parameter, in visiting order.
        for (field, bounds) in node_params(n) {
            if matches!(n.kind, NodeKind::MaterialTag { .. }) || field.starts_with("placements[") {
                continue;
            }
            let path = format!("fractal.nodes[{}].{field}", n.id);
            let (name, axis) = field.split_once('.').unwrap_or((&field, ""));
            let label = if axis.is_empty() {
                field_label(name).to_string()
            } else {
                format!("{} {axis}", field_label(name))
            };
            let mut p = panels::param(&path, &label)
                .default_value(graph_edit::default_param(&n.kind, &field));
            if is_integer(name) {
                p = p.topology();
            }
            if is_log(bounds) {
                p = p.log();
            }
            p.show(ui, state);
        }
    }

    /// No selection: what the whole graph costs.
    pub(super) fn summary(&mut self, ui: &mut Ui, state: &AppState, g: &Graph) {
        if self.mode != FormMode::Expert {
            ui.label(syfrack_i18n::tr!(
                "Cliquer sur le nom d’une forme ou d’une étape pour la régler."
            ));
            ui.label(
                RichText::new(syfrack_i18n::tr_format!(
                    "{} éléments · {} liens",
                    g.nodes.len(),
                    g.links.len()
                ))
                .color(theme::TEXT_DIM),
            );
            egui::CollapsingHeader::new(syfrack_i18n::tr!("Détails du moteur"))
                .show(ui, |ui| self.technical_summary(ui, state, g));
        } else {
            self.technical_summary(ui, state, g);
        }
    }

    fn relationships(&self, ui: &mut Ui, state: &mut AppState, n: &Node) {
        let g = state.scene.fractal.to_graph();
        let mut links: Vec<(&str, NodeId)> = (0..n.kind.inputs().len())
            .filter_map(|p| {
                g.input_of(n.id, p as u32).map(|src| {
                    (
                        if n.kind == NodeKind::Subtraction {
                            if p == 0 {
                                syfrack_i18n::tr!("Garder")
                            } else {
                                syfrack_i18n::tr!("Retirer")
                            }
                        } else {
                            syfrack_i18n::tr!("À partir de")
                        },
                        src.0,
                    )
                })
            })
            .collect();
        if let Some(owner) = graph_edit::frame_of(&g, n.id) {
            links.push((syfrack_i18n::tr!("Dans la boucle"), owner));
        }
        links.extend(
            g.links
                .iter()
                .filter(|l| l.from.0 == n.id)
                .map(|l| (syfrack_i18n::tr!("Utilisé par"), l.to.0)),
        );
        for (role, id) in links {
            if let Some(other) = g.node(id) {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(role).small().color(theme::TEXT_DIM));
                    if ui.link(&other.name).clicked() {
                        state.form.selected = vec![id];
                    }
                });
            }
        }
    }

    fn modifier_menu(&self, ui: &mut Ui, state: &mut AppState, n: &Node) {
        if !n.kind.is_fence() && !n.kind.is_point_to_point() && n.kind != NodeKind::Input {
            return;
        }
        ui.add_space(8.0);
        ui.menu_button(syfrack_i18n::tr!("Ajouter une étape…"), |ui| {
            for kind in ALL_KINDS.iter().filter(|k| k.is_point_to_point()) {
                let label =
                    syfrack_core::scene::graph::language::display_verb(kind).replace('_', " ");
                if ui.button(label).clicked() {
                    let mut added = None;
                    if state.edit_graph(|g| {
                        added = Some(insert_modifier(g, n.id, kind.clone())?);
                        Ok(())
                    }) {
                        state.form.selected = added.into_iter().collect();
                    }
                    ui.close();
                }
            }
        });
        ui.label(
            RichText::new(if n.kind.is_fence() {
                syfrack_i18n::tr!("Avant cette surface.")
            } else {
                syfrack_i18n::tr!("Après cette étape, pour ses utilisations.")
            })
            .small()
            .color(theme::TEXT_DIM),
        );
        if n.kind.is_point_to_point()
            && ui
                .small_button(syfrack_i18n::tr!("Retirer cette étape"))
                .clicked()
        {
            state.edit_graph(|g| {
                graph_edit::dissolve_nodes(g, &[n.id]);
                Ok(())
            });
            state.form.selected.clear();
        }
    }

    fn technical_summary(&mut self, ui: &mut Ui, state: &AppState, g: &Graph) {
        let now = ui.input(|i| i.time);
        // Off the UI thread: « … » until the worker answers.
        let radius = self.radius.poll(g, now);
        if radius.is_none() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        let radius = radius.map_or_else(|| "…".to_string(), |r| widgets::format_value(r, 2));
        let active = g.active_nodes().len();
        let lines = [
            (
                syfrack_i18n::tr!("Nœuds"),
                syfrack_i18n::tr_format!("{} · {active} actifs", g.nodes.len(), active = active),
            ),
            (syfrack_i18n::tr!("Fils"), g.links.len().to_string()),
            (
                syfrack_i18n::tr!("Profondeur"),
                g.iterate_depth().to_string(),
            ),
            (
                syfrack_i18n::tr!("Itérations"),
                format!("× {}", g.iteration_product()),
            ),
            (
                syfrack_i18n::tr!("Scalaires"),
                format!(
                    "{} / {}",
                    g.scalar_count(),
                    syfrack_core::scene::graph::MAX_SCALARS
                ),
            ),
            (
                syfrack_i18n::tr!("Borne L"),
                widgets::format_value(g.lipschitz(g.bounding_radius_hint), 2),
            ),
            (
                syfrack_i18n::tr!("Pas"),
                widgets::format_value(g.step_scale(), 2),
            ),
            (syfrack_i18n::tr!("Rayon estimé"), radius),
            (
                "Compilation",
                self.compile_ms
                    .map_or_else(|| "—".to_string(), |ms| format!("{ms:.0} ms")),
            ),
        ];
        for (k, v) in lines {
            panels::labeled(ui, k, |ui| {
                ui.label(RichText::new(v).color(theme::TEXT_DIM));
            });
        }
        for (_, msg) in &state.form.errors {
            ui.add_space(theme::GRID / 2.0);
            ui.label(RichText::new(msg).color(theme::ERROR));
        }
    }
}

/// Several cards selected: their count and the actions that apply to all.
fn insert_modifier(
    g: &mut Graph,
    target: NodeId,
    kind: NodeKind,
) -> Result<NodeId, syfrack_core::SceneError> {
    let n = g
        .node(target)
        .ok_or_else(|| {
            syfrack_core::SceneError::Graph(syfrack_i18n::tr!("Élément inconnu").into())
        })?
        .clone();
    let id = graph_edit::add_node(g, kind, [n.pos[0] + 32.0, n.pos[1] + 120.0]);
    if let Some(owner) = graph_edit::frame_of(g, target) {
        let index = g
            .body_ids_of(owner)
            .iter()
            .position(|b| *b == target)
            .unwrap_or(0)
            + 1;
        graph_edit::move_into_frame(g, id, owner, index)?;
    } else if n.kind.is_fence() {
        let source = g.input_of(target, 0).ok_or_else(|| {
            syfrack_core::SceneError::Graph(
                syfrack_i18n::tr!("Relier d’abord la source de cette surface").into(),
            )
        })?;
        graph_edit::connect(g, source, (id, 0))?;
        graph_edit::connect(g, (id, 0), (target, 0))?;
    } else {
        let consumers: Vec<_> = g
            .links
            .iter()
            .filter(|l| l.from.0 == target)
            .map(|l| l.to)
            .collect();
        graph_edit::connect(g, (target, 0), (id, 0))?;
        for to in consumers {
            graph_edit::connect(g, (id, 0), to)?;
        }
    }
    Ok(id)
}

pub(super) fn selection_inspector(ui: &mut Ui, state: &mut AppState, count: usize) {
    ui.label(syfrack_i18n::tr_format!("{count} cartes", count = count));
    ui.add_space(theme::GRID);
    let sel = state.form.selected.clone();
    ui.horizontal(|ui| {
        if ui.button(syfrack_i18n::tr!("Désactiver")).clicked() {
            state.edit_graph(|g| {
                graph_edit::set_enabled(g, &sel, false);
                Ok(())
            });
        }
        if ui.button(syfrack_i18n::tr!("Activer")).clicked() {
            state.edit_graph(|g| {
                graph_edit::set_enabled(g, &sel, true);
                Ok(())
            });
        }
        if ui.button(syfrack_i18n::tr!("Supprimer")).clicked() {
            delete_selection(state);
        }
    });
}

/// The trap measures, as the inspector names them.
pub(super) const MEASURES: [(Measure, &str); 9] = [
    (Measure::Norm, "Norme"),
    (Measure::AxisX, "X"),
    (Measure::AxisY, "Y"),
    (Measure::AxisZ, "Z"),
    (Measure::PlaneY, "Plan Y"),
    (Measure::AngleXY, "Angle XY"),
    (Measure::AngleXZ, "Angle XZ"),
    (Measure::AngleYZ, "Angle YZ"),
    (Measure::LogNorm, "Norme logarithmique"),
];

/// What the inspector says about the unwired inputs of node `id` (active
/// nodes only: a dangling card is not an error).
pub(super) fn port_notes(g: &Graph, id: NodeId) -> Vec<String> {
    let unwired: Vec<u32> = g
        .unwired_inputs()
        .into_iter()
        .filter(|(n, _)| *n == id)
        .map(|(_, p)| p)
        .collect();
    let many = g.node(id).is_some_and(|n| n.kind.inputs().len() > 1);
    unwired
        .into_iter()
        .map(|p| {
            if many {
                syfrack_i18n::tr_format!("port {} non relié", p + 1)
            } else {
                syfrack_i18n::tr!("port non relié").to_string()
            }
        })
        .collect()
}

#[cfg(test)]
mod construction_tests {
    use super::*;
    #[test]
    fn modifiers_can_be_inserted_before_a_surface_and_inside_a_loop() {
        let mut g = Graph::new_minimal();
        let added = insert_modifier(&mut g, 1, NodeKind::Twist { rate: 0.2 }).unwrap();
        g.validate().unwrap();
        assert_eq!(g.input_of(1, 0), Some((added, 0)));
        assert_eq!(g.input_of(added, 0), Some((0, 0)));
        graph_edit::dissolve_nodes(&mut g, &[added]);
        assert_eq!(g.nodes, Graph::new_minimal().nodes);
        assert_eq!(g.input_of(1, 0), Some((0, 0)));
        assert_eq!(g.input_of(2, 0), Some((1, 0)));
        let mut g = Graph::bundled("mandelbox").unwrap();
        let (owner, target) = g
            .nodes
            .iter()
            .find_map(|n| match &n.kind {
                NodeKind::Iterate { body, .. } => Some((n.id, body[0])),
                _ => None,
            })
            .unwrap();
        let added = insert_modifier(&mut g, target, NodeKind::Twist { rate: 0.2 }).unwrap();
        g.validate().unwrap();
        assert_eq!(g.body_ids_of(owner)[1], added);
        assert!(g.links.iter().all(|l| l.from.0 != added && l.to.0 != added));
    }
}
