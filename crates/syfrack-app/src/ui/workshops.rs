//! Artist-facing controls: each visible command edits an actual render input.
use super::{panels, theme, widgets};
use crate::state::{AppState, Inspector, Tab};
use egui::{RichText, Ui};
use syfrack_core::{
    Color, Coloring, Dimension, Light, NodeKind, OrbitColoring, PlanarFamily, PlanarFractal, Ramp,
    Scene, TextureChannel,
};

pub fn pages(dimension: Dimension) -> &'static [Inspector] {
    if dimension == Dimension::TwoD {
        &[Inspector::Scene, Inspector::Material, Inspector::Look]
    } else {
        &[
            Inspector::Scene,
            Inspector::Material,
            Inspector::Lights,
            Inspector::Look,
        ]
    }
}

pub fn label(page: Inspector, dimension: Dimension) -> &'static str {
    if dimension == Dimension::TwoD {
        match page {
            Inspector::Scene => syfrack_i18n::tr!("Motif"),
            Inspector::Material => syfrack_i18n::tr!("Couleurs"),
            _ => syfrack_i18n::tr!("Cadrage"),
        }
    } else {
        match page {
            Inspector::Scene => "Volume",
            Inspector::Atmosphere => syfrack_i18n::tr!("Ambiance"),
            _ => page.label(),
        }
    }
}

/// A useful creative range, with the current value retained for existing projects.
pub fn knob(
    ui: &mut Ui,
    state: &mut AppState,
    path: &str,
    label: &str,
    lo: f32,
    hi: f32,
    log: bool,
    integer: bool,
    help: &str,
) {
    // The unified graph owns form parameters; the lowered geometry is a cache.
    let unified = path.strip_prefix("fractal.nodes[").and_then(|suffix| {
        state
            .scene
            .creation
            .as_ref()
            .map(|_| format!("creation.nodes[{suffix}"))
    });
    let path = unified.as_deref().unwrap_or(path);
    let Some(info) = state.scene.params().into_iter().find(|p| p.path == path) else {
        return;
    };
    let mut value = info.value;
    let range = lo.min(value).max(info.bounds.min)..=hi.max(value).min(info.bounds.max);
    let bounds = syfrack_core::Bounds::new(*range.start(), *range.end(), &[]);
    let mut slider = widgets::ParamSlider::new(label, bounds).decimals(if integer { 0 } else { 3 });
    if log {
        slider = slider.log();
    }
    let response = slider.show(ui, &mut value).on_hover_text(help);
    #[cfg(test)]
    ui.data_mut(|d| {
        d.insert_temp(egui::Id::new(("creative-param", path)), response.rect);
        d.insert_temp(egui::Id::new(("param-row", path)), response.rect);
    });
    if integer {
        value = value.round();
    }
    if response.changed() && value != info.value {
        state.set_param(path, value);
    }
}

fn planar_knob(
    ui: &mut Ui,
    state: &mut AppState,
    field: &str,
    label: &str,
    lo: f32,
    hi: f32,
    integer: bool,
    help: &str,
) {
    knob(
        ui,
        state,
        &format!("material.texture.planar.{field}"),
        label,
        lo,
        hi,
        false,
        integer,
        help,
    );
}

pub fn family_start(family: PlanarFamily) -> PlanarFractal {
    use PlanarFamily::*;
    let mut p = PlanarFractal {
        family,
        iterations: 192,
        ..PlanarFractal::default()
    };
    match family {
        Julia => {
            p.julia = [-0.8, 0.156];
            p.center = [0.08, 0.02];
            p.zoom = 5.0;
            p.angle = 23.0;
            p.stripes = 8.0;
        }
        Mandelbrot => {
            p.center = [-0.7435, 0.1314];
            p.zoom = 240.0;
            p.iterations = 256;
            p.stripes = 17.0;
        }
        BurningShip => {
            p.center = [-1.76, -0.035];
            p.zoom = 120.0;
            p.stripes = 11.0;
        }
        Newton => {
            p.power = 3;
            p.zoom = 1.4;
            p.stripes = 8.0;
            p.iterations = 96;
        }
        Nova => {
            p.power = 3;
            p.julia = [-0.16, 0.18];
            p.zoom = 2.4;
            p.relaxation = 0.95;
            p.twist = 6.0;
            p.stripes = 8.0;
        }
        Phoenix => {
            p.julia = [0.56667, 0.0];
            p.zoom = 2.8;
            p.stripes = 13.0;
        }
    }
    p
}

