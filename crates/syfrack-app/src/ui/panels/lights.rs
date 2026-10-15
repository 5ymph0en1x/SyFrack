use egui::{Sense, Vec2};
use syfrack_core::{Color, Light, Scene, MAX_LIGHTS};

use super::{advanced, color_row, new_area, new_sun, param, toggle};
use crate::state::AppState;
use crate::ui::theme;
use crate::ui::widgets::{direction_angles, direction_compass, format_value, to_srgb32};

/// Height above the camera where « + Surfacique » places the new light.
const AREA_LIFT: f32 = 2.0;
/// Size of the sun direction compass.
const COMPASS: f32 = 7.0 * theme::GRID;
/// Temperature given to a colour switched back from RGB to Kelvin.
const KELVIN_FALLBACK: f32 = 5500.0;

fn name(light: &Light) -> &'static str {
    match light {
        Light::Sun { .. } => syfrack_i18n::tr!("Soleil"),
        Light::Area { .. } => syfrack_i18n::tr!("Surfacique"),
        Light::Projector { .. } => syfrack_i18n::tr!("Projecteur"),
        Light::Tube { .. } => syfrack_i18n::tr!("Tube néon"),
        Light::Environment { .. } => syfrack_i18n::tr!("Ciel"),
        Light::Hdri { .. } => "HDRI",
    }
}

/// The colour a light header shows: its own, brightest channel at 1.
fn swatch_color(light: &Light) -> [f32; 3] {
    let c = match light {
        Light::Sun { color, .. }
        | Light::Area { color, .. }
        | Light::Projector { color, .. }
        | Light::Tube { color, .. } => color.to_linear(),
        Light::Environment { zenith, .. } => zenith.to_linear(),
        Light::Hdri { .. } => [0.85, 0.9, 1.0],
    };
    let m = c[0].max(c[1]).max(c[2]).max(1e-6);
    c.map(|v| v / m)
}

/// Header titles: « Soleil », then « Soleil 2 » for a second sun.
fn titles(lights: &[Light]) -> Vec<String> {
    lights
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let same = lights[..i].iter().filter(|o| name(o) == name(l)).count();
            if same == 0 {
                name(l).to_string()
            } else {
                format!("{} {}", name(l), same + 1)
            }
        })
        .collect()
}

/// A Kelvin colour is a temperature; an RGB one is just a colour.
fn color_label(color: &Color) -> &'static str {
    match color {
        Color::Kelvin(_) => syfrack_i18n::tr!("Température"),
        Color::Linear(_) => syfrack_i18n::tr!("Couleur"),
    }
}

/// The colour of light `i`, written back through `edit`.
fn set_color(s: &mut Scene, i: usize, c: Color) {
    if let Light::Sun { color, .. }
    | Light::Area { color, .. }
    | Light::Projector { color, .. }
    | Light::Tube { color, .. } = &mut s.lights[i]
    {
        *color = c;
    }
}

/// « RVB » toggle: a Kelvin colour becomes its linear RGB, an RGB colour
/// becomes a neutral daylight temperature.
fn rgb_toggle(ui: &mut egui::Ui, state: &mut AppState, i: usize, color: &Color) {
    let is_rgb = matches!(color, Color::Linear(_));
    if let Some(on) = toggle(ui, syfrack_i18n::tr!("Couleur RVB"), is_rgb) {
        let c = if on {
            Color::Linear(color.to_linear())
        } else {
            Color::Kelvin(KELVIN_FALLBACK)
        };
        state.edit(|s| set_color(s, i, c));
    }
}

