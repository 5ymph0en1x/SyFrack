//! Focused artistic controls over the same authoritative creation graph.
use super::{panels, theme, widgets, workshops};
use crate::{
    interface::InterfaceMode,
    state::{AppState, Inspector, Tab},
};
use egui::{Id, RichText, Ui};
use syfrack_core::{
    scene::phenomena::FieldOp, CreationGraph, CreationKind, Dimension, PlanarFractal, Ramp,
};

pub(super) fn show(ui: &mut Ui, state: &mut AppState) {
    let dimension = state.scene.dimension;
    let pages = workshops::pages(dimension);
    if !pages.contains(&state.inspector) {
        state.inspector = Inspector::Scene;
    }
    ui.add_space(8.0);
    ui.label(
        RichText::new(syfrack_i18n::tr!("À vous de créer"))
            .strong()
            .color(theme::ACCENT),
    );
    ui.label(
        RichText::new(if dimension == Dimension::TwoD {
            syfrack_i18n::tr!("Un motif, des couleurs, un cadre. Ajustez l’image à votre rythme.")
        } else {
            syfrack_i18n::tr!(
                "Une forme, une matière, une lumière. Composez directement dans l’image."
            )
        })
        .small()
        .color(theme::TEXT_DIM),
    );
    ui.add_space(10.0);
    ui.horizontal_wrapped(|ui| {
        for &page in pages {
            let response =
                ui.selectable_label(state.inspector == page, workshops::label(page, dimension));
            #[cfg(test)]
            ui.data_mut(|d| d.insert_temp(Id::new(("simple-page", page.label())), response.rect));
            if response.clicked() {
                state.inspector = page;
            }
        }
    });
    ui.add_space(8.0);
    ui.separator();
    ui.push_id(("simple-inspector", state.inspector), |ui| {
        panels::column(ui, |ui| {
            ui.heading(workshops::label(state.inspector, dimension));
            ui.add_space(8.0);
            // Keep incomplete wiring available in Advanced; do not overwrite it
            // through controls which display the last valid render inputs.
            if state.scene.creation_draft.is_some() {
                ui.label(syfrack_i18n::tr!(
                    "Un graphe en cours attend d’être complété dans l’interface avancée."
                ));
            } else {
                match (dimension, state.inspector) {
                    (Dimension::TwoD, Inspector::Scene) => sources(ui, state, false),
                    (Dimension::TwoD, Inspector::Material) => sources(ui, state, true),
                    (Dimension::TwoD, _) => workshops::framing_2d(ui, state),
                    (_, Inspector::Scene) => workshops::volume(ui, state),
                    _ => workshops::show(ui, state),
                }
            }
            ui.add_space(16.0);
            ui.separator();
            ui.small(syfrack_i18n::tr!(
                "Envie d’aller plus loin ? Retrouvez chaque outil dans l’interface avancée."
            ));
            let response = ui.button(syfrack_i18n::tr!("Ouvrir le graphe avancé"));
            #[cfg(test)]
            ui.data_mut(|d| d.insert_temp(Id::new("simple-open-advanced"), response.rect));
            if response.clicked() {
                state.tab = Tab::Forme;
            }
        });
    });
    if state.tab == Tab::Forme {
        state.set_interface(InterfaceMode::Advanced);
    }
}

// Follow connected branches and iteration bodies. Parked sources are retained
// in the document but must not present apparently ineffective creative knobs.
fn active_nodes(g: &CreationGraph) -> std::collections::BTreeSet<u32> {
    let mut active = std::collections::BTreeSet::new();
    let mut todo: Vec<_> = g
        .nodes
        .iter()
        .filter(|n| matches!(n.kind, CreationKind::Output))
        .map(|n| n.id)
        .collect();
    while let Some(id) = todo.pop() {
        if !active.insert(id) {
            continue;
        }
        todo.extend(g.links.iter().filter(|l| l.to.0 == id).map(|l| l.from.0));
        if let Some(n) = g.node(id) {
            if let CreationKind::Form(syfrack_core::NodeKind::Iterate { body, .. }) = &n.kind {
                todo.extend(body);
            }
        }
    }
    active
}

fn pattern(kind: &mut CreationKind) -> Option<(&mut PlanarFractal, &'static str)> {
    match kind {
        CreationKind::Pattern { fractal, .. }
        | CreationKind::Field(FieldOp::Planar { fractal, .. }) => Some((fractal, "pattern")),
        CreationKind::Texture(t) | CreationKind::Field(FieldOp::Texture { texture: t }) => {
            t.planar.as_mut().map(|p| (p, "texture.planar"))
        }
        _ => None,
    }
}