fn flame_controls(ui: &mut Ui, state: &mut AppState, colours: bool) {
    if colours {
        let mut ramp = state.scene.canvas.flame.as_ref().unwrap().palette.clone();
        if widgets::ramp_editor(ui, &mut ramp) {
            state.edit(|s| s.canvas.flame.as_mut().unwrap().palette = ramp);
        }
        knob(
            ui,
            state,
            "canvas.flame.brightness",
            syfrack_i18n::tr!("Incandescence"),
            0.1,
            6.0,
            true,
            false,
            syfrack_i18n::tr!("Puissance de la lumière issue de la densité d'orbites."),
        );
        knob(
            ui,
            state,
            "canvas.flame.density_log",
            syfrack_i18n::tr!("Souplesse des densités"),
            0.0,
            32.0,
            false,
            false,
            syfrack_i18n::tr!("Compression continue de la densité, avant le développement HDR."),
        );
    } else {
        ui.label(syfrack_i18n::tr!(
            "Les transformations composent les orbites ; le cadre reste indépendant."
        ));
        let n = state.scene.canvas.flame.as_ref().unwrap().transforms.len();
        for i in 0..n {
            egui::CollapsingHeader::new(syfrack_i18n::tr_format!("Courant {}", i + 1)).show(
                ui,
                |ui| {
                    for (name, label, lo, hi) in [
                        ("weight", syfrack_i18n::tr!("Présence"), 0.01, 4.0),
                        ("color", syfrack_i18n::tr!("Couleur de l'orbite"), 0.0, 1.0),
                        (
                            "color_speed",
                            syfrack_i18n::tr!("Mélange des orbites"),
                            0.0,
                            1.0,
                        ),
                    ] {
                        knob(
                            ui,
                            state,
                            &format!("canvas.flame.transforms[{i}].{name}"),
                            label,
                            lo,
                            hi,
                            false,
                            false,
                            syfrack_i18n::tr!("Paramètre de la transformation native."),
                        );
                    }
                    for j in 0..6 {
                        knob(
                            ui,
                            state,
                            &format!("canvas.flame.transforms[{i}].affine[{j}]"),
                            [
                                syfrack_i18n::tr!("Échelle X"),
                                syfrack_i18n::tr!("Inclinaison X"),
                                "Position X",
                                syfrack_i18n::tr!("Inclinaison Y"),
                                syfrack_i18n::tr!("Échelle Y"),
                                "Position Y",
                            ][j],
                            -2.0,
                            2.0,
                            false,
                            false,
                            syfrack_i18n::tr!(
                                "Déforme les courants avant les variations non linéaires."
                            ),
                        );
                    }
                },
            );
        }
        knob(
            ui,
            state,
            "canvas.flame.filter_radius",
            syfrack_i18n::tr!("Continuité des filaments"),
            0.4,
            2.0,
            false,
            false,
            syfrack_i18n::tr!("Reconstruction orientée par la densité, avec poids communs RGB."),
        );
        knob(
            ui,
            state,
            "canvas.flame.points_per_sample",
            syfrack_i18n::tr!("Densité de calcul"),
            1.0,
            16.0,
            false,
            true,
            syfrack_i18n::tr!(
                "Points d'orbite par pixel et échantillon ; davantage réduit le bruit."
            ),
        );
    }
}