/// Body of one light. Returns true when the user asked to delete it.
fn body(ui: &mut egui::Ui, state: &mut AppState, i: usize, light: &Light, deletable: bool) -> bool {
    let p = format!("lights[{i}]");
    let mut delete = false;
    let mut delete_button = |ui: &mut egui::Ui| {
        if deletable && ui.small_button(syfrack_i18n::tr!("Supprimer")).clicked() {
            delete = true;
        }
    };
    match light {
        Light::Hdri { path, .. } => {
            ui.label(path.file_name().unwrap_or_default().to_string_lossy())
                .on_hover_text(path.display().to_string());
            if ui
                .button(syfrack_i18n::tr!("Choisir le panorama…"))
                .clicked()
            {
                crate::ui::file_browser::open_dialog(
                    ui,
                    state,
                    crate::ui::file_browser::FileAction::Hdri(Some(i)),
                );
            }
            param(
                &format!("{p}.hdri.intensity"),
                syfrack_i18n::tr!("Intensité"),
            )
            .log()
            .no_reference()
            .show(ui, state);
            param(&format!("{p}.hdri.rotation"), "Rotation")
                .unit("°")
                .no_reference()
                .show(ui, state);
            if deletable {
                advanced(ui, &p, delete_button);
            }
        }
        Light::Sun {
            direction, color, ..
        } => {
            ui.horizontal(|ui| {
                let mut d = *direction;
                if direction_compass(ui, &mut d, COMPASS) {
                    state.edit(|s| {
                        if let Light::Sun { direction, .. } = &mut s.lights[i] {
                            *direction = d;
                        }
                    });
                }
                let (az, el) = direction_angles(*direction);
                ui.vertical(|ui| {
                    ui.add_space(theme::GRID * 2.0);
                    for (label, v) in [("azimut", az), ("hauteur", el)] {
                        ui.label(
                            egui::RichText::new(format!("{label} {}°", format_value(v, 0)))
                                .monospace()
                                .color(theme::TEXT_DIM),
                        );
                    }
                });
            });
            param(
                &format!("{p}.sun.intensity"),
                syfrack_i18n::tr!("Intensité"),
            )
            .log()
            .show(ui, state);
            color_row(
                ui,
                state,
                &format!("{p}.sun.color"),
                color_label(color),
                color,
                |s, c| set_color(s, i, c),
            );
            param(
                &format!("{p}.sun.angular_size"),
                syfrack_i18n::tr!("Taille"),
            )
            .unit("°")
            .log()
            .show(ui, state);
            advanced(ui, &p, |ui| {
                rgb_toggle(ui, state, i, color);
                delete_button(ui);
            });
        }
        Light::Area { color, .. } | Light::Projector { color, .. } => {
            param(
                &format!("{p}.area.intensity"),
                syfrack_i18n::tr!("Intensité"),
            )
            .log()
            .show(ui, state);
            color_row(
                ui,
                state,
                &format!("{p}.area.color"),
                color_label(color),
                color,
                |s, c| set_color(s, i, c),
            );
            param(&format!("{p}.area.size.w"), syfrack_i18n::tr!("Largeur"))
                .unit("m")
                .log()
                .show(ui, state);
            param(&format!("{p}.area.size.h"), syfrack_i18n::tr!("Hauteur"))
                .unit("m")
                .log()
                .show(ui, state);
            if matches!(light, Light::Projector { .. }) {
                param(
                    &format!("{p}.area.inner_angle"),
                    syfrack_i18n::tr!("Demi-angle du cœur"),
                )
                .unit("°")
                .show(ui, state);
                param(
                    &format!("{p}.area.outer_angle"),
                    syfrack_i18n::tr!("Demi-angle du faisceau"),
                )
                .unit("°")
                .show(ui, state);
            }
            advanced(ui, &p, |ui| {
                for axis in ["x", "y", "z"] {
                    param(
                        &format!("{p}.area.position.{axis}"),
                        &format!("Position {axis}"),
                    )
                    .unit("m")
                    .no_reference()
                    .show(ui, state);
                }
                for axis in ["x", "y", "z"] {
                    param(
                        &format!("{p}.area.normal.{axis}"),
                        &syfrack_i18n::tr_format!("Normale {axis}", axis = axis),
                    )
                    .no_reference()
                    .show(ui, state);
                }
                rgb_toggle(ui, state, i, color);
                delete_button(ui);
            });
        }
        Light::Tube { color, .. } => {
            param(
                &format!("{p}.tube.intensity"),
                syfrack_i18n::tr!("Intensité"),
            )
            .log()
            .show(ui, state);
            color_row(
                ui,
                state,
                &format!("{p}.tube.color"),
                color_label(color),
                color,
                |s, c| set_color(s, i, c),
            );
            param(
                &format!("{p}.tube.radius"),
                syfrack_i18n::tr!("Rayon du tube"),
            )
            .unit("m")
            .log()
            .show(ui, state);
            advanced(ui, &p, |ui| {
                for end in ["start", "end"] {
                    for axis in ["x", "y", "z"] {
                        param(
                            &format!("{p}.tube.{end}.{axis}"),
                            &format!(
                                "{} {axis}",
                                if end == "start" {
                                    syfrack_i18n::tr!("Début")
                                } else {
                                    syfrack_i18n::tr!("Fin")
                                }
                            ),
                        )
                        .unit("m")
                        .no_reference()
                        .show(ui, state);
                    }
                }
                rgb_toggle(ui, state, i, color);
                delete_button(ui);
            });
        }
        Light::Environment {
            zenith,
            horizon,
            ground,
            ..
        } => {
            param(
                &format!("{p}.environment.intensity"),
                syfrack_i18n::tr!("Intensité"),
            )
            .log()
            .show(ui, state);
            let rows = [
                ("zenith", syfrack_i18n::tr!("Zénith"), zenith),
                ("horizon", "Horizon", horizon),
                ("ground", syfrack_i18n::tr!("Sol"), ground),
            ];
            for (field, label, color) in rows {
                let prefix = format!("{p}.environment.{field}");
                color_row(ui, state, &prefix, label, color, |s, c| {
                    if let Light::Environment {
                        zenith,
                        horizon,
                        ground,
                        ..
                    } = &mut s.lights[i]
                    {
                        match field {
                            "zenith" => *zenith = c,
                            "horizon" => *horizon = c,
                            _ => *ground = c,
                        }
                    }
                });
            }
            if deletable {
                advanced(ui, &p, delete_button);
            }
        }
    }
    delete
}

