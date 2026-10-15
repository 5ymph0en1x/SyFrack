use super::{advanced, param, rgb_bounds, rule, toggle};
use crate::state::AppState;
use crate::ui::widgets::linear_color;

/// The fog read together (density, colour, height), then the cavities;
/// falloff and anisotropy under « Avancé ».
pub fn show(ui: &mut egui::Ui, state: &mut AppState) {
    if let Some(on) = toggle(
        ui,
        syfrack_i18n::tr!("Sol opaque"),
        state.scene.atmosphere.ground.is_some(),
    ) {
        state.edit(|s| s.atmosphere.ground = on.then(syfrack_core::GroundPlane::default));
    }
    if state.scene.atmosphere.ground.is_some() {
        param(
            "atmosphere.ground.height",
            syfrack_i18n::tr!("Niveau du sol"),
        )
        .unit("m")
        .show(ui, state);
        let current = state.scene.atmosphere.ground.as_ref().unwrap().finish;
        let mut selected = current;
        let name = if current == 0 {
            syfrack_i18n::tr!("Matériau global")
        } else {
            state
                .scene
                .material
                .finishes
                .get(current as usize - 1)
                .map_or(syfrack_i18n::tr!("Matériau absent"), |f| f.name.as_str())
        };
        egui::ComboBox::from_id_salt("ground_finish")
            .selected_text(name)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut selected, 0, syfrack_i18n::tr!("Matériau global"));
                for (i, finish) in state.scene.material.finishes.iter().enumerate() {
                    ui.selectable_value(&mut selected, (i + 1) as u32, &finish.name);
                }
            });
        if selected != current {
            state.edit(|s| s.atmosphere.ground.as_mut().unwrap().finish = selected);
        }
        let dunes = state
            .scene
            .atmosphere
            .ground
            .as_ref()
            .unwrap()
            .dunes
            .is_some();
        if let Some(on) = toggle(ui, syfrack_i18n::tr!("Dunes continues"), dunes) {
            state.edit(|s| {
                s.atmosphere.ground.as_mut().unwrap().dunes = on.then(syfrack_core::Dunes::default)
            });
        }
        if state
            .scene
            .atmosphere
            .ground
            .as_ref()
            .unwrap()
            .dunes
            .is_some()
        {
            for (field, label) in [
                ("amplitude", syfrack_i18n::tr!("Relief des dunes")),
                ("wavelength", syfrack_i18n::tr!("Espacement des crêtes")),
                ("wind_direction", syfrack_i18n::tr!("Direction du vent")),
                ("seed", syfrack_i18n::tr!("Variation du terrain")),
            ] {
                param(&format!("atmosphere.ground.dunes.{field}"), label).show(ui, state);
            }
        }
    }
    rule(ui);
    if let Some(on) = toggle(
        ui,
        syfrack_i18n::tr!("Passage vers un autre monde"),
        state.scene.atmosphere.portal.is_some(),
    ) {
        state.edit(|s| s.atmosphere.portal = on.then(syfrack_core::PortalWorld::default));
    }
    if state.scene.atmosphere.portal.is_some() {
        for (field, label) in [
            ("center.x", syfrack_i18n::tr!("Centre X")),
            ("center.y", syfrack_i18n::tr!("Centre Y")),
            ("center.z", syfrack_i18n::tr!("Centre Z")),
            ("radius", syfrack_i18n::tr!("Rayon de l’ouverture")),
            ("scale", syfrack_i18n::tr!("Échelle du paysage")),
            ("elevation", syfrack_i18n::tr!("Altitude d’arrivée")),
            ("energy", syfrack_i18n::tr!("Émission du flux")),
            ("turbulence", syfrack_i18n::tr!("Turbulences")),
            ("time", syfrack_i18n::tr!("Instant du flux")),
        ] {
            param(&format!("atmosphere.portal.{field}"), label).show(ui, state);
        }
        let mut color = state.scene.atmosphere.portal.as_ref().unwrap().color;
        let bounds = rgb_bounds(&state.scene, "atmosphere.portal.color");
        if linear_color(
            ui,
            syfrack_i18n::tr!("Couleur du ciel lointain"),
            &mut color,
            bounds,
            &state.scene.look,
        ) {
            state.edit(|s| s.atmosphere.portal.as_mut().unwrap().color = color);
        }
    }
    rule(ui);
    param("atmosphere.fog_density", syfrack_i18n::tr!("Brouillard"))
        .unit("/m")
        .log()
        .decimals(3)
        .show(ui, state);
    let mut color = state.scene.atmosphere.fog_color;
    let bounds = rgb_bounds(&state.scene, "atmosphere.fog_color");
    if linear_color(
        ui,
        syfrack_i18n::tr!("Couleur"),
        &mut color,
        bounds,
        &state.scene.look,
    ) {
        state.edit(|s| s.atmosphere.fog_color = color);
    }
    param("atmosphere.fog_height", syfrack_i18n::tr!("Hauteur"))
        .unit("m")
        .show(ui, state);
    rule(ui);
    param("atmosphere.cavity_darkness", syfrack_i18n::tr!("Cavités")).show(ui, state);
    advanced(ui, "atmosphere", |ui| {
        param("atmosphere.fog_falloff", syfrack_i18n::tr!("Décroissance"))
            .unit("/m")
            .log()
            .show(ui, state);
        param("atmosphere.anisotropy", syfrack_i18n::tr!("Anisotropie")).show(ui, state);
    });
    rule(ui);
    if let Some(on) = toggle(
        ui,
        syfrack_i18n::tr!("Nuage / nébuleuse volumétrique"),
        state.scene.atmosphere.nebula.is_some(),
    ) {
        state.edit(|s| s.atmosphere.nebula = on.then(syfrack_core::Nebula::default));
    }
    if state.scene.atmosphere.nebula.is_some() {
        let has_particles = state
            .scene
            .atmosphere
            .nebula
            .as_ref()
            .unwrap()
            .particles
            .is_some();
        if let Some(on) = toggle(
            ui,
            syfrack_i18n::tr!("Transport volumétrique"),
            has_particles,
        ) {
            state.edit(|s| {
                s.atmosphere.nebula.as_mut().unwrap().particles =
                    on.then(syfrack_core::Particles::default)
            });
        }
        if state
            .scene
            .atmosphere
            .nebula
            .as_ref()
            .unwrap()
            .particles
            .is_some()
        {
            let smoke = state
                .scene
                .atmosphere
                .nebula
                .as_ref()
                .unwrap()
                .particles
                .as_ref()
                .unwrap()
                .smoke;
            if let Some(on) = toggle(ui, syfrack_i18n::tr!("Émetteur de fumée"), smoke) {
                state.edit(|s| {
                    s.atmosphere
                        .nebula
                        .as_mut()
                        .unwrap()
                        .particles
                        .as_mut()
                        .unwrap()
                        .smoke = on
                });
            }
            let filaments = state
                .scene
                .atmosphere
                .nebula
                .as_ref()
                .unwrap()
                .particles
                .as_ref()
                .unwrap()
                .filaments;
            if let Some(on) = toggle(
                ui,
                syfrack_i18n::tr!("Filaments continus de nébuleuse"),
                filaments,
            ) {
                state.edit(|s| {
                    s.atmosphere
                        .nebula
                        .as_mut()
                        .unwrap()
                        .particles
                        .as_mut()
                        .unwrap()
                        .filaments = on
                });
            }
            if filaments && !smoke {
                let spider = state
                    .scene
                    .atmosphere
                    .nebula
                    .as_ref()
                    .unwrap()
                    .particles
                    .as_ref()
                    .unwrap()
                    .spider;
                if let Some(on) = toggle(
                    ui,
                    syfrack_i18n::tr!("Cavités et fronts d’ionisation"),
                    spider,
                ) {
                    state.edit(|s| {
                        let p = s
                            .atmosphere
                            .nebula
                            .as_mut()
                            .unwrap()
                            .particles
                            .as_mut()
                            .unwrap();
                        p.spider = on;
                        if on {
                            p.storm = false;
                            p.remnant = false;
                        }
                    });
                }
                let storm = state
                    .scene
                    .atmosphere
                    .nebula
                    .as_ref()
                    .unwrap()
                    .particles
                    .as_ref()
                    .unwrap()
                    .storm;
                if let Some(on) = toggle(
                    ui,
                    syfrack_i18n::tr!("Courants ramifiés et déchirure de poussière"),
                    storm,
                ) {
                    state.edit(|s| {
                        let p = s
                            .atmosphere
                            .nebula
                            .as_mut()
                            .unwrap()
                            .particles
                            .as_mut()
                            .unwrap();
                        p.storm = on;
                        if on {
                            p.spider = false;
                            p.remnant = false;
                        }
                    });
                }
                let remnant = state
                    .scene
                    .atmosphere
                    .nebula
                    .as_ref()
                    .unwrap()
                    .particles
                    .as_ref()
                    .unwrap()
                    .remnant;
                if let Some(on) = toggle(
                    ui,
                    syfrack_i18n::tr!("Rémanent de supernova et chocs fragmentés"),
                    remnant,
                ) {
                    state.edit(|s| {
                        let p = s
                            .atmosphere
                            .nebula
                            .as_mut()
                            .unwrap()
                            .particles
                            .as_mut()
                            .unwrap();
                        p.remnant = on;
                        if on {
                            p.spider = false;
                            p.storm = false;
                        }
                    });
                }
                if remnant {
                    for (field, label) in [
                        (
                            "convergence",
                            syfrack_i18n::tr!("Lumière aux rencontres des volutes"),
                        ),
                        ("shock", syfrack_i18n::tr!("Lumière des fronts de choc")),
                        (
                            "shock_radius",
                            syfrack_i18n::tr!("Rayon d’expansion de la supernova"),
                        ),
                        (
                            "gas_opacity",
                            syfrack_i18n::tr!("Opacité du gaz (0 : émission additive)"),
                        ),
                        (
                            "depth_sampling",
                            syfrack_i18n::tr!("Finesse relative en profondeur"),
                        ),
                        (
                            "spectral_detail",
                            syfrack_i18n::tr!("Filaments fins et bandes d’émission"),
                        ),
                    ] {
                        param(&format!("atmosphere.nebula.particles.{field}"), label)
                            .show(ui, state);
                    }
                }
                if spider || storm || remnant {
                    for (field, label) in [
                        ("front_width", syfrack_i18n::tr!("Épaisseur des fronts")),
                        (
                            "dust_clumping",
                            syfrack_i18n::tr!("Concentration des poussières"),
                        ),
                        ("vapour", syfrack_i18n::tr!("Gaz diffus et volutes")),
                        (
                            "gas_clumping",
                            syfrack_i18n::tr!("Répartition en poches du gaz"),
                        ),
                        (
                            "dispersion",
                            syfrack_i18n::tr!("Dispersion autour des nuages"),
                        ),
                        (
                            "cluster_stars",
                            syfrack_i18n::tr!("Densité de l’amas stellaire"),
                        ),
                    ] {
                        param(&format!("atmosphere.nebula.particles.{field}"), label)
                            .show(ui, state);
                    }
                }
            }
            for (field, label) in [
                ("count", syfrack_i18n::tr!("Particules")),
                ("resolution", syfrack_i18n::tr!("Résolution du volume")),
                ("time", syfrack_i18n::tr!("Temps simulé")),
                ("buoyancy", syfrack_i18n::tr!("Flottabilité")),
                ("emitter_radius", syfrack_i18n::tr!("Rayon de l’émetteur")),
                ("inlet_speed", syfrack_i18n::tr!("Vitesse du jet")),
                ("radius", syfrack_i18n::tr!("Rayon du noyau")),
                ("dissipation", "Dissipation"),
                ("shadow_steps", syfrack_i18n::tr!("Pas des ombres")),
                (
                    "multiple_scattering",
                    syfrack_i18n::tr!("Diffusion multiple approchée"),
                ),
            ] {
                if !filaments
                    || smoke
                    || !matches!(
                        field,
                        "count" | "radius" | "emitter_radius" | "inlet_speed" | "buoyancy"
                    )
                {
                    param(&format!("atmosphere.nebula.particles.{field}"), label).show(ui, state);
                }
            }
            for axis in ["x", "y", "z"] {
                param(
                    &format!("atmosphere.nebula.particles.wind.{axis}"),
                    &syfrack_i18n::tr_format!("Vent {axis}", axis = axis),
                )
                .show(ui, state);
            }
            ui.small(syfrack_i18n::tr!("Transport du gaz, projection de pression, reconstruction 3D et cache d’éclairage. Calcul préparé en arrière-plan."));
        }
        for (field, label) in [
            ("density", syfrack_i18n::tr!("Densité du gaz")),
            ("emission", syfrack_i18n::tr!("Émission du gaz")),
            ("dust", syfrack_i18n::tr!("Poussière sombre")),
            ("scattering", syfrack_i18n::tr!("Diffusion du gaz")),
            ("anisotropy", syfrack_i18n::tr!("Direction de diffusion")),
            ("turbulence", syfrack_i18n::tr!("Turbulences")),
            ("detail", syfrack_i18n::tr!("Finesse des filaments")),
            ("angle", syfrack_i18n::tr!("Inclinaison")),
            ("core_energy", syfrack_i18n::tr!("Énergie du foyer")),
            ("core_radius", syfrack_i18n::tr!("Rayon du foyer")),
            ("stars", syfrack_i18n::tr!("Densité des étoiles")),
            ("steps", syfrack_i18n::tr!("Pas dans le volume")),
            ("seed", syfrack_i18n::tr!("Variation de la nébuleuse")),
        ] {
            param(&format!("atmosphere.nebula.{field}"), label).show(ui, state);
        }
        for (field, label) in [
            ("cool_color", syfrack_i18n::tr!("Gaz bleu")),
            ("warm_color", syfrack_i18n::tr!("Gaz chaud")),
        ] {
            let n = state.scene.atmosphere.nebula.as_ref().unwrap();
            let mut color = if field == "cool_color" {
                n.cool_color
            } else {
                n.warm_color
            };
            let bounds = rgb_bounds(&state.scene, &format!("atmosphere.nebula.{field}"));
            if linear_color(ui, label, &mut color, bounds, &state.scene.look) {
                state.edit(|s| {
                    let n = s.atmosphere.nebula.as_mut().unwrap();
                    if field == "cool_color" {
                        n.cool_color = color;
                    } else {
                        n.warm_color = color;
                    }
                });
            }
        }
        advanced(ui, "nebula", |ui| {
            for (field, label) in [
                ("center", "Centre"),
                ("half_size", syfrack_i18n::tr!("Demi-dimension")),
                ("core_position", syfrack_i18n::tr!("Foyer local")),
            ] {
                for axis in ["x", "y", "z"] {
                    param(
                        &format!("atmosphere.nebula.{field}.{axis}"),
                        &format!("{label} {axis}"),
                    )
                    .show(ui, state);
                }
            }
        });
        ui.small(syfrack_i18n::tr!("Absorption et diffusion dans le volume. 32 pas pour étudier, 128–192 pour une grille fine."));
    }
    rule(ui);
    if let Some(on) = toggle(
        ui,
        syfrack_i18n::tr!("Mer et ciel procéduraux"),
        state.scene.atmosphere.seascape.is_some(),
    ) {
        state.edit(|s| s.atmosphere.seascape = on.then(syfrack_core::Seascape::default));
    }
    if state.scene.atmosphere.seascape.is_some() {
        for (path, label) in [
            ("sea_level", syfrack_i18n::tr!("Niveau de la mer")),
            ("intensity", syfrack_i18n::tr!("Lumière du ciel")),
            ("clouds", syfrack_i18n::tr!("Nuages")),
            ("haze", syfrack_i18n::tr!("Brume d’horizon")),
        ] {
            param(&format!("atmosphere.seascape.{path}"), label).show(ui, state);
        }
        for (field, label) in [
            ("sky_zenith", syfrack_i18n::tr!("Ciel au zénith")),
            ("sky_horizon", syfrack_i18n::tr!("Ciel à l’horizon")),
            ("water_color", syfrack_i18n::tr!("Couleur de l’eau")),
        ] {
            let sea = state.scene.atmosphere.seascape.as_ref().unwrap();
            let mut color = match field {
                "sky_zenith" => sea.sky_zenith,
                "sky_horizon" => sea.sky_horizon,
                _ => sea.water_color,
            };
            let bounds = rgb_bounds(&state.scene, &format!("atmosphere.seascape.{field}"));
            if linear_color(ui, label, &mut color, bounds, &state.scene.look) {
                state.edit(|s| {
                    let sea = s.atmosphere.seascape.as_mut().unwrap();
                    match field {
                        "sky_zenith" => sea.sky_zenith = color,
                        "sky_horizon" => sea.sky_horizon = color,
                        _ => sea.water_color = color,
                    }
                });
            }
        }
        rule(ui);
        let physical = state
            .scene
            .atmosphere
            .seascape
            .as_ref()
            .unwrap()
            .surface
            .is_some();
        if let Some(on) = toggle(
            ui,
            syfrack_i18n::tr!("Surface aquatique avec profondeur"),
            physical,
        ) {
            state.edit(|s| {
                s.atmosphere.seascape.as_mut().unwrap().surface =
                    on.then(syfrack_core::OceanSurface::default);
            });
        }
        if state
            .scene
            .atmosphere
            .seascape
            .as_ref()
            .unwrap()
            .surface
            .is_some()
        {
            ocean(ui, state);
        } else {
            param(
                "atmosphere.seascape.wave_strength",
                syfrack_i18n::tr!("Relief du décor distant"),
            )
            .show(ui, state);
            param(
                "atmosphere.seascape.wave_scale",
                syfrack_i18n::tr!("Fréquence du décor"),
            )
            .show(ui, state);
            ui.small(syfrack_i18n::tr!("Activez la surface aquatique pour obtenir un relief réel, de la réfraction et de l’écume."));
        }
        if ui
            .button(syfrack_i18n::tr!("Régler le soleil et les sources →"))
            .clicked()
        {
            state.inspector = crate::state::Inspector::Lights;
        }
    }
}

