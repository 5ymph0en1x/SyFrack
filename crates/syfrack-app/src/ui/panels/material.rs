use syfrack_core::{
    Coloring, FanMapping, FractalTexture, Graph, OrbitColoring, Ramp, SurfaceVariation,
    TextureChannel, TextureMask,
};

use super::{advanced, labeled, param, rgb_bounds, rule, toggle};
use crate::state::AppState;
use crate::ui::widgets::{linear_color, ramp_editor};
use std::sync::OnceLock;

/// The colouring choices: none, the four trap slots of the form, then the
/// iteration (4) and normal (5) pseudo-slots.
pub const TRAP_CHOICES: [&str; 8] = [
    "Aucun",
    "Piège 1",
    "Piège 2",
    "Piège 3",
    "Piège 4",
    "Itérations",
    "Normale",
    "Mélange des orbites",
];

/// The choice showing a shader slot (`Coloring::slot`, 0xffff for none).
pub fn choice_of_slot(slot: u32) -> usize {
    match slot {
        s @ 0..=3 => s as usize + 1,
        4 => 5,
        5 => 6,
        6 => 7,
        _ => 0,
    }
}

/// The colouring a choice asks for, keeping `ramp`.
pub fn coloring_of_choice(choice: usize, ramp: Ramp) -> Coloring {
    match choice {
        0 => Coloring::None,
        c @ 1..=4 => Coloring::Trap {
            slot: c as u32 - 1,
            ramp,
        },
        5 => Coloring::Trap { slot: 4, ramp },
        6 => Coloring::Trap { slot: 5, ramp },
        _ => Coloring::Orbit {
            channels: OrbitColoring::default(),
            ramp,
        },
    }
}

/// The ramp a colouring starts with when none was set.
fn default_ramp() -> Ramp {
    Ramp {
        stops: vec![[0.6, 0.3, 0.2], [0.9, 0.85, 0.75]],
        ..Ramp::default()
    }
}

/// Albedo, roughness and metal read together; then the colouring and its
/// ramp; the dielectric reflectance under « Avancé ».
fn show_optics(ui: &mut egui::Ui, state: &mut AppState, finish: Option<usize>) {
    let existing = finish.map_or_else(
        || state.scene.material.optics.clone(),
        |i| state.scene.material.finishes[i].optics.clone(),
    );
    let prefix = finish.map_or_else(
        || "material.optics".to_string(),
        |i| format!("material.finishes[{i}].optics"),
    );
    ui.push_id(&prefix, |ui| {
        if let Some(on) = toggle(
            ui,
            syfrack_i18n::tr!("Transmission et diffusion"),
            existing.is_some(),
        ) {
            state.edit(|s| {
                let value = on.then(syfrack_core::Optics::default);
                if let Some(i) = finish {
                    s.material.finishes[i].optics = value;
                } else {
                    s.material.optics = value;
                }
            });
        }
        let optics = finish.map_or_else(
            || state.scene.material.optics.clone(),
            |i| state.scene.material.finishes[i].optics.clone(),
        );
        let Some(optics) = optics else {
            return;
        };
        for (field, label) in [
            ("opacity", syfrack_i18n::tr!("Opacité de la surface")),
            ("transmission", "Transmission"),
            ("ior", syfrack_i18n::tr!("Indice de réfraction")),
            (
                "absorption_distance",
                syfrack_i18n::tr!("Distance d’absorption"),
            ),
            ("subsurface", syfrack_i18n::tr!("Diffusion sous la surface")),
            (
                "subsurface_radius",
                syfrack_i18n::tr!("Libre parcours moyen"),
            ),
        ] {
            param(&format!("{prefix}.{field}"), label).show(ui, state);
        }
        if let Some(on) = toggle(ui, syfrack_i18n::tr!("Paroi mince"), optics.thin_walled) {
            state.edit(|s| {
                let o = if let Some(i) = finish {
                    s.material.finishes[i].optics.as_mut().unwrap()
                } else {
                    s.material.optics.as_mut().unwrap()
                };
                o.thin_walled = on;
            });
        }
        for (field, label, mut color) in [
            (
                "absorption_color",
                syfrack_i18n::tr!("Couleur transmise"),
                optics.absorption_color,
            ),
            (
                "subsurface_color",
                syfrack_i18n::tr!("Couleur de diffusion"),
                optics.subsurface_color,
            ),
        ] {
            if linear_color(
                ui,
                label,
                &mut color,
                rgb_bounds(&state.scene, &format!("{prefix}.{field}")),
                &state.scene.look,
            ) {
                state.edit(|s| {
                    let o = if let Some(i) = finish {
                        s.material.finishes[i].optics.as_mut().unwrap()
                    } else {
                        s.material.optics.as_mut().unwrap()
                    };
                    if field == "absorption_color" {
                        o.absorption_color = color;
                    } else {
                        o.subsurface_color = color;
                    }
                });
            }
        }
        ui.small(syfrack_i18n::tr!(
            "Rugosité : verre poli ou dépoli. Augmenter les rebonds pour les volumes diffusants."
        ));
    });
}