fn motif_2d(ui: &mut Ui, state: &mut AppState) {
    let mut flame = state.scene.canvas.flame.is_some();
    if ui
        .checkbox(
            &mut flame,
            syfrack_i18n::tr!("Flammes fractales · orbites et filaments"),
        )
        .changed()
    {
        state.edit(|s| {
            s.canvas.flame = if flame {
                Some(syfrack_core::Flame::default())
            } else {
                None
            };
            if !flame
                && s.material
                    .texture
                    .as_ref()
                    .and_then(|t| t.planar.as_ref())
                    .is_none()
            {
                s.material.texture = Scene::example_2d().material.texture;
            }
        });
    }
    if flame {
        flame_controls(ui, state, false);
        return;
    }
    let Some(p) = state
        .scene
        .material
        .texture
        .as_ref()
        .and_then(|t| t.planar.as_ref())
    else {
        return;
    };
    let current = p.family;
    ui.label(RichText::new(syfrack_i18n::tr!("Choisir une famille")).strong());
    let mut chosen = current;
    ui.horizontal_wrapped(|ui| {
        for (family, name) in [
            (PlanarFamily::Julia, "Julia"),
            (PlanarFamily::Mandelbrot, "Mandelbrot"),
            (PlanarFamily::BurningShip, "Burning Ship"),
            (PlanarFamily::Newton, "Newton"),
            (PlanarFamily::Nova, "Nova"),
            (PlanarFamily::Phoenix, "Phoenix"),
        ] {
            if widgets::choice_button(ui, current == family, name).clicked() {
                chosen = family;
            }
        }
    });
    if chosen != current {
        state.edit(|s| {
            s.material.texture.as_mut().unwrap().planar = Some(family_start(chosen));
            s.canvas = Default::default();
            let root = matches!(chosen, PlanarFamily::Newton | PlanarFamily::Nova);
            s.material.texture.as_mut().unwrap().channel = TextureChannel::Orbit {
                channels: OrbitColoring {
                    traps: if root {
                        [0.2, 0.4, 0.4, 0.0]
                    } else {
                        [1.0, 0.08, 0.18, 0.0]
                    },
                    ..OrbitColoring::default()
                },
            };
        });
    }
    ui.add_space(12.0);
    ui.label(RichText::new(syfrack_i18n::tr!("Dessiner les courbes")).strong());
    if matches!(
        chosen,
        PlanarFamily::Julia | PlanarFamily::Nova | PlanarFamily::Phoenix
    ) {
        planar_knob(
            ui,
            state,
            "julia[0]",
            syfrack_i18n::tr!("Courbes · X"),
            -1.3,
            1.3,
            false,
            syfrack_i18n::tr!(
                "Change la constante réelle : les branches et les spirales se recomposent."
            ),
        );
        planar_knob(
            ui,
            state,
            "julia[1]",
            syfrack_i18n::tr!("Courbes · Y"),
            -1.0,
            1.0,
            false,
            syfrack_i18n::tr!("Change la constante imaginaire : explore des motifs voisins."),
        );
    }
    planar_knob(
        ui,
        state,
        "power",
        syfrack_i18n::tr!("Ramifications"),
        2.0,
        8.0,
        true,
        syfrack_i18n::tr!("Change le degré de la formule et le nombre de branches."),
    );
    planar_knob(
        ui,
        state,
        "twist",
        syfrack_i18n::tr!("Torsion des branches"),
        -30.0,
        30.0,
        false,
        syfrack_i18n::tr!("Tourne chaque étape de la formule ; déforme les spirales."),
    );
    planar_knob(
        ui,
        state,
        "inversion",
        syfrack_i18n::tr!("Courbure du motif"),
        0.0,
        2.0,
        false,
        syfrack_i18n::tr!(
            "Inverse le domaine autour de son centre ; zéro conserve le motif d'origine."
        ),
    );
    if matches!(chosen, PlanarFamily::Newton | PlanarFamily::Nova) {
        planar_knob(
            ui,
            state,
            "relaxation",
            syfrack_i18n::tr!("Tension des branches"),
            0.5,
            1.3,
            false,
            syfrack_i18n::tr!(
                "Modifie l'amplitude du pas de Newton, donc les bassins et leurs frontières."
            ),
        );
    }
    if chosen == PlanarFamily::Phoenix {
        planar_knob(
            ui,
            state,
            "memory",
            syfrack_i18n::tr!("Mémoire du motif"),
            -0.8,
            0.8,
            false,
            syfrack_i18n::tr!("Mélange l'étape précédente à la suivante : modifie les filaments."),
        );
    }
    let bands = state
        .scene
        .material
        .texture
        .as_ref()
        .is_some_and(|t| match t.channel {
            TextureChannel::Orbit { channels } => channels.traps[2] != 0.0,
            TextureChannel::Trap { slot } => slot == 2,
            _ => false,
        });
    if bands {
        planar_knob(
            ui,
            state,
            "stripes",
            syfrack_i18n::tr!("Nervures"),
            0.0,
            48.0,
            true,
            syfrack_i18n::tr!("Trace des bandes sinusoïdales le long des angles de l'orbite."),
        );
    }
    planar_knob(ui,state,"iterations",syfrack_i18n::tr!("Détail et rythme"),32.0,384.0,true,syfrack_i18n::tr!("Augmente le détail des frontières et modifie le rythme de la coloration récursive ; demande plus de calcul."));
    if ui
        .button(syfrack_i18n::tr!("Réinitialiser le motif"))
        .clicked()
    {
        state.edit(|s| {
            s.material.texture.as_mut().unwrap().planar = Some(family_start(chosen));
            s.canvas = Default::default();
        });
    }
    egui::CollapsingHeader::new(syfrack_i18n::tr!("Explorer une autre région")).show(ui, |ui| {
        planar_knob(
            ui,
            state,
            "center[0]",
            syfrack_i18n::tr!("Région · X"),
            -2.0,
            2.0,
            false,
            syfrack_i18n::tr!("Déplace la région explorée dans le plan complexe."),
        );
        planar_knob(
            ui,
            state,
            "center[1]",
            syfrack_i18n::tr!("Région · Y"),
            -2.0,
            2.0,
            false,
            syfrack_i18n::tr!("Déplace la région explorée dans le plan complexe."),
        );
        knob(
            ui,
            state,
            "material.texture.planar.zoom",
            syfrack_i18n::tr!("Échelle du motif"),
            0.1,
            1000.0,
            true,
            false,
            syfrack_i18n::tr!(
                "Choisit l'étendue mathématique du motif, avant le cadrage de l'image."
            ),
        );
    });
}

pub fn set_palette(state: &mut AppState, index: usize) {
    let mut ramp = syfrack_core::scene::palette::neon_palette(index);
    let existing = if state.scene.dimension == Dimension::TwoD {
        state.scene.material.texture.as_ref().map(|t| &t.ramp)
    } else {
        state.scene.material.coloring.ramp()
    };
    if let Some(r) = existing {
        ramp.frequency = r.frequency;
        ramp.phase = r.phase;
        ramp.gamma = r.gamma;
        ramp.interpolation = r.interpolation;
    }
    state.edit(|s| {
        if s.dimension == Dimension::TwoD {
            s.material.texture.as_mut().unwrap().ramp = ramp;
        } else {
            if let Some(existing) = s.material.coloring.ramp_mut() {
                *existing = ramp;
            } else {
                s.material.coloring = Coloring::Trap { slot: 5, ramp };
            }
            s.material.albedo = [1.0; 3];
        }
    });
}

