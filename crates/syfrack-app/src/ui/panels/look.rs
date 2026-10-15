use syfrack_core::Tonemap;

use super::{
    advanced, choice, new_grain, new_vignette, param, reinhard, rule, temperature, toggle,
    AUTO_EXPOSURE_TARGET,
};
use crate::state::AppState;
use crate::ui::theme;

/// Exposure, white balance, curve, colour, vignette, in the order a
/// photographer develops; the curve's fine terms, the three-zone grade and
/// the grain under « Avancé ».
pub fn show(ui: &mut egui::Ui, state: &mut AppState) {
    let auto = state.scene.look.auto_exposure.is_some();
    // Under automatic exposure the manual EV is added on top: a compensation.
    let ev_label = if auto {
        "Compensation"
    } else {
        syfrack_i18n::tr!("Exposition")
    };
    param("look.exposure_ev", ev_label)
        .unit("EV")
        .show(ui, state);
    if let Some(on) = toggle(ui, syfrack_i18n::tr!("Automatique"), auto) {
        state.edit(|s| s.look.auto_exposure = on.then_some(AUTO_EXPOSURE_TARGET));
    }
    param("look.auto_exposure", syfrack_i18n::tr!("Gris cible"))
        .enabled(auto)
        .show(ui, state);
    rule(ui);

    temperature(
        ui,
        state,
        "look.temperature",
        syfrack_i18n::tr!("Température"),
    );
    param("look.tint", syfrack_i18n::tr!("Teinte")).show(ui, state);
    rule(ui);

    let current = match state.scene.look.tonemap {
        Tonemap::Agx { .. } => 0,
        Tonemap::Reinhard { .. } => 1,
        Tonemap::Direct => 2,
    };
    if let Some(i) = choice(
        ui,
        syfrack_i18n::tr!("Courbe"),
        &["AgX", "Reinhard", syfrack_i18n::tr!("Directe")],
        current,
    ) {
        let curve = if i == 0 {
            Tonemap::Agx {
                slope: 1.0,
                power: 1.0,
                offset: 0.0,
            }
        } else if i == 1 {
            reinhard()
        } else {
            Tonemap::Direct
        };
        state.edit(|s| s.look.tonemap = curve);
    }
    param("look.tonemap.agx.power", syfrack_i18n::tr!("Contraste")).show(ui, state);
    param(
        "look.tonemap.reinhard.white",
        syfrack_i18n::tr!("Point blanc"),
    )
    .log()
    .decimals(1)
    .show(ui, state);
    rule(ui);

    param("look.saturation", "Saturation").show(ui, state);
    param("look.vibrance", "Vibrance").show(ui, state);
    rule(ui);

    let vignette = state.scene.look.vignette.is_some();
    if let Some(on) = toggle(ui, syfrack_i18n::tr!("Vignettage"), vignette) {
        state.edit(|s| s.look.vignette = on.then(new_vignette));
    }
    param("look.vignette.intensity", syfrack_i18n::tr!("Intensité")).show(ui, state);
    param("look.vignette.radius", syfrack_i18n::tr!("Rayon")).show(ui, state);

    advanced(ui, "look", |ui| {
        param("look.tonemap.agx.slope", syfrack_i18n::tr!("Pente")).show(ui, state);
        param("look.tonemap.agx.offset", syfrack_i18n::tr!("Décalage"))
            .decimals(3)
            .show(ui, state);
        param("look.vignette.softness", syfrack_i18n::tr!("Douceur")).show(ui, state);
        for (zone, title) in [
            ("shadows", syfrack_i18n::tr!("Ombres")),
            ("midtones", syfrack_i18n::tr!("Tons moyens")),
            ("highlights", syfrack_i18n::tr!("Hautes lumières")),
        ] {
            ui.add_space(theme::GRID / 2.0);
            ui.label(egui::RichText::new(title).small().color(theme::TEXT_DIM));
            param(&format!("look.{zone}.gain"), "Gain").show(ui, state);
            for (ch, label) in [
                ("r", syfrack_i18n::tr!("Rouge")),
                ("g", syfrack_i18n::tr!("Vert")),
                ("b", syfrack_i18n::tr!("Bleu")),
            ] {
                param(&format!("look.{zone}.offset.{ch}"), label)
                    .decimals(3)
                    .show(ui, state);
            }
        }
        ui.add_space(theme::GRID / 2.0);
        let grain = state.scene.look.grain.is_some();
        if let Some(on) = toggle(ui, "Grain", grain) {
            state.edit(|s| s.look.grain = on.then(new_grain));
        }
        param("look.grain.amount", syfrack_i18n::tr!("Quantité"))
            .decimals(3)
            .show(ui, state);
        param("look.grain.size", syfrack_i18n::tr!("Taille"))
            .unit("px")
            .decimals(1)
            .show(ui, state);
        if state.scene.look.grain.is_some() {
            super::labeled(ui, syfrack_i18n::tr!("Graine du grain"), |ui| {
                let mut seed = state.scene.look.grain.as_ref().unwrap().seed;
                if ui.add(egui::DragValue::new(&mut seed)).changed() {
                    state.edit(|s| s.look.grain.as_mut().unwrap().seed = seed);
                }
            });
        }
        if state.scene.look.vignette.is_some() {
            if let Some(on) = toggle(
                ui,
                syfrack_i18n::tr!("Vignette elliptique"),
                state.scene.look.vignette.as_ref().unwrap().elliptical,
            ) {
                state.edit(|s| s.look.vignette.as_mut().unwrap().elliptical = on);
            }
        }
    });
    rule(ui);
    ui.label(egui::RichText::new(syfrack_i18n::tr!("Couleur de l’export")).strong());
    let p3 = state.scene.look.output == syfrack_core::OutputSpace::DisplayP3;
    if let Some(i) = choice(
        ui,
        syfrack_i18n::tr!("Espace couleur"),
        &["sRGB", "Display P3"],
        usize::from(p3),
    ) {
        state.edit(|s| {
            s.look.output = if i == 0 {
                syfrack_core::OutputSpace::Srgb
            } else {
                syfrack_core::OutputSpace::DisplayP3
            }
        });
    }
    if let Some(on) = toggle(
        ui,
        syfrack_i18n::tr!("Dithering à l’export"),
        state.scene.look.dither,
    ) {
        state.edit(|s| s.look.dither = on);
    }
    ui.small(syfrack_i18n::tr!(
        "L’aperçu reste en sRGB. Grain et dithering sont appliqués au PNG final."
    ));
}