fn palette(kind: &mut CreationKind) -> Option<&mut Ramp> {
    match kind {
        CreationKind::Palette(r) | CreationKind::Field(FieldOp::Palette { ramp: r }) => Some(r),
        CreationKind::Texture(t) | CreationKind::Field(FieldOp::Texture { texture: t }) => {
            Some(&mut t.ramp)
        }
        CreationKind::Flame(f) => Some(&mut f.flame.palette),
        _ => None,
    }
}

fn sources(ui: &mut Ui, state: &mut AppState, colours: bool) {
    let Some(graph) = state.scene.creation.clone() else {
        return;
    };
    let active = active_nodes(&graph);
    let sources: Vec<_> = graph
        .nodes
        .iter()
        .filter(|n| active.contains(&n.id) && n.enabled)
        .filter(|n| {
            let mut kind = n.kind.clone();
            if colours {
                // In 2D a Texture scalar is coloured by its connected Palette,
                // whose ramp (not the dormant surface ramp) owns the final colour.
                matches!(
                    kind,
                    CreationKind::Palette(_)
                        | CreationKind::Field(FieldOp::Palette { .. })
                        | CreationKind::Flame(_)
                )
            } else {
                pattern(&mut kind).is_some() || matches!(kind, CreationKind::Flame(_))
            }
        })
        .collect();
    if sources.is_empty() {
        ui.label(syfrack_i18n::tr!("Cette composition utilise des sources spécialisées. Le graphe avancé donne accès à leurs réglages."));
        let response = ui.button(syfrack_i18n::tr!("Créer un motif 2D"));
        #[cfg(test)]
        ui.data_mut(|d| d.insert_temp(Id::new("simple-add-pattern"), response.rect));
        if response.clicked() {
            if let Err(error) = super::creation::add_pattern(state) {
                state.banner = Some(crate::state::Banner::error(error));
            }
        }
        return;
    }
    let key = Id::new(("simple-source", colours, state.document_generation));
    let mut selected = ui
        .data(|d| d.get_temp::<u32>(key))
        .filter(|id| sources.iter().any(|n| n.id == *id))
        .unwrap_or(sources[0].id);
    if sources.len() > 1 {
        egui::ComboBox::from_id_salt(key)
            .width(ui.available_width() - 16.0)
            .selected_text(&sources.iter().find(|n| n.id == selected).unwrap().name)
            .show_ui(ui, |ui| {
                for n in &sources {
                    ui.selectable_value(&mut selected, n.id, &n.name);
                }
            });
    }
    ui.data_mut(|d| d.insert_temp(key, selected));
    let mut kind = graph.node(selected).unwrap().kind.clone();
    if colours {
        if let Some(ramp) = palette(&mut kind) {
            ui.label(RichText::new(syfrack_i18n::tr!("Palettes néon")).strong());
            ui.horizontal_wrapped(|ui| {
                for (index, name) in syfrack_core::scene::palette::NEON_NAMES.iter().enumerate() {
                    let preset = syfrack_core::scene::palette::neon_palette(index);
                    let response = widgets::choice_button(
                        ui,
                        ramp.stops == preset.stops,
                        syfrack_i18n::text(name),
                    );
                    #[cfg(test)]
                    ui.data_mut(|d| {
                        d.insert_temp(Id::new(("simple-palette", index)), response.rect)
                    });
                    if response.clicked() {
                        ramp.stops = preset.stops;
                    }
                }
            });
            let (rect, _) = ui
                .allocate_exact_size(egui::vec2(ui.available_width(), 20.0), egui::Sense::hover());
            for i in 0..128 {
                let colour = ramp.sample((i as f32 / 128.0 - ramp.phase) / ramp.frequency);
                ui.painter().rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(rect.left() + i as f32 * rect.width() / 128.0, rect.top()),
                        egui::pos2(
                            rect.left() + (i + 1) as f32 * rect.width() / 128.0 + 0.5,
                            rect.bottom(),
                        ),
                    ),
                    0.0,
                    widgets::to_srgb32(widgets::developed(colour, &state.scene.look)),
                );
            }
            egui::CollapsingHeader::new(syfrack_i18n::tr!("Personnaliser la palette")).show(
                ui,
                |ui| {
                    widgets::ramp_editor(ui, ramp);
                },
            );
        }
    } else if let Some((p, _)) = pattern(&mut kind) {
        ui.label(syfrack_i18n::tr!("Choisir une famille"));
        ui.horizontal_wrapped(|ui| {
            use syfrack_core::PlanarFamily::*;
            for (family, name) in [
                (Julia, "Julia"),
                (Mandelbrot, "Mandelbrot"),
                (BurningShip, "Burning Ship"),
                (Newton, "Newton"),
                (Nova, "Nova"),
                (Phoenix, "Phoenix"),
            ] {
                let response = widgets::choice_button(ui, p.family == family, name);
                #[cfg(test)]
                ui.data_mut(|d| d.insert_temp(Id::new(("simple-family", name)), response.rect));
                if response.clicked() && p.family != family {
                    *p = workshops::family_start(family);
                }
            }
        });
    }
    if graph.node(selected).unwrap().kind != kind {
        state.edit(|s| {
            if let Some(node) = s
                .creation
                .as_mut()
                .and_then(|g| g.nodes.iter_mut().find(|n| n.id == selected))
            {
                node.kind = kind.clone();
            }
        });
    }
    let prefix = format!("creation.nodes[{selected}]");
    if !colours {
        if let Some((p, suffix)) = pattern(&mut kind) {
            let family = p.family;
            let prefix = format!("{prefix}.{suffix}");
            if matches!(
                family,
                syfrack_core::PlanarFamily::Julia
                    | syfrack_core::PlanarFamily::Nova
                    | syfrack_core::PlanarFamily::Phoenix
            ) {
                for (field, label) in [
                    ("julia[0]", syfrack_i18n::tr!("Courbes · X")),
                    ("julia[1]", syfrack_i18n::tr!("Courbes · Y")),
                ] {
                    control(
                        ui,
                        state,
                        &format!("{prefix}.{field}"),
                        label,
                        -1.3,
                        1.3,
                        false,
                    );
                }
            }
            for (field, label, lo, hi, integer) in [
                ("power", syfrack_i18n::tr!("Ramifications"), 2.0, 8.0, true),
                (
                    "twist",
                    syfrack_i18n::tr!("Torsion des branches"),
                    -30.0,
                    30.0,
                    false,
                ),
                (
                    "inversion",
                    syfrack_i18n::tr!("Courbure du motif"),
                    0.0,
                    2.0,
                    false,
                ),
                (
                    "iterations",
                    syfrack_i18n::tr!("Détail et rythme"),
                    32.0,
                    384.0,
                    true,
                ),
            ] {
                control(
                    ui,
                    state,
                    &format!("{prefix}.{field}"),
                    label,
                    lo,
                    hi,
                    integer,
                );
            }
        } else {
            // Orbit sources expose artistic controls, retaining resource budgets
            // and individual transformations in the advanced graph inspector.
            for (field, label, lo, hi) in [
                (
                    "flame.brightness",
                    syfrack_i18n::tr!("Incandescence"),
                    0.1,
                    6.0,
                ),
                (
                    "flame.density_log",
                    syfrack_i18n::tr!("Souplesse des densités"),
                    0.0,
                    32.0,
                ),
                ("angle", "Rotation", -180.0, 180.0),
            ] {
                control(
                    ui,
                    state,
                    &format!("{prefix}.{field}"),
                    label,
                    lo,
                    hi,
                    false,
                );
            }
        }
    } else {
        let suffix = if matches!(kind, CreationKind::Flame(_)) {
            "flame.palette"
        } else {
            "palette"
        };
        for (field, label, lo, hi) in [
            (
                "frequency",
                syfrack_i18n::tr!("Finesse des couleurs"),
                1.0,
                34.0,
            ),
            ("phase", syfrack_i18n::tr!("Faire défiler"), -1.0, 1.0),
            ("gamma", syfrack_i18n::tr!("Répartition"), 0.25, 4.0),
        ] {
            control(
                ui,
                state,
                &format!("{prefix}.{suffix}.{field}"),
                label,
                lo,
                hi,
                false,
            );
        }
        workshops::neon_softness(ui, state);
        control(
            ui,
            state,
            "look.exposure_ev",
            syfrack_i18n::tr!("Luminosité"),
            -2.0,
            2.0,
            false,
        );
        control(
            ui,
            state,
            "look.saturation",
            syfrack_i18n::tr!("Éclat des couleurs"),
            0.0,
            2.0,
            false,
        );
    }
}

fn control(
    ui: &mut Ui,
    state: &mut AppState,
    path: &str,
    label: &str,
    lo: f32,
    hi: f32,
    integer: bool,
) {
    workshops::knob(
        ui,
        state,
        path,
        label,
        lo,
        hi,
        false,
        integer,
        syfrack_i18n::tr!("Ajuste la composition ; Ctrl+Z permet de revenir en arrière."),
    );
}