fn palette(ui: &mut Ui, state: &mut AppState) {
    ui.label(RichText::new(syfrack_i18n::tr!("Palettes néon")).strong());
    let current_stops = if state.scene.dimension == Dimension::TwoD {
        state
            .scene
            .material
            .texture
            .as_ref()
            .map(|t| t.ramp.stops.clone())
    } else {
        state
            .scene
            .material
            .coloring
            .ramp()
            .map(|r| r.stops.clone())
    };
    ui.horizontal_wrapped(|ui| {
        for (i, name) in syfrack_core::scene::palette::NEON_NAMES.iter().enumerate() {
            let active = current_stops
                .as_ref()
                .is_some_and(|stops| *stops == syfrack_core::scene::palette::neon_palette(i).stops);
            let r = widgets::choice_button(ui, active, syfrack_i18n::text(name)).on_hover_text(
                syfrack_i18n::tr!(
                    "Changer l'harmonie tout en conservant son rythme et son défilement"
                ),
            );
            #[cfg(test)]
            ui.data_mut(|d| d.insert_temp(egui::Id::new(("palette-preset", i)), r.rect));
            if r.clicked() && !active {
                set_palette(state, i);
            }
        }
    });
    let is_2d = state.scene.dimension == Dimension::TwoD;
    let prefix = if is_2d {
        "material.texture.ramp".to_string()
    } else {
        match &state.scene.material.coloring {
            Coloring::Trap { .. } => "material.coloring.trap.ramp".to_string(),
            Coloring::Orbit { .. } => "material.coloring.orbit.ramp".to_string(),
            _ => String::new(),
        }
    };
    let ramp: Option<Ramp> = if is_2d {
        state
            .scene
            .material
            .texture
            .as_ref()
            .map(|t| t.ramp.clone())
    } else {
        state.scene.material.coloring.ramp().cloned()
    };
    if let Some(mut ramp) = ramp {
        // The palette itself remains visible while browsing presets.
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 20.0), egui::Sense::hover());
        for i in 0..128 {
            let c = ramp.sample(i as f32 / 128.0 / ramp.frequency - ramp.phase / ramp.frequency);
            let mut swatch = syfrack_core::HdrImage::new(1, 1);
            swatch.set(0, 0, [c[0], c[1], c[2], 1.0]);
            let displayed = syfrack_color::develop(&swatch, &state.scene.look);
            let pixel = displayed.get(0, 0);
            let c = [pixel[0], pixel[1], pixel[2]];
            let rgb = c.map(|v| {
                if v <= 0.0031308 {
                    12.92 * v
                } else {
                    1.055 * v.powf(1.0 / 2.4) - 0.055
                }
            });
            ui.painter().rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(rect.left() + i as f32 * rect.width() / 128.0, rect.top()),
                    egui::pos2(
                        rect.left() + (i + 1) as f32 * rect.width() / 128.0 + 0.5,
                        rect.bottom(),
                    ),
                ),
                0.0,
                egui::Color32::from_rgb(
                    (rgb[0] * 255.0) as u8,
                    (rgb[1] * 255.0) as u8,
                    (rgb[2] * 255.0) as u8,
                ),
            );
        }
        if !prefix.is_empty() {
            knob(
                ui,
                state,
                &format!("{prefix}.frequency"),
                syfrack_i18n::tr!("Finesse des couleurs"),
                1.0,
                34.0,
                false,
                false,
                syfrack_i18n::tr!(
                    "Répète la palette : davantage de cycles crée des liserés plus fins."
                ),
            );
            knob(
                ui,
                state,
                &format!("{prefix}.phase"),
                syfrack_i18n::tr!("Faire défiler"),
                -1.0,
                1.0,
                false,
                false,
                syfrack_i18n::tr!("Décale toutes les couleurs le long du motif."),
            );
            knob(
                ui,
                state,
                &format!("{prefix}.gamma"),
                syfrack_i18n::tr!("Répartition des couleurs"),
                0.25,
                4.0,
                false,
                false,
                syfrack_i18n::tr!("Donne plus de place au début ou à la fin de chaque cycle."),
            );
        }
        egui::CollapsingHeader::new(syfrack_i18n::tr!("Personnaliser la palette")).show(ui, |ui| {
            if widgets::ramp_editor(ui, &mut ramp) && ramp.validate("palette").is_ok() {
                state.edit(|s| {
                    if is_2d {
                        s.material.texture.as_mut().unwrap().ramp = ramp;
                    } else if let Some(r) = s.material.coloring.ramp_mut() {
                        *r = ramp;
                    }
                });
            }
        });
    }
}