pub fn show(ui: &mut egui::Ui, state: &mut AppState) {
    let mut albedo = state.scene.material.albedo;
    let bounds = rgb_bounds(&state.scene, "material.albedo");
    ui.label(egui::RichText::new("Surface").strong());
    if linear_color(
        ui,
        syfrack_i18n::tr!("Couleur de base"),
        &mut albedo,
        bounds,
        &state.scene.look,
    ) {
        state.edit(|s| s.material.albedo = albedo);
    }
    ui.add_enabled_ui(state.scene.material.surface.is_none(), |ui| {
        param("material.roughness", syfrack_i18n::tr!("Rugosité")).show(ui, state);
        param("material.metallic", syfrack_i18n::tr!("Métal")).show(ui, state);
    });
    rule(ui);
    let current = choice_of_slot(state.scene.material.coloring.slot());
    show_optics(ui, state, None);
    let mut chosen = current;
    // Seven choices do not fit one row of the column: a list.
    ui.label(egui::RichText::new(syfrack_i18n::tr!("Coloration de la forme")).strong());
    labeled(ui, "Source", |ui| {
        egui::ComboBox::from_id_salt("coloring")
            .width(ui.available_width())
            .selected_text(syfrack_i18n::text(TRAP_CHOICES[current]))
            .show_ui(ui, |ui| {
                for (i, name) in TRAP_CHOICES.iter().enumerate() {
                    ui.selectable_value(&mut chosen, i, syfrack_i18n::text(name));
                }
            });
    });
    if chosen != current {
        // The ramp survives a change of slot.
        let ramp = state
            .scene
            .material
            .coloring
            .ramp()
            .cloned()
            .unwrap_or_else(default_ramp);
        state.edit(|s| s.material.coloring = coloring_of_choice(chosen, ramp));
    }
    if matches!(state.scene.material.coloring, Coloring::Orbit { .. }) {
        ui.small(syfrack_i18n::tr!(
            "Les poids mélangent les mesures enregistrées dans le graphe de forme."
        ));
        for (field, label) in [
            ("traps[0]", syfrack_i18n::tr!("Poids du piège 1")),
            ("traps[1]", syfrack_i18n::tr!("Poids du piège 2")),
            ("traps[2]", syfrack_i18n::tr!("Poids du piège 3")),
            ("traps[3]", syfrack_i18n::tr!("Poids du piège 4")),
            ("iterations", syfrack_i18n::tr!("Poids des itérations")),
            ("normal", syfrack_i18n::tr!("Poids de la normale")),
            ("logarithmic", syfrack_i18n::tr!("Étalement logarithmique")),
        ] {
            param(&format!("material.coloring.orbit.channels.{field}"), label).show(ui, state);
        }
    }
    if let Some(mut ramp) = state.scene.material.coloring.ramp().cloned() {
        if ramp_editor(ui, &mut ramp) {
            state.edit(|s| {
                if let Some(r) = s.material.coloring.ramp_mut() {
                    *r = ramp;
                }
            });
        }
    }
    advanced(ui, "material", |ui| {
        param("material.specular", syfrack_i18n::tr!("Spéculaire"))
            .decimals(3)
            .show(ui, state);
    });
    rule(ui);
    show_surface_variation(ui, state);
    rule(ui);
    show_finishes(ui, state);
    rule(ui);
    if ui
        .button(syfrack_i18n::tr!("Ouvrir la texture fractale →"))
        .clicked()
    {
        state.inspector = crate::state::Inspector::Texture;
    }
}

