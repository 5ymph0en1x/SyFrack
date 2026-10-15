//! A readable view of the actual graph: assemblies, shapes, then their
//! ordered space modifiers. Shared sources are referenced, never duplicated.
use super::*;
use std::collections::HashSet;
use syfrack_core::scene::graph::language;

pub(super) fn point_chain(g: &Graph, target: NodeId) -> Vec<NodeId> {
    let mut chain = vec![];
    let mut next = g.input_of(target, 0).map(|s| s.0);
    let mut seen = HashSet::new();
    while let Some(id) = next {
        if !seen.insert(id) {
            break;
        }
        chain.push(id);
        next = g.input_of(id, 0).map(|s| s.0);
    }
    chain.reverse();
    chain
}

/// Only a plain, enabled union is associative. Cuts and smooth joins keep
/// their exact operand order and hierarchy.
fn union_parts(
    g: &Graph,
    id: NodeId,
    parts: &mut Vec<NodeId>,
    intermediate: &mut Vec<NodeId>,
    seen: &mut HashSet<NodeId>,
) {
    if !seen.insert(id) {
        return;
    }
    if g.node(id)
        .is_some_and(|n| n.enabled && n.kind == NodeKind::Union)
    {
        intermediate.push(id);
        for p in 0..2 {
            if let Some(src) = g.input_of(id, p) {
                union_parts(g, src.0, parts, intermediate, seen);
            }
        }
    } else {
        parts.push(id);
    }
}

fn select(ui: &mut Ui, state: &mut AppState, n: &Node) {
    let label = if n.name.is_empty() {
        n.kind.label()
    } else {
        &n.name
    };
    let on = state.form.selected == [n.id];
    let response = ui.selectable_label(
        on,
        RichText::new(label).color(if on { theme::ACCENT } else { theme::TEXT }),
    );
    #[cfg(test)]
    ui.data_mut(|d| d.insert_temp(Id::new(("construction-node", n.id)), response.rect));
    if response.clicked() {
        state.form.selected = vec![n.id];
    }
}

fn step(ui: &mut Ui, state: &mut AppState, g: &Graph, id: NodeId, shown: &mut HashSet<NodeId>) {
    let Some(n) = g.node(id) else { return };
    shown.insert(id);
    ui.horizontal_wrapped(|ui| {
        select(ui, state, n);
        ui.label(
            RichText::new(language::display_verb(&n.kind).replace('_', " "))
                .small()
                .color(theme::TEXT_DIM),
        );
        if !n.enabled {
            ui.label(
                RichText::new(syfrack_i18n::tr!("en pause"))
                    .small()
                    .color(theme::TEXT_DIM),
            );
        }
    });
    if let NodeKind::Iterate {
        body, iterations, ..
    } = &n.kind
    {
        egui::CollapsingHeader::new(syfrack_i18n::tr_format!(
            "{iterations} passages · {} étapes",
            body.len(),
            iterations = iterations
        ))
        .id_salt(("construction-loop", id))
        .show(ui, |ui| {
            for b in body {
                step(ui, state, g, *b, shown);
            }
        });
    }
}

fn result(
    ui: &mut Ui,
    state: &mut AppState,
    g: &Graph,
    id: NodeId,
    depth: usize,
    shown: &mut HashSet<NodeId>,
) {
    let Some(n) = g.node(id) else { return };
    if shown.contains(&id) {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(syfrack_i18n::tr!("Réutiliser")).color(theme::TEXT_DIM));
            select(ui, state, n);
        });
        return;
    }
    shown.insert(id);
    if matches!(
        n.kind,
        NodeKind::Bounded { .. } | NodeKind::MaterialTag { .. } | NodeKind::Instances { .. }
    ) {
        egui::CollapsingHeader::new(RichText::new(&n.name).strong())
            .id_salt(("construction-detail", id))
            .default_open(depth < 2)
            .show(ui, |ui| {
                select(ui, state, n);
                let port = if matches!(n.kind, NodeKind::Bounded { .. }) {
                    1
                } else {
                    0
                };
                if let Some((source, _)) = g.input_of(id, port) {
                    result(ui, state, g, source, depth + 1, shown);
                }
            });
        return;
    }
    if n.kind.is_combine() {
        let mut parts = vec![];
        let mut intermediate = vec![];
        if n.enabled && n.kind == NodeKind::Union {
            union_parts(g, id, &mut parts, &mut intermediate, &mut HashSet::new());
        } else {
            parts.extend((0..2).filter_map(|p| g.input_of(id, p).map(|s| s.0)));
        }
        let title = format!(
            "{} · {}",
            n.name,
            language::display_verb(&n.kind).replace('_', " ")
        );
        let indent = ui.spacing().indent;
        if depth >= 3 {
            ui.spacing_mut().indent = 0.0;
        }
        egui::CollapsingHeader::new(RichText::new(title).strong())
            .id_salt(("construction-assembly", id))
            .default_open(depth < 2)
            .show(ui, |ui| {
                select(ui, state, n);
                ui.label(
                    RichText::new(match n.kind {
                        NodeKind::Subtraction => {
                            syfrack_i18n::tr!("Garder la première forme ; retirer la seconde.")
                        }
                        NodeKind::Intersection | NodeKind::SmoothIntersection { .. } => {
                            syfrack_i18n::tr!("Conserver la partie commune aux formes.")
                        }
                        NodeKind::SmoothUnion { .. } => {
                            syfrack_i18n::tr!("Assembler les formes avec une transition douce.")
                        }
                        _ => syfrack_i18n::tr!("Assembler les éléments ci-dessous."),
                    })
                    .small()
                    .color(theme::TEXT_DIM),
                );
                if intermediate.len() > 1 {
                    egui::CollapsingHeader::new(syfrack_i18n::tr_format!(
                        "{} assemblages intermédiaires",
                        intermediate.len() - 1
                    ))
                    .id_salt(("union-details", id))
                    .show(ui, |ui| {
                        for id in &intermediate[1..] {
                            step(ui, state, g, *id, shown);
                        }
                    });
                }
                for (i, part) in parts.into_iter().enumerate() {
                    if n.kind == NodeKind::Subtraction {
                        ui.label(
                            RichText::new(if i == 0 {
                                syfrack_i18n::tr!("GARDER")
                            } else {
                                syfrack_i18n::tr!("RETIRER")
                            })
                            .small()
                            .color(theme::ACCENT),
                        );
                    }
                    result(ui, state, g, part, depth + 1, shown);
                }
                if g.input_of(id, 0).is_none() || g.input_of(id, 1).is_none() {
                    ui.label(
                        RichText::new(syfrack_i18n::tr!(
                            "Une source reste à relier dans le graphe ou le langage"
                        ))
                        .color(theme::ERROR),
                    );
                }
            });
        ui.spacing_mut().indent = indent;
    } else {
        egui::Frame::new()
            .fill(theme::PANEL)
            .stroke(Stroke::new(1.0, theme::SEPARATOR))
            .corner_radius(6)
            .inner_margin(12)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    select(ui, state, n);
                    ui.label(
                        RichText::new(language::display_verb(&n.kind).replace('_', " "))
                            .small()
                            .color(theme::ACCENT),
                    );
                });
                let values = language::parameters(n)
                    .into_iter()
                    .filter(|(k, _)| k != "actif")
                    .map(|(k, v)| format!("{} : {v}", k.replace('_', " ")))
                    .collect::<Vec<_>>()
                    .join(" · ");
                ui.label(RichText::new(values).small().color(theme::TEXT_DIM));
                let chain = point_chain(g, id);
                egui::CollapsingHeader::new(syfrack_i18n::tr_format!(
                    "Construction · {} étapes avant la surface",
                    chain.len()
                ))
                .id_salt(("construction-chain", id))
                .show(ui, |ui| {
                    for (index, id) in chain.into_iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("{}", index + 1)).color(theme::TEXT_DIM),
                            );
                            step(ui, state, g, id, shown);
                        });
                    }
                });
            });
        ui.add_space(8.0);
    }
}