fn colours_2d(ui: &mut Ui, state: &mut AppState) {
    palette(ui, state);
    neon_softness(ui, state);
    ui.add_space(12.0);
    ui.label(RichText::new(syfrack_i18n::tr!("Dessiner la couleur")).strong());
    ui.horizontal_wrapped(|ui| {
        for (name, weights) in [
            (syfrack_i18n::tr!("Soie"), [1.0, 0.08, 0.18, 0.0]),
            (syfrack_i18n::tr!("Filigrane"), [0.6, 0.24, 0.3, 0.0]),
            (syfrack_i18n::tr!("Anneaux"), [0.1, 1.0, 0.1, 0.0]),
            (syfrack_i18n::tr!("Rubans"), [0.15, 0.12, 0.8, 0.0]),
        ] {
            if ui.button(name).clicked() {
                state.edit(|s| {
                    s.material.texture.as_mut().unwrap().channel = TextureChannel::Orbit {
                        channels: OrbitColoring {
                            traps: weights,
                            ..OrbitColoring::default()
                        },
                    }
                });
            }
        }
    });
    knob(
        ui,
        state,
        "material.texture.planar.trap_radius",
        syfrack_i18n::tr!("Rayon des anneaux"),
        0.0,
        2.0,
        false,
        false,
        syfrack_i18n::tr!("Modifie le cercle observé par l'orbite ; agit sur les recettes qui mélangent les anneaux."),
    );
    knob(
        ui,
        state,
        "look.exposure_ev",
        syfrack_i18n::tr!("Luminosité"),
        -1.5,
        1.5,
        false,
        false,
        syfrack_i18n::tr!("Éclaircit ou assombrit les couleurs."),
    );
    knob(
        ui,
        state,
        "look.saturation",
        syfrack_i18n::tr!("Éclat des couleurs"),
        0.0,
        2.0,
        false,
        false,
        syfrack_i18n::tr!("Renforce les couleurs ou les rapproche du gris."),
    );
    if ui
        .button(syfrack_i18n::tr!("Retrouver les couleurs fidèles"))
        .clicked()
    {
        state.edit(|s| {
            s.look = syfrack_core::Look {
                tonemap: syfrack_core::Tonemap::Direct,
                neon_softness: s.look.neon_softness,
                ..syfrack_core::Look::neutral()
            }
        });
    }
}

pub(super) fn framing_2d(ui: &mut Ui, state: &mut AppState) {
    ui.label(syfrack_i18n::tr!(
        "Glisser l'image pour la déplacer. Molette : zoomer autour du pointeur."
    ));
    knob(
        ui,
        state,
        "canvas.zoom",
        "Zoom",
        0.25,
        64.0,
        true,
        false,
        syfrack_i18n::tr!("Agrandit l'image sans changer sa formule."),
    );
    knob(
        ui,
        state,
        "canvas.rotation",
        "Rotation",
        -180.0,
        180.0,
        false,
        false,
        syfrack_i18n::tr!("Tourne l'ensemble de la composition."),
    );
    knob(
        ui,
        state,
        "canvas.center[0]",
        syfrack_i18n::tr!("Déplacement · X"),
        -3.0,
        3.0,
        false,
        false,
        syfrack_i18n::tr!("Décale le cadre horizontalement."),
    );
    knob(
        ui,
        state,
        "canvas.center[1]",
        syfrack_i18n::tr!("Déplacement · Y"),
        -3.0,
        3.0,
        false,
        false,
        syfrack_i18n::tr!("Décale le cadre verticalement."),
    );
    if ui.button(syfrack_i18n::tr!("Recentrer le cadre")).clicked() {
        state.edit(|s| {
            s.canvas.center = [0.0; 2];
            s.canvas.zoom = 1.0;
            s.canvas.rotation = 0.0;
        });
    }
}

