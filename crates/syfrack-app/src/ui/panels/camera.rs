use syfrack_core::CameraMode;

use super::{advanced, choice, param, rule};
use crate::input;
use crate::state::AppState;

/// Distance in front of a free camera where « Orbite » puts the orbit point.
const ORBIT_REACH: f32 = 3.0;

/// Mode, then the lens read together (focal, aperture, focus), then the
/// framing of the orbit; the orbit point and roll under « Avancé ».
pub fn show(ui: &mut egui::Ui, state: &mut AppState) {
    let is_orbit = matches!(state.scene.camera.mode, CameraMode::Orbit { .. });
    if let Some(i) = choice(
        ui,
        "Mode",
        &[syfrack_i18n::tr!("Orbite"), syfrack_i18n::tr!("Libre")],
        usize::from(!is_orbit),
    ) {
        let basis = state.scene.camera.basis();
        let mode = if i == 0 {
            input::orbit_around(&basis, basis.origin + basis.forward * ORBIT_REACH, 0.0)
        } else {
            CameraMode::Free {
                position: basis.origin,
                target: basis.origin + basis.forward,
                roll: 0.0,
            }
        };
        state.edit(|s| s.camera.mode = mode);
    }
    rule(ui);
    param("camera.focal_mm", syfrack_i18n::tr!("Focale"))
        .unit("mm")
        .log()
        .decimals(0)
        .show(ui, state);
    param("camera.f_number", syfrack_i18n::tr!("Ouverture f/"))
        .decimals(1)
        .show(ui, state);
    param("camera.focus_distance", syfrack_i18n::tr!("Mise au point"))
        .unit("m")
        .log()
        .show(ui, state);
    rule(ui);
    if is_orbit {
        param("camera.orbit.distance", "Distance")
            .unit("m")
            .log()
            .no_reference()
            .show(ui, state);
        param("camera.orbit.azimuth", syfrack_i18n::tr!("Azimut"))
            .unit("°")
            .decimals(1)
            .no_reference()
            .show(ui, state);
        param("camera.orbit.elevation", syfrack_i18n::tr!("Élévation"))
            .unit("°")
            .decimals(1)
            .no_reference()
            .show(ui, state);
        advanced(ui, "camera", |ui| {
            for axis in ["x", "y", "z"] {
                param(
                    &format!("camera.orbit.point.{axis}"),
                    &syfrack_i18n::tr_format!("Point {axis}", axis = axis),
                )
                .unit("m")
                .no_reference()
                .show(ui, state);
            }
            param("camera.orbit.roll", syfrack_i18n::tr!("Roulis"))
                .unit("°")
                .decimals(1)
                .show(ui, state);
        });
    } else {
        param("camera.free.roll", syfrack_i18n::tr!("Roulis"))
            .unit("°")
            .decimals(1)
            .show(ui, state);
        advanced(ui, "camera", |ui| {
            for axis in ["x", "y", "z"] {
                param(
                    &format!("camera.free.position.{axis}"),
                    &format!("Position {axis}"),
                )
                .unit("m")
                .no_reference()
                .show(ui, state);
            }
            for axis in ["x", "y", "z"] {
                param(
                    &format!("camera.free.target.{axis}"),
                    &syfrack_i18n::tr_format!("Cible {axis}", axis = axis),
                )
                .unit("m")
                .no_reference()
                .show(ui, state);
            }
        });
    }
}