impl FormPanel {
    pub(super) fn construction(&mut self, ui: &mut Ui, state: &mut AppState, g: &Graph) {
        egui::Frame::NONE.inner_margin(16).show(ui, |ui| {
            ui.heading(syfrack_i18n::tr!("Lire la construction"));
            ui.label(RichText::new(syfrack_i18n::tr!("Les formes s’assemblent ; chaque forme possède une suite d’étapes. Cliquer sur un nom pour le régler dans l’inspecteur.")).color(theme::TEXT_DIM));
            ui.add_space(8.0);
            ui.add(egui::TextEdit::singleline(&mut self.query).hint_text(syfrack_i18n::tr!("Rechercher un élément ou une opération…")).desired_width(ui.available_width()));
            ui.separator();
            egui::ScrollArea::vertical().id_salt("construction-document").auto_shrink([false, false]).show(ui, |ui| {
                if !self.query.trim().is_empty() {
                    let query = self.query.to_lowercase();
                    let mut count = 0;
                    for n in &g.nodes {
                        if format!("{} {} {}", n.name, n.kind.label(), language::display_verb(&n.kind)).to_lowercase().contains(&query) {
                            step(ui, state, g, n.id, &mut HashSet::new()); count += 1;
                        }
                    }
                    if count == 0 { ui.label(syfrack_i18n::tr!("Aucun élément correspondant")); }
                } else {
                    let mut shown = HashSet::new();
                    if let Some(out) = g.nodes.iter().find(|n| n.kind == NodeKind::Output) {
                        ui.horizontal(|ui| { ui.label(RichText::new(syfrack_i18n::tr!("RÉSULTAT")).small().color(theme::TEXT_DIM)); select(ui, state, out); });
                        shown.insert(out.id);
                        if let Some(src) = g.input_of(out.id, 0) { result(ui, state, g, src.0, 0, &mut shown); }
                        else { ui.label(RichText::new(syfrack_i18n::tr!("Choisir la forme à montrer dans le langage ou le graphe")).color(theme::ERROR)); }
                    }
                    egui::CollapsingHeader::new(syfrack_i18n::tr!("Toutes les étapes · y compris les éléments non reliés")).id_salt("construction-all").show(ui, |ui| {
                        for n in &g.nodes { step(ui, state, g, n.id, &mut shown); }
                    });
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_shape_lists_its_modifiers_in_source_order() {
        let g = language::compile(language::EXAMPLE, None).unwrap();
        let sphere = g
            .nodes
            .iter()
            .find(|n| matches!(n.kind, NodeKind::Sphere { .. }))
            .unwrap();
        let chain = point_chain(&g, sphere.id);
        assert_eq!(chain.len(), 2);
        assert!(matches!(g.node(chain[0]).unwrap().kind, NodeKind::Input));
        assert!(matches!(
            g.node(chain[1]).unwrap().kind,
            NodeKind::Translate { .. }
        ));
    }
    #[test]
    fn subtraction_is_never_flattened_or_reordered() {
        let g = language::compile(
            &language::EXAMPLE.replace("union(base, ball)", "subtract(base, ball)"),
            None,
        )
        .unwrap();
        let cut = g
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Subtraction)
            .unwrap();
        let mut parts = vec![];
        let mut internals = vec![];
        union_parts(&g, cut.id, &mut parts, &mut internals, &mut HashSet::new());
        assert_eq!(parts, [cut.id]);
        assert!(internals.is_empty());
    }
}