pub(super) fn volume(ui: &mut Ui, state: &mut AppState) {
    ui.label(RichText::new(syfrack_i18n::tr!("Choisir un volume")).strong());
    ui.horizontal_wrapped(|ui| {
        for (key, label) in [
            ("mandelbulb", syfrack_i18n::tr!("Corail")),
            ("mandelbox", "Architecture"),
            ("menger", syfrack_i18n::tr!("Éponge")),
            ("sphere", syfrack_i18n::tr!("Sphère")),
        ] {
            if ui.button(label).clicked() {
                let next = Scene::example_for(key).expect("bundled form");
                state.edit(|s| {
                    s.fractal = next.fractal;
                    s.camera = next.camera;
                });
            }
        }
    });
    ui.add_space(12.0);
    ui.label(RichText::new(syfrack_i18n::tr!("Façonner le volume")).strong());
    let graph = state.scene.fractal.to_graph();
    let mut active: std::collections::HashSet<_> = graph.active_nodes().into_iter().collect();
    let mut todo: Vec<_> = active.iter().copied().collect();
    while let Some(id) = todo.pop() {
        if let Some(n) = graph.node(id) {
            if n.enabled {
                if let NodeKind::Iterate { body, .. } = &n.kind {
                    for id in body {
                        if active.insert(*id) {
                            todo.push(*id);
                        }
                    }
                }
            }
        }
    }
    let mut shown = std::collections::HashSet::new();
    for node in graph
        .nodes
        .iter()
        .filter(|n| n.enabled && active.contains(&n.id))
    {
        let settings: &[(&str, &str, f32, f32, bool, &str)] = match node.kind {
            NodeKind::Iterate { .. } => &[(
                "iterations",
                syfrack_i18n::tr!("Détail récursif"),
                2.0,
                20.0,
                true,
                syfrack_i18n::tr!(
                    "Ajoute des générations au volume ; le calcul devient plus long."
                ),
            )],
            NodeKind::Power { .. } => &[(
                "power",
                syfrack_i18n::tr!("Ramifications"),
                2.0,
                16.0,
                false,
                syfrack_i18n::tr!("Change le nombre et la forme des branches du volume."),
            )],
            NodeKind::BoxFold { .. } => &[(
                "limit",
                syfrack_i18n::tr!("Ouverture des plis"),
                0.25,
                2.0,
                false,
                syfrack_i18n::tr!("Déplace les plans de pliage de la structure."),
            )],
            NodeKind::SphereFold { .. } => &[(
                "min_radius",
                syfrack_i18n::tr!("Courbure des cavités"),
                0.1,
                0.95,
                false,
                syfrack_i18n::tr!("Change la région intérieure de l'inversion sphérique."),
            )],
            NodeKind::MengerFold { .. } => &[(
                "scale",
                syfrack_i18n::tr!("Densité de la structure"),
                2.0,
                5.0,
                false,
                syfrack_i18n::tr!("Change l'échelle répétée des cavités."),
            )],
            NodeKind::Twist { .. } => &[(
                "rate",
                syfrack_i18n::tr!("Torsion"),
                -3.0,
                3.0,
                false,
                syfrack_i18n::tr!("Tord l'espace autour du volume."),
            )],
            NodeKind::Bend { .. } => &[(
                "rate",
                syfrack_i18n::tr!("Courbure"),
                -3.0,
                3.0,
                false,
                syfrack_i18n::tr!("Courbe l'espace autour du volume."),
            )],
            NodeKind::Kaleido { .. } => &[(
                "planes",
                syfrack_i18n::tr!("Symétrie"),
                2.0,
                16.0,
                true,
                syfrack_i18n::tr!("Change le nombre de plans de symétrie."),
            )],
            NodeKind::Sphere { .. } => &[(
                "radius",
                syfrack_i18n::tr!("Taille du volume"),
                0.1,
                3.0,
                false,
                syfrack_i18n::tr!("Agrandit la sphère."),
            )],
            _ => &[],
        };
        for &(field, label, lo, hi, int, help) in settings {
            if shown.insert(label) && shown.len() <= 7 {
                knob(
                    ui,
                    state,
                    &format!("fractal.nodes[{}].{field}", node.id),
                    label,
                    lo,
                    hi,
                    false,
                    int,
                    help,
                );
            }
        }
    }
    let edit = ui.button(syfrack_i18n::tr!("Construire une forme personnalisée"));
    #[cfg(test)]
    ui.data_mut(|d| d.insert_temp(egui::Id::new("edit-form-button"), edit.rect));
    if edit.clicked() {
        state.tab = Tab::Forme;
    }
    egui::CollapsingHeader::new(syfrack_i18n::tr!("Cadrer le volume"))
        .default_open(true)
        .show(ui, |ui| {
            knob(
                ui,
                state,
                "camera.orbit.distance",
                "Distance",
                0.5,
                40.0,
                true,
                false,
                syfrack_i18n::tr!("Rapproche ou éloigne le regard du volume."),
            );
            knob(
                ui,
                state,
                "camera.orbit.azimuth",
                syfrack_i18n::tr!("Tourner autour"),
                -180.0,
                180.0,
                false,
                false,
                syfrack_i18n::tr!("Tourne autour du volume."),
            );
            knob(
                ui,
                state,
                "camera.orbit.elevation",
                syfrack_i18n::tr!("Hauteur du regard"),
                -80.0,
                80.0,
                false,
                false,
                syfrack_i18n::tr!("Observe le volume par-dessus ou par-dessous."),
            );
            knob(
                ui,
                state,
                "camera.focal_mm",
                "Perspective",
                18.0,
                85.0,
                false,
                false,
                syfrack_i18n::tr!(
                    "Une focale courte accentue la profondeur ; une longue focale l'aplatit."
                ),
            );
        });
    egui::CollapsingHeader::new(syfrack_i18n::tr!("Profondeur de champ et caméra libre"))
        .show(ui, |ui| panels::camera::show(ui, state));
}