pub(crate) fn show_finishes(ui: &mut egui::Ui, state: &mut AppState) {
    ui.label(egui::RichText::new(syfrack_i18n::tr!("Matériaux de la scène")).strong());
    ui.small(syfrack_i18n::tr!(
        "Attribuer un matériau à une forme avec l’élément « Matériau de branche »."
    ));
    let finishes = state.scene.material.finishes.clone();
    for (i, finish) in finishes.into_iter().enumerate() {
        egui::CollapsingHeader::new(&finish.name)
            .id_salt(("finish", i))
            .show(ui, |ui| {
                let mut name = finish.name.clone();
                labeled(ui, syfrack_i18n::tr!("Nom"), |ui| {
                    if ui.text_edit_singleline(&mut name).changed() {
                        state.edit(|s| s.material.finishes[i].name = name.clone());
                    }
                });
                let prefix = format!("material.finishes[{i}]");
                let mut color = finish.albedo;
                if linear_color(
                    ui,
                    syfrack_i18n::tr!("Couleur"),
                    &mut color,
                    rgb_bounds(&state.scene, &format!("{prefix}.albedo")),
                    &state.scene.look,
                ) {
                    state.edit(|s| s.material.finishes[i].albedo = color);
                }
                param(
                    &format!("{prefix}.roughness"),
                    syfrack_i18n::tr!("Rugosité"),
                )
                .show(ui, state);
                param(&format!("{prefix}.metallic"), syfrack_i18n::tr!("Métal")).show(ui, state);
                show_finish_detail(ui, state, i, &prefix);
                show_optics(ui, state, Some(i));
                advanced(ui, &format!("finish_{i}"), |ui| {
                    param(
                        &format!("{prefix}.specular"),
                        syfrack_i18n::tr!("Spéculaire"),
                    )
                    .show(ui, state);
                    ui.small(syfrack_i18n::tr!("Émission lumineuse · radiance linéaire"));
                    for (axis, label) in [
                        ("r", syfrack_i18n::tr!("Rouge")),
                        ("g", syfrack_i18n::tr!("Vert")),
                        ("b", syfrack_i18n::tr!("Bleu")),
                    ] {
                        param(&format!("{prefix}.emission.{axis}"), label).show(ui, state);
                    }
                });
            });
    }
    if state.scene.material.finishes.len() < syfrack_core::scene::material::MAX_FINISHES
        && ui
            .button(syfrack_i18n::tr!("Ajouter un matériau"))
            .clicked()
    {
        state.edit(|s| s.material.finishes.push(syfrack_core::Finish::default()));
    }
    if let Some(last) = state.scene.material.finishes.last() {
        let index = state.scene.material.finishes.len() as u32;
        let used = state.scene.fractal.graph().nodes.iter().any(
            |n| matches!(n.kind, syfrack_core::NodeKind::MaterialTag { index: k } if k == index),
        ) || state
            .scene
            .atmosphere
            .ground
            .as_ref()
            .is_some_and(|g| g.finish == index);
        if ui
            .add_enabled(
                !used,
                egui::Button::new(syfrack_i18n::tr_format!("Retirer {}", last.name)),
            )
            .on_hover_text(syfrack_i18n::tr!(
                "Seul un matériau sans référence peut être retiré."
            ))
            .clicked()
        {
            state.edit(|s| {
                s.material.finishes.pop();
            });
        }
    }
}

fn show_finish_detail(ui: &mut egui::Ui, state: &mut AppState, i: usize, prefix: &str) {
    use syfrack_core::{FinishDetail, FinishPattern};
    let detail = state.scene.material.finishes[i].detail.clone();
    if let Some(on) = toggle(
        ui,
        syfrack_i18n::tr!("Texture personnalisée"),
        detail.is_some(),
    ) {
        state.edit(|s| s.material.finishes[i].detail = on.then(FinishDetail::default));
    }
    let Some(detail) = state.scene.material.finishes[i].detail.clone() else {
        return;
    };
    let patterns = [
        (
            FinishPattern::VeinedStone,
            syfrack_i18n::tr!("Pierre veinée"),
        ),
        (
            FinishPattern::Patina,
            syfrack_i18n::tr!("Patine métallique"),
        ),
        (
            FinishPattern::BrushedMetal,
            syfrack_i18n::tr!("Métal brossé"),
        ),
        (FinishPattern::Basalt, syfrack_i18n::tr!("Grain minéral")),
        (FinishPattern::Ripples, syfrack_i18n::tr!("Ondulations")),
        (
            FinishPattern::Rosettes,
            syfrack_i18n::tr!("Rosaces gravées"),
        ),
        (
            FinishPattern::CoastalRock,
            syfrack_i18n::tr!("Roche littorale"),
        ),
        (FinishPattern::Sand, syfrack_i18n::tr!("Sable éolien")),
        (
            FinishPattern::Dust,
            syfrack_i18n::tr!("Dépôts de poussière"),
        ),
        (FinishPattern::Concrete, syfrack_i18n::tr!("Béton rugueux")),
        (
            FinishPattern::WeatheredConcrete,
            syfrack_i18n::tr!("Ciment ruisselé"),
        ),
        (
            FinishPattern::WetConcrete,
            syfrack_i18n::tr!("Ciment et flaques"),
        ),
    ];
    let mut selected = detail.pattern;
    labeled(ui, "Structure", |ui| {
        egui::ComboBox::from_id_salt(("finish_texture", i))
            .selected_text(patterns.iter().find(|(p, _)| *p == selected).unwrap().1)
            .show_ui(ui, |ui| {
                for (pattern, name) in patterns {
                    ui.selectable_value(&mut selected, pattern, name);
                }
            });
    });
    if selected != detail.pattern {
        state.edit(|s| s.material.finishes[i].detail.as_mut().unwrap().pattern = selected);
    }
    let path = format!("{prefix}.detail");
    param(&format!("{path}.scale"), syfrack_i18n::tr!("Fréquence"))
        .log()
        .show(ui, state);
    let wet = selected == FinishPattern::WetConcrete;
    param(
        &format!("{path}.strength"),
        if wet {
            syfrack_i18n::tr!("Présence des flaques")
        } else {
            syfrack_i18n::tr!("Variation de couleur")
        },
    )
    .show(ui, state);
    param(
        &format!("{path}.roughness_delta"),
        syfrack_i18n::tr!("Variation de rugosité"),
    )
    .show(ui, state);
    param(
        &format!("{path}.bump"),
        syfrack_i18n::tr!("Relief / gravure"),
    )
    .decimals(3)
    .show(ui, state);
    let mut tint = detail.tint;
    if linear_color(
        ui,
        syfrack_i18n::tr!("Teinte de variation"),
        &mut tint,
        rgb_bounds(&state.scene, &format!("{path}.tint")),
        &state.scene.look,
    ) {
        state.edit(|s| s.material.finishes[i].detail.as_mut().unwrap().tint = tint);
    }
    param(&format!("{path}.seed"), syfrack_i18n::tr!("Variante"))
        .decimals(0)
        .show(ui, state);
    ui.small(syfrack_i18n::tr!(
        "Texture 3D sans raccord ; les détails trop fins sont filtrés selon la taille du pixel."
    ));
    if wet {
        ui.small(syfrack_i18n::tr!("Une variation de rugosité négative lisse les flaques ; la teinte de variation colore les zones mouillées."));
    }
}