/// One fold per light, sun first as the scene lists it, each header carrying
/// a swatch of the light's colour; add buttons below.
pub fn show(ui: &mut egui::Ui, state: &mut AppState) {
    let lights = state.scene.lights.clone();
    let titles = titles(&lights);
    let deletable = lights.len() > 1;
    let mut remove = None;
    for (i, light) in lights.iter().enumerate() {
        let id = ui.make_persistent_id(("light", i));
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, i == 0)
            .show_header(ui, |ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(theme::GRID * 1.5), Sense::hover());
                ui.painter()
                    .rect_filled(r, 2.0, to_srgb32(swatch_color(light)));
                ui.label(&titles[i]);
            })
            .body(|ui| {
                if body(ui, state, i, light, deletable) {
                    remove = Some(i);
                }
            });
    }
    if let Some(i) = remove {
        state.edit(|s| {
            s.lights.remove(i);
        });
    }
    if state.scene.lights.len() < MAX_LIGHTS {
        ui.add_space(theme::GRID / 2.0);
        ui.horizontal(|ui| {
            if ui.small_button(syfrack_i18n::tr!("+ Soleil")).clicked() {
                state.edit(|s| s.lights.push(new_sun()));
            }
            if ui.small_button(syfrack_i18n::tr!("+ Surfacique")).clicked() {
                let b = state.scene.camera.basis();
                let light = new_area(b.origin + b.up * AREA_LIFT, b.forward);
                state.edit(|s| s.lights.push(light));
            }
            if ui.small_button(syfrack_i18n::tr!("+ Néon")).clicked() {
                let b = state.scene.camera.basis();
                let center = b.origin + b.forward * 3.0;
                state.edit(|s| {
                    s.lights.push(Light::Tube {
                        start: center - b.up,
                        end: center + b.up,
                        radius: 0.04,
                        color: Color::Linear([1.0, 0.025, 0.008]),
                        intensity: 20.0,
                    })
                });
            }
            if ui.small_button(syfrack_i18n::tr!("+ Projecteur")).clicked() {
                let b = state.scene.camera.basis();
                state.edit(|s| {
                    s.lights.push(Light::Projector {
                        position: b.origin + b.up * AREA_LIFT,
                        normal: b.forward,
                        size: [0.8, 1.2],
                        color: Color::Kelvin(5500.0),
                        intensity: 35.0,
                        inner_angle: 10.0,
                        outer_angle: 18.0,
                    })
                });
            }
            if ui.small_button("+ HDRI").clicked() {
                crate::ui::file_browser::open_dialog(
                    ui,
                    state,
                    crate::ui::file_browser::FileAction::Hdri(None),
                );
            }
        });
    }
}