fn lighting(ui: &mut Ui, state: &mut AppState) {
    ui.horizontal_wrapped(|ui| {
        for (name, i) in [
            (syfrack_i18n::tr!("Studio doux"), 0),
            (syfrack_i18n::tr!("Dramatique"), 1),
            (syfrack_i18n::tr!("Néons"), 2),
        ] {
            if ui.button(name).clicked() {
                state.edit(|s| {
                    s.lights = match i {
                        1 => vec![
                            Light::Sun {
                                direction: glam::Vec3::new(-0.8, 0.5, 0.2).normalize(),
                                color: Color::Linear([1.0, 0.78, 0.5]),
                                intensity: 5.0,
                                angular_size: 0.3,
                            },
                            Light::Environment {
                                zenith: Color::Linear([0.12, 0.2, 0.5]),
                                horizon: Color::Linear([0.04, 0.05, 0.1]),
                                ground: Color::Linear([0.01; 3]),
                                intensity: 0.15,
                                rotation: 0.0,
                            },
                        ],
                        2 => vec![
                            Light::Area {
                                position: glam::Vec3::new(-3.0, 2.0, 3.0),
                                normal: glam::Vec3::new(3.0, -2.0, -3.0).normalize(),
                                size: [3.0; 2],
                                color: Color::Linear([1.0, 0.01, 0.4]),
                                intensity: 8.0,
                            },
                            Light::Area {
                                position: glam::Vec3::new(3.0, 1.0, -1.0),
                                normal: glam::Vec3::new(-3.0, -1.0, 1.0).normalize(),
                                size: [2.0; 2],
                                color: Color::Linear([0.01, 0.6, 1.0]),
                                intensity: 8.0,
                            },
                            Light::Environment {
                                zenith: Color::Linear([0.04, 0.02, 0.1]),
                                horizon: Color::Linear([0.02; 3]),
                                ground: Color::Linear([0.01; 3]),
                                intensity: 0.2,
                                rotation: 0.0,
                            },
                        ],
                        _ => vec![
                            Light::Area {
                                position: glam::Vec3::new(-3.0, 4.0, 4.0),
                                normal: glam::Vec3::new(3.0, -4.0, -4.0).normalize(),
                                size: [5.0; 2],
                                color: Color::Linear([1.0, 0.9, 0.8]),
                                intensity: 6.0,
                            },
                            Light::Environment {
                                zenith: Color::Linear([0.2, 0.4, 0.8]),
                                horizon: Color::Linear([0.4, 0.5, 0.7]),
                                ground: Color::Linear([0.1; 3]),
                                intensity: 0.6,
                                rotation: 0.0,
                            },
                        ],
                    }
                });
            }
        }
    });
    let lights = state.scene.lights.clone();
    for (i, light) in lights.iter().enumerate() {
        ui.add_space(10.0);
        ui.label(RichText::new(format!("Source {}", i + 1)).strong());
        let path = format!("lights[{i}].{}", panels::light_kind(light));
        knob(
            ui,
            state,
            &format!("{path}.intensity"),
            syfrack_i18n::tr!("Intensité"),
            if matches!(light, Light::Environment { .. } | Light::Hdri { .. }) {
                0.0
            } else {
                0.001
            },
            if matches!(light, Light::Environment { .. } | Light::Hdri { .. }) {
                5.0
            } else {
                3000.0
            },
            !matches!(light, Light::Environment { .. } | Light::Hdri { .. }),
            false,
            syfrack_i18n::tr!("Renforce l'éclairage de cette source."),
        );
        if matches!(light, Light::Sun { .. }) {
            knob(
                ui,
                state,
                &format!("{path}.angular_size"),
                syfrack_i18n::tr!("Douceur des ombres"),
                0.1,
                15.0,
                false,
                false,
                syfrack_i18n::tr!("Une source plus large produit des ombres plus douces."),
            );
        }
    }
    knob(
        ui,
        state,
        "atmosphere.fog_density",
        syfrack_i18n::tr!("Brume"),
        0.0,
        0.12,
        false,
        false,
        syfrack_i18n::tr!("Ajoute de la profondeur atmosphérique."),
    );
    egui::CollapsingHeader::new(syfrack_i18n::tr!("Placer et colorer les sources"))
        .show(ui, |ui| panels::lights::show(ui, state));
}

pub fn show(ui: &mut Ui, state: &mut AppState) {
    if matches!(state.inspector, Inspector::Scene | Inspector::Texture) {
        ui.heading(syfrack_i18n::tr!("Graphe de création"));
        ui.label(syfrack_i18n::tr!(
            "Motifs, flammes et champs naturels se combinent dans le même graphe."
        ));
        if ui.button(syfrack_i18n::tr!("Ouvrir le graphe")).clicked() {
            state.tab = crate::state::Tab::Forme;
        }
        if state.inspector == Inspector::Scene {
            if state.scene.dimension == Dimension::TwoD {
                framing_2d(ui, state);
            } else {
                panels::camera::show(ui, state);
            }
        }
        return;
    }
    if state.scene.dimension == Dimension::TwoD && state.inspector == Inspector::Material {
        ui.label(syfrack_i18n::tr!(
            "Les palettes et les mélanges de couleur se règlent sur les nœuds du graphe."
        ));
        if ui.button(syfrack_i18n::tr!("Ouvrir le graphe")).clicked() {
            state.tab = crate::state::Tab::Forme;
        }
        return;
    }
    if state.scene.dimension == Dimension::TwoD {
        match state.inspector {
            Inspector::Scene => motif_2d(ui, state),
            Inspector::Material => {
                if state.scene.canvas.flame.is_some() {
                    flame_controls(ui, state, true);
                } else {
                    colours_2d(ui, state);
                }
            }
            _ => framing_2d(ui, state),
        }
        return;
    }
    match state.inspector {
        Inspector::Scene => volume(ui, state),
        Inspector::Material => {
            if state.scene.creation.is_none() {
                palette(ui, state);
            }
            let mut colour = state.scene.material.albedo;
            if widgets::linear_color(
                ui,
                syfrack_i18n::tr!("Couleur de base"),
                &mut colour,
                syfrack_core::Bounds::new(0.0, 1.0, &[(0.0, 1.0)]),
                &state.scene.look,
            ) {
                state.edit(|s| s.material.albedo = colour);
            }
            knob(ui,state,"material.roughness",syfrack_i18n::tr!("Mat / brillant"),0.05,1.0,false,false,syfrack_i18n::tr!("Une faible rugosité donne des reflets nets ; une forte rugosité donne une surface mate."));
            knob(
                ui,
                state,
                "material.metallic",
                syfrack_i18n::tr!("Métallique"),
                0.0,
                1.0,
                false,
                false,
                syfrack_i18n::tr!("Teinte les reflets avec la couleur de la surface."),
            );
            if state.scene.creation.is_none() {
                egui::CollapsingHeader::new(syfrack_i18n::tr!("Motif de surface"))
                    .show(ui, |ui| texture_3d(ui, state));
            } else if ui
                .button(syfrack_i18n::tr!("Motifs et couleurs dans le graphe"))
                .clicked()
            {
                state.tab = crate::state::Tab::Forme;
            }
            if !state.scene.material.finishes.is_empty() {
                egui::CollapsingHeader::new(syfrack_i18n::tr!("Surfaces de la scène"))
                    .show(ui, |ui| panels::material::show_finishes(ui, state));
            }
        }
        Inspector::Lights => lighting(ui, state),
        Inspector::Texture => panels::material::show_texture(ui, state),
        Inspector::Atmosphere => {
            knob(
                ui,
                state,
                "atmosphere.fog_density",
                syfrack_i18n::tr!("Brume"),
                0.0,
                0.12,
                false,
                false,
                syfrack_i18n::tr!("Ajoute de la profondeur atmosphérique."),
            );
            egui::CollapsingHeader::new(syfrack_i18n::tr!("Mer, ciel et atmosphère"))
                .show(ui, |ui| panels::atmosphere::show(ui, state));
        }
        Inspector::Look => {
            neon_softness(ui, state);
            knob(
                ui,
                state,
                "look.exposure_ev",
                syfrack_i18n::tr!("Luminosité"),
                -2.0,
                2.0,
                false,
                false,
                syfrack_i18n::tr!("Éclaircit ou assombrit l'image."),
            );
            knob(
                ui,
                state,
                "look.tonemap.agx.power",
                syfrack_i18n::tr!("Contraste"),
                0.7,
                1.6,
                false,
                false,
                syfrack_i18n::tr!("Sépare davantage les zones claires et sombres."),
            );
            knob(
                ui,
                state,
                "look.saturation",
                syfrack_i18n::tr!("Éclat des couleurs"),
                0.0,
                2.0,
                false,
                false,
                syfrack_i18n::tr!("Renforce les couleurs ou les rapproche du gris."),
            );
            knob(
                ui,
                state,
                "look.vibrance",
                syfrack_i18n::tr!("Couleurs discrètes"),
                0.0,
                1.0,
                false,
                false,
                syfrack_i18n::tr!("Renforce surtout les couleurs les moins saturées."),
            );
            egui::CollapsingHeader::new(syfrack_i18n::tr!("Développement de l'image"))
                .show(ui, |ui| panels::look::show(ui, state));
        }
    }
    ui.add_space(8.0);
    ui.label(
        RichText::new(syfrack_i18n::tr!(
            "Glisser : tourner · molette : zoomer · bouton milieu : déplacer"
        ))
        .small()
        .color(theme::TEXT_DIM),
    );
}