fn show_surface_variation(ui: &mut egui::Ui, state: &mut AppState) {
    if let Some(on) = toggle(
        ui,
        syfrack_i18n::tr!("Finitions selon le piège"),
        state.scene.material.surface.is_some(),
    ) {
        state.edit(|s| s.material.surface = on.then(SurfaceVariation::default));
    }
    if state.scene.material.surface.is_none() {
        return;
    }
    ui.small(syfrack_i18n::tr!(
        "Les valeurs de finition remplacent la rugosité et le métal uniformes."
    ));
    let current = state.scene.material.surface.as_ref().unwrap().slot;
    let mut slot = current;
    labeled(ui, syfrack_i18n::tr!("Piège de finition"), |ui| {
        egui::ComboBox::from_id_salt("surface_variation_channel")
            .width(ui.available_width())
            .selected_text(syfrack_i18n::text(TRAP_CHOICES[current as usize + 1]))
            .show_ui(ui, |ui| {
                for (i, name) in TRAP_CHOICES.iter().enumerate().skip(1).take(6) {
                    ui.selectable_value(&mut slot, (i - 1) as u32, syfrack_i18n::text(name));
                }
            });
    });
    if slot != current {
        state.edit(|s| s.material.surface.as_mut().unwrap().slot = slot);
    }
    for (i, name) in [
        (0, syfrack_i18n::tr!("Début du piège")),
        (1, syfrack_i18n::tr!("Fin du piège")),
    ] {
        ui.small(name);
        param(
            &format!("material.surface.roughness[{i}]"),
            syfrack_i18n::tr!("Rugosité"),
        )
        .show(ui, state);
        param(
            &format!("material.surface.metallic[{i}]"),
            syfrack_i18n::tr!("Métal"),
        )
        .show(ui, state);
    }
}