fn ocean(ui: &mut egui::Ui, state: &mut AppState) {
    ui.small(syfrack_i18n::tr!(
        "Une mer continue au niveau choisi. Retirez tout ancien plan d’eau opaque qui la recouvre."
    ));
    ui.add_space(8.0);
    ui.strong(syfrack_i18n::tr!("Vagues"));
    for (field, label, unit) in [
        ("wave_height", syfrack_i18n::tr!("Hauteur des vagues"), "m"),
        ("wavelength", syfrack_i18n::tr!("Longueur de la houle"), "m"),
        (
            "wind_direction",
            syfrack_i18n::tr!("Direction du vent"),
            "°",
        ),
        ("crest_sharpness", syfrack_i18n::tr!("Crêtes"), ""),
        ("ripple_strength", syfrack_i18n::tr!("Rides de surface"), ""),
        ("time", syfrack_i18n::tr!("Instant de la mer"), "s"),
    ] {
        param(&format!("atmosphere.seascape.surface.{field}"), label)
            .unit(unit)
            .show(ui, state);
    }
    ui.small(syfrack_i18n::tr!(
        "L’instant reste fixe pendant le rendu, pour une image reproductible."
    ));
    rule(ui);
    ui.strong(syfrack_i18n::tr!("Écume"));
    param(
        "atmosphere.seascape.surface.foam_amount",
        syfrack_i18n::tr!("Couverture"),
    )
    .show(ui, state);
    param(
        "atmosphere.seascape.surface.shore_width",
        syfrack_i18n::tr!("Largeur aux rivages"),
    )
    .unit("m")
    .show(ui, state);
    ocean_color(
        ui,
        state,
        "foam_color",
        syfrack_i18n::tr!("Couleur de l’écume"),
    );
    rule(ui);
    ui.strong(syfrack_i18n::tr!("Profondeur et transparence"));
    param(
        "atmosphere.seascape.surface.depth",
        syfrack_i18n::tr!("Profondeur du fond"),
    )
    .unit("m")
    .show(ui, state);
    param(
        "atmosphere.seascape.surface.roughness",
        syfrack_i18n::tr!("Diffusion des reflets"),
    )
    .show(ui, state);
    param(
        "atmosphere.seascape.surface.scattering",
        syfrack_i18n::tr!("Turbidité"),
    )
    .unit("/m")
    .decimals(3)
    .show(ui, state);
    advanced(ui, "ocean-optics", |ui| {
        param(
            "atmosphere.seascape.surface.ior",
            syfrack_i18n::tr!("Indice de réfraction"),
        )
        .decimals(3)
        .show(ui, state);
        ui.small(syfrack_i18n::tr!(
            "Absorption par mètre : augmenter un canal réduit cette couleur sous l’eau."
        ));
        for (channel, label) in [
            ("r", syfrack_i18n::tr!("Absorption rouge")),
            ("g", syfrack_i18n::tr!("Absorption verte")),
            ("b", syfrack_i18n::tr!("Absorption bleue")),
        ] {
            param(
                &format!("atmosphere.seascape.surface.absorption.{channel}"),
                label,
            )
            .unit("/m")
            .decimals(3)
            .show(ui, state);
        }
        ocean_color(
            ui,
            state,
            "seabed_color",
            syfrack_i18n::tr!("Couleur du fond"),
        );
    });
}

fn ocean_color(ui: &mut egui::Ui, state: &mut AppState, field: &str, label: &str) {
    let surface = state
        .scene
        .atmosphere
        .seascape
        .as_ref()
        .unwrap()
        .surface
        .as_ref()
        .unwrap();
    let mut color = if field == "foam_color" {
        surface.foam_color
    } else {
        surface.seabed_color
    };
    let bounds = rgb_bounds(
        &state.scene,
        &format!("atmosphere.seascape.surface.{field}"),
    );
    if linear_color(ui, label, &mut color, bounds, &state.scene.look) {
        state.edit(|s| {
            let surface = s
                .atmosphere
                .seascape
                .as_mut()
                .unwrap()
                .surface
                .as_mut()
                .unwrap();
            if field == "foam_color" {
                surface.foam_color = color;
            } else {
                surface.seabed_color = color;
            }
        });
    }
}