pub(super) fn neon_softness(ui: &mut Ui, state: &mut AppState) {
    let mut percent = state.scene.look.neon_softness * 100.0;
    let r=ui.add(egui::Slider::new(&mut percent,0.0..=100.0).integer().suffix(" %").text(syfrack_i18n::tr!("Douceur du néon")))
        .on_hover_text(syfrack_i18n::tr!("0 % : couleurs électriques. 100 % : néons plus doux, pics lumineux atténués. Les noirs et les détails restent nets."));
    #[cfg(test)]
    ui.data_mut(|d| d.insert_temp(egui::Id::new("neon-softness"), r.rect));
    if r.changed() {
        state.set_param("look.neon_softness", percent / 100.0);
    }
}

fn texture_3d(ui: &mut Ui, state: &mut AppState) {
    let mut on = state.scene.material.texture.is_some();
    if ui
        .checkbox(&mut on, syfrack_i18n::tr!("Activer le motif"))
        .changed()
    {
        state.edit(|s| s.material.texture = on.then(syfrack_core::FractalTexture::default));
    }
    if !on {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        for (key, label) in [
            ("mandelbox", syfrack_i18n::tr!("Veines")),
            ("menger", syfrack_i18n::tr!("Gravure")),
            ("mandelbulb", syfrack_i18n::tr!("Corail")),
        ] {
            if ui.button(label).clicked() {
                state.edit(|s| {
                    let t = s.material.texture.as_mut().unwrap();
                    t.graph = syfrack_core::Graph::bundled(key).unwrap();
                    t.planar = None;
                    t.channel = TextureChannel::Trap { slot: 0 };
                    t.ramp = syfrack_core::scene::palette::neon_palette(0);
                });
            }
        }
    });
    knob(
        ui,
        state,
        "material.texture.scale",
        syfrack_i18n::tr!("Densité du motif"),
        0.1,
        8.0,
        true,
        false,
        syfrack_i18n::tr!("Augmente le nombre de détails du motif sur la surface."),
    );
    knob(
        ui,
        state,
        "material.texture.strength",
        syfrack_i18n::tr!("Couleur du motif"),
        0.0,
        1.0,
        false,
        false,
        syfrack_i18n::tr!("Mélange les couleurs du motif à celles du volume."),
    );
    knob(
        ui,
        state,
        "material.texture.bump",
        syfrack_i18n::tr!("Relief fin"),
        0.0,
        0.08,
        false,
        false,
        syfrack_i18n::tr!("Ajoute du relief aux reflets sans changer la silhouette."),
    );
    knob(
        ui,
        state,
        "material.texture.roughness",
        syfrack_i18n::tr!("Variation mat / brillant"),
        0.0,
        1.0,
        false,
        false,
        syfrack_i18n::tr!("Fait varier la rugosité selon le motif."),
    );
}