fn texture_sources() -> &'static [(Graph, &'static str)] {
    static SOURCES: OnceLock<Vec<(Graph, &'static str)>> = OnceLock::new();
    SOURCES.get_or_init(|| {
        [
            ("mandelbox", "Mandelbox"),
            ("mandelbulb", "Mandelbulb"),
            ("menger", "Menger"),
            ("menger-kaleido", "Menger kaléido"),
            ("cathedrale", "Cathédrale"),
            ("boite-bulbe", "Boîte bulbe"),
            ("evidement", "Évidement"),
            ("sphere", "Sphère"),
        ]
        .into_iter()
        .filter_map(|(key, name)| Graph::bundled(key).map(|g| (g, name)))
        .collect()
    })
}

fn has_trap(graph: &Graph, slot: u32) -> bool {
    let mut nodes = graph.active_nodes();
    while let Some(id) = nodes.pop() {
        let Some(n) = graph.node(id) else { continue };
        if !n.enabled {
            continue;
        }
        match &n.kind {
            syfrack_core::NodeKind::Trap { slot: s, .. } if *s == slot => return true,
            syfrack_core::NodeKind::Iterate { body, .. } => nodes.extend(body),
            _ => {}
        }
    }
    false
}

pub fn show_texture(ui: &mut egui::Ui, state: &mut AppState) {
    let on = state.scene.material.texture.is_some();
    if let Some(on) = toggle(ui, syfrack_i18n::tr!("Texture fractale"), on) {
        state.edit(|s| s.material.texture = on.then(FractalTexture::default));
    }
    if state.scene.material.texture.is_none() {
        ui.label(syfrack_i18n::tr!(
            "Active une texture pour générer un motif, varier la rugosité et ajouter du relief."
        ));
        return;
    }
    texture_presets(ui, state);
    crate::ui::texture_preview::show(ui, &state.scene);
    rule(ui);
    let Some(texture) = &state.scene.material.texture else {
        return;
    };
    let sources = texture_sources();
    let name = sources
        .iter()
        .find(|(graph, _)| *graph == texture.graph)
        .map(|(_, name)| *name)
        .unwrap_or(syfrack_i18n::tr!("Personnalisée"));
    let mut source = None;
    labeled(ui, "Source", |ui| {
        egui::ComboBox::from_id_salt("texture_source")
            .width(ui.available_width())
            .selected_text(syfrack_i18n::text(name))
            .show_ui(ui, |ui| {
                for (graph, name) in sources {
                    if ui
                        .selectable_label(*graph == texture.graph, syfrack_i18n::text(name))
                        .clicked()
                    {
                        source = Some(graph.clone());
                    }
                }
            });
    });
    if ui
        .add_enabled(
            state.scene.fractal.graph().validate().is_ok(),
            egui::Button::new(syfrack_i18n::tr!("Copier la forme actuelle")),
        )
        .on_hover_text(syfrack_i18n::tr!(
            "La copie devient une texture indépendante de la géométrie."
        ))
        .clicked()
    {
        source = Some(state.scene.fractal.to_graph());
    }
    if let Some(graph) = source {
        state.edit(|s| s.material.texture.as_mut().unwrap().graph = graph);
    }
    let channel = state.scene.material.texture.as_ref().unwrap().channel;
    let mut chosen = channel;
    let channels = [
        (
            TextureChannel::Trap { slot: 0 },
            syfrack_i18n::tr!("Piège 1"),
        ),
        (
            TextureChannel::Trap { slot: 1 },
            syfrack_i18n::tr!("Piège 2"),
        ),
        (
            TextureChannel::Trap { slot: 2 },
            syfrack_i18n::tr!("Piège 3"),
        ),
        (
            TextureChannel::Trap { slot: 3 },
            syfrack_i18n::tr!("Piège 4"),
        ),
        (TextureChannel::Iterations, syfrack_i18n::tr!("Itérations")),
        (TextureChannel::Distance, "Distance"),
        (
            TextureChannel::Orbit {
                channels: OrbitColoring::default(),
            },
            syfrack_i18n::tr!("Mélange des orbites"),
        ),
    ];
    let channel_name = channels
        .iter()
        .find(|(c, _)| {
            *c == channel
                || matches!(
                    (c, channel),
                    (TextureChannel::Orbit { .. }, TextureChannel::Orbit { .. })
                )
        })
        .map(|(_, n)| *n)
        .unwrap_or(syfrack_i18n::tr!("Piège 1"));
    labeled(ui, syfrack_i18n::tr!("Motif"), |ui| {
        egui::ComboBox::from_id_salt("texture_channel")
            .width(ui.available_width())
            .selected_text(channel_name)
            .show_ui(ui, |ui| {
                for (channel, name) in channels {
                    ui.selectable_value(&mut chosen, channel, syfrack_i18n::text(name));
                }
            });
    });
    if chosen != channel {
        state.edit(|s| s.material.texture.as_mut().unwrap().channel = chosen);
    }
    if matches!(
        state.scene.material.texture.as_ref().unwrap().channel,
        TextureChannel::Orbit { .. }
    ) {
        for (field, label) in [
            ("traps[0]", syfrack_i18n::tr!("Poids du piège 1")),
            ("traps[1]", syfrack_i18n::tr!("Poids du piège 2")),
            ("traps[2]", syfrack_i18n::tr!("Poids du piège 3")),
            ("traps[3]", syfrack_i18n::tr!("Poids du piège 4")),
            ("iterations", syfrack_i18n::tr!("Poids des itérations")),
            ("normal", syfrack_i18n::tr!("Décalage")),
            ("logarithmic", syfrack_i18n::tr!("Étalement logarithmique")),
        ] {
            param(&format!("material.texture.orbit.{field}"), label).show(ui, state);
        }
    }
    let texture = state.scene.material.texture.as_ref().unwrap();
    if let TextureChannel::Trap { slot } = texture.channel {
        if !has_trap(&texture.graph, slot) {
            ui.small(syfrack_i18n::tr!(
                "Ce piège est vide dans la source : choisis un autre motif."
            ));
        }
    }
    let mut ramp = texture.ramp.clone();
    if ramp_editor(ui, &mut ramp) {
        state.edit(|s| s.material.texture.as_mut().unwrap().ramp = ramp);
    }
    param("material.texture.strength", syfrack_i18n::tr!("Couleur")).show(ui, state);
    param("material.texture.scale", syfrack_i18n::tr!("Échelle"))
        .log()
        .show(ui, state);
    param("material.texture.octaves", syfrack_i18n::tr!("Détails"))
        .decimals(0)
        .show(ui, state);
    param("material.texture.contrast", syfrack_i18n::tr!("Contraste")).show(ui, state);
    param("material.texture.roughness", syfrack_i18n::tr!("Rugosité")).show(ui, state);
    rule(ui);
    show_planar(ui, state);
    rule(ui);
    ui.label(egui::RichText::new(syfrack_i18n::tr!("Relief de la surface")).strong());
    param(
        "material.texture.bump",
        syfrack_i18n::tr!("Relief fin · bump"),
    )
    .decimals(3)
    .show(ui, state);
    param(
        "material.texture.parallax",
        syfrack_i18n::tr!("Profondeur · POM"),
    )
    .decimals(3)
    .show(ui, state);
    if state.scene.material.texture.as_ref().unwrap().parallax > 0.0 {
        param(
            "material.texture.parallax_steps",
            syfrack_i18n::tr!("Pas de parallax"),
        )
        .decimals(0)
        .show(ui, state);
    }
    ui.small(syfrack_i18n::tr!(
        "Bump : lumière sur les détails. Parallax : profondeur apparente du motif."
    ));
    rule(ui);
    if let Some(on) = toggle(
        ui,
        syfrack_i18n::tr!("Motif en éventail"),
        state.scene.material.texture.as_ref().unwrap().fan.is_some(),
    ) {
        state.edit(|s| s.material.texture.as_mut().unwrap().fan = on.then(FanMapping::default));
    }
    if state.scene.material.texture.as_ref().unwrap().fan.is_some() {
        param(
            "material.texture.fan.period[0]",
            syfrack_i18n::tr!("Largeur"),
        )
        .unit("m")
        .show(ui, state);
        param(
            "material.texture.fan.period[1]",
            syfrack_i18n::tr!("Hauteur"),
        )
        .unit("m")
        .show(ui, state);
        param("material.texture.fan.rays", syfrack_i18n::tr!("Rayons"))
            .decimals(0)
            .show(ui, state);
    }
    if let Some(on) = toggle(
        ui,
        syfrack_i18n::tr!("Masque selon le piège"),
        state
            .scene
            .material
            .texture
            .as_ref()
            .unwrap()
            .mask
            .is_some(),
    ) {
        state.edit(|s| s.material.texture.as_mut().unwrap().mask = on.then(TextureMask::default));
    }
    if state
        .scene
        .material
        .texture
        .as_ref()
        .unwrap()
        .mask
        .is_some()
    {
        let current = state
            .scene
            .material
            .texture
            .as_ref()
            .unwrap()
            .mask
            .as_ref()
            .unwrap()
            .slot;
        let mut slot = current;
        labeled(ui, syfrack_i18n::tr!("Piège du masque"), |ui| {
            egui::ComboBox::from_id_salt("texture_mask_channel")
                .selected_text(syfrack_i18n::text(TRAP_CHOICES[current as usize + 1]))
                .show_ui(ui, |ui| {
                    for (i, name) in TRAP_CHOICES.iter().enumerate().skip(1).take(4) {
                        ui.selectable_value(&mut slot, (i - 1) as u32, syfrack_i18n::text(name));
                    }
                });
        });
        if slot != current {
            state.edit(|s| {
                s.material
                    .texture
                    .as_mut()
                    .unwrap()
                    .mask
                    .as_mut()
                    .unwrap()
                    .slot = slot
            });
        }
        param(
            "material.texture.mask.value",
            syfrack_i18n::tr!("Valeur ciblée"),
        )
        .show(ui, state);
        param("material.texture.mask.width", "Transition")
            .decimals(3)
            .show(ui, state);
        ui.small(syfrack_i18n::tr!("Le masque utilise les pièges de la géométrie. Il limite couleur, rugosité et relief à la même région."));
    }
    egui::CollapsingHeader::new(syfrack_i18n::tr!("Placement de la texture")).show(ui, |ui| {
        for (path, label) in [
            ("material.texture.offset.x", "Position X"),
            ("material.texture.offset.y", "Position Y"),
            ("material.texture.offset.z", "Position Z"),
            ("material.texture.rotation.x", "Rotation X"),
            ("material.texture.rotation.y", "Rotation Y"),
            ("material.texture.rotation.z", "Rotation Z"),
        ] {
            param(path, label)
                .unit(if path.contains("rotation") { "°" } else { "" })
                .no_reference()
                .show(ui, state);
        }
    });
    texture_graph(ui, state);
}

/// Curated starting points affect only the independent texture, in one undo step.
pub fn preset(name: usize) -> FractalTexture {
    let mut texture = FractalTexture::default();
    match name {
        0 => {
            texture.roughness = 0.12;
            texture.bump = 0.025;
        }
        1 => {
            texture.fan = Some(FanMapping {
                period: [2.6, 2.2],
                rays: 12,
            });
            texture.scale = 1.6;
            texture.contrast = 1.3;
            texture.offset = glam::Vec3::new(0.0, 0.15, 0.0);
            texture.ramp = Ramp {
                stops: vec![
                    [0.09, 0.015, 0.035],
                    [0.28, 0.045, 0.085],
                    [0.55, 0.16, 0.23],
                    [0.85, 0.58, 0.48],
                ],
                ..Ramp::default()
            };
            texture.roughness = 0.1;
            texture.bump = 0.003;
            texture.parallax = 0.008;
        }
        _ => {
            texture.graph = Graph::bundled("menger").unwrap();
            texture.channel = TextureChannel::Distance;
            texture.scale = 2.0;
            texture.bump = 0.08;
            texture.parallax = 0.06;
            texture.roughness = 0.25;
        }
    }
    texture
}

fn texture_presets(ui: &mut egui::Ui, state: &mut AppState) {
    ui.menu_button(syfrack_i18n::tr!("Point de départ…"), |ui| {
        for (i, name) in [
            syfrack_i18n::tr!("Minéral fractal"),
            syfrack_i18n::tr!("Éventails de marbre rose"),
            syfrack_i18n::tr!("Gravure Menger"),
        ]
        .iter()
        .enumerate()
        {
            if ui.button(*name).clicked() {
                let mut texture = preset(i);
                // A preset keeps the artist's geometry mask and placement.
                if let Some(old) = &state.scene.material.texture {
                    texture.mask = old.mask.clone();
                    texture.offset = old.offset;
                    texture.rotation = old.rotation;
                }
                state.edit(|s| s.material.texture = Some(texture));
                ui.close();
            }
        }
    })
    .response
    .on_hover_text(syfrack_i18n::tr!(
        "Remplacer le motif et ses effets ; garder le masque et le placement. Ctrl+Z pour revenir."
    ));
}

fn texture_graph(ui: &mut egui::Ui, state: &mut AppState) {
    egui::CollapsingHeader::new(syfrack_i18n::tr!("Graphe de la texture · paramètres")).show(
        ui,
        |ui| {
            ui.small(syfrack_i18n::tr!(
                "Ces réglages modifient la source du motif indépendamment de la forme."
            ));
            let graph = state.scene.material.texture.as_ref().unwrap().graph.clone();
            let mut params = Vec::new();
            let mut copy = graph.clone();
            use syfrack_core::Params;
            copy.visit_params("material.texture.graph", &mut |path, _, _| {
                params.push(path.to_owned())
            });
            param(
                "material.texture.graph.safety",
                syfrack_i18n::tr!("Sûreté du pas"),
            )
            .no_reference()
            .show(ui, state);
            for node in &graph.nodes {
                let prefix = format!("material.texture.graph.nodes[{}].", node.id);
                let paths: Vec<_> = params.iter().filter(|p| p.starts_with(&prefix)).collect();
                if paths.is_empty() {
                    continue;
                }
                egui::CollapsingHeader::new(format!(
                    "{} · {}",
                    node.id,
                    if node.name.is_empty() {
                        node.kind.label()
                    } else {
                        &node.name
                    }
                ))
                .id_salt(("texture-node", node.id))
                .show(ui, |ui| {
                    for path in paths {
                        let field = path.strip_prefix(&prefix).unwrap();
                        let (name, axis) = field.split_once('.').unwrap_or((field, ""));
                        let label =
                            format!("{} {}", crate::ui::form_panel::field_label(name), axis);
                        let p = param(path, label.trim()).no_reference();
                        let p = if crate::ui::form_panel::is_integer(name) {
                            p.topology()
                        } else {
                            p
                        };
                        p.show(ui, state);
                    }
                });
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starting_points_validate_and_cover_relief_and_fans() {
        for i in 0..3 {
            preset(i).validate("material.texture").unwrap();
        }
        let fan = preset(1);
        assert!(fan.fan.is_some() && fan.parallax > 0.0 && fan.bump > 0.0);
        assert_eq!(fan.ramp.stops.len(), 4);
    }

    #[test]
    fn trap_detection_includes_enabled_iteration_bodies() {
        let mut graph = Graph::bundled("mandelbox").unwrap();
        assert!(has_trap(&graph, 0));
        assert!(!has_trap(&graph, 1));
        graph
            .nodes
            .iter_mut()
            .find(|n| matches!(n.kind, syfrack_core::NodeKind::Iterate { .. }))
            .unwrap()
            .enabled = false;
        assert!(!has_trap(&graph, 0));
    }

    #[test]
    fn every_choice_round_trips_through_its_slot_and_keeps_the_ramp() {
        let ramp = default_ramp();
        for (i, _) in TRAP_CHOICES.iter().enumerate() {
            let c = coloring_of_choice(i, ramp.clone());
            assert_eq!(choice_of_slot(c.slot()), i);
            if i > 0 {
                assert_eq!(c.ramp(), Some(&ramp));
                assert!(
                    matches!(c, Coloring::Trap { .. } | Coloring::Orbit { .. }),
                    "every enabled source has a palette"
                );
            }
        }
        assert_eq!(coloring_of_choice(5, ramp.clone()).slot(), 4, "Itérations");
        assert_eq!(coloring_of_choice(6, ramp).slot(), 5, "Normale");
        assert_eq!(choice_of_slot(Coloring::None.slot()), 0);
    }
}

fn show_planar(ui: &mut egui::Ui, state: &mut AppState) {
    use syfrack_core::{PlanarFamily, PlanarFractal};
    let existing = state
        .scene
        .material
        .texture
        .as_ref()
        .and_then(|t| t.planar.as_ref());
    if let Some(on) = toggle(ui, "Fractale complexe", existing.is_some()) {
        state.edit(|s| {
            s.material.texture.as_mut().unwrap().planar = on.then(PlanarFractal::default)
        });
    }
    let Some(planar) = state
        .scene
        .material
        .texture
        .as_ref()
        .and_then(|t| t.planar.clone())
    else {
        return;
    };
    let choices = [
        (PlanarFamily::Julia, "Julia"),
        (PlanarFamily::Mandelbrot, "Mandelbrot"),
        (PlanarFamily::BurningShip, "Burning Ship"),
        (PlanarFamily::Newton, "Newton"),
        (PlanarFamily::Nova, "Nova"),
        (PlanarFamily::Phoenix, "Phoenix"),
    ];
    let mut family = planar.family;
    labeled(ui, syfrack_i18n::tr!("Équation"), |ui| {
        egui::ComboBox::from_id_salt("planar_family")
            .selected_text(choices.iter().find(|(f, _)| *f == family).unwrap().1)
            .show_ui(ui, |ui| {
                for (f, label) in choices {
                    ui.selectable_value(&mut family, f, label);
                }
            });
    });
    if family != planar.family {
        state.edit(|s| {
            s.material
                .texture
                .as_mut()
                .unwrap()
                .planar
                .as_mut()
                .unwrap()
                .family = family
        });
    }
    ui.small(syfrack_i18n::tr!(
        "Pièges : 1 échappement lissé · 2 anneau · 3 bandes d’orbite · 4 angle final."
    ));
    for (field, label) in [
        ("power", syfrack_i18n::tr!("Puissance")),
        ("iterations", syfrack_i18n::tr!("Itérations")),
        ("zoom", "Zoom"),
        ("angle", syfrack_i18n::tr!("Rotation du cadre")),
        ("inversion", syfrack_i18n::tr!("Inversion du cadre")),
        ("twist", syfrack_i18n::tr!("Torsion des orbites")),
        ("relaxation", syfrack_i18n::tr!("Relaxation de Newton")),
        ("memory", syfrack_i18n::tr!("Mémoire de Phoenix")),
        ("bailout", syfrack_i18n::tr!("Rayon d’échappement")),
        ("trap_radius", syfrack_i18n::tr!("Rayon de l’anneau")),
        ("stripes", syfrack_i18n::tr!("Bandes angulaires")),
        ("julia[0]", syfrack_i18n::tr!("Constante réelle")),
        ("julia[1]", syfrack_i18n::tr!("Constante imaginaire")),
        ("center[0]", syfrack_i18n::tr!("Centre réel")),
        ("center[1]", syfrack_i18n::tr!("Centre imaginaire")),
    ] {
        let path = format!("material.texture.planar.{field}");
        let p = param(&path, label);
        if field == "zoom" {
            p.log().show(ui, state);
        } else {
            p.show(ui, state);
        }
    }
}
