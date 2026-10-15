//! Dielectric interfaces and a homogeneous interior medium in scene units.
use crate::params::{join, visit_rgb, Bounds, Params, UNIT};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Optics {
    /// Stochastic surface coverage, independent of dielectric transmission.
    pub opacity: f32,
    pub transmission: f32,
    pub ior: f32,
    /// A sheet transmits without entering a solid interior medium.
    pub thin_walled: bool,
    /// Beer-Lambert transmittance after `absorption_distance` scene units.
    pub absorption_color: [f32; 3],
    pub absorption_distance: f32,
    pub subsurface: f32,
    pub subsurface_color: [f32; 3],
    /// Mean free path when subsurface = 1. Larger values scatter less often.
    pub subsurface_radius: f32,
}

impl Default for Optics {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            transmission: 1.0,
            ior: 1.5,
            thin_walled: false,
            absorption_color: [1.0; 3],
            absorption_distance: 1.0,
            subsurface: 0.0,
            subsurface_color: [0.9; 3],
            subsurface_radius: 0.5,
        }
    }
}

impl Params for Optics {
    fn visit_params(&mut self, prefix: &str, f: &mut dyn FnMut(&str, &mut f32, Bounds)) {
        f(&join(prefix, "opacity"), &mut self.opacity, UNIT);
        f(&join(prefix, "transmission"), &mut self.transmission, UNIT);
        f(
            &join(prefix, "ior"),
            &mut self.ior,
            Bounds::new(1.0, 3.0, &[(1.0, 2.0)]),
        );
        visit_rgb(
            &join(prefix, "absorption_color"),
            &mut self.absorption_color,
            UNIT,
            f,
        );
        visit_rgb(
            &join(prefix, "subsurface_color"),
            &mut self.subsurface_color,
            UNIT,
            f,
        );
        f(
            &join(prefix, "absorption_distance"),
            &mut self.absorption_distance,
            Bounds::new(0.001, 1000.0, &[(0.1, 10.0)]),
        );
        f(&join(prefix, "subsurface"), &mut self.subsurface, UNIT);
        f(
            &join(prefix, "subsurface_radius"),
            &mut self.subsurface_radius,
            Bounds::new(0.001, 1000.0, &[(0.05, 2.0)]),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Finish, Scene};
    #[test]
    fn old_materials_keep_their_opaque_finish_and_optics_roundtrips() {
        let mut scene = Scene::example();
        assert!(scene.material.optics.is_none());
        scene.material.optics = Some(Optics::default());
        scene.material.finishes.push(Finish {
            optics: Some(Optics {
                subsurface: 0.8,
                thin_walled: true,
                ..Default::default()
            }),
            ..Default::default()
        });
        assert_eq!(Scene::from_ron(&scene.to_ron()).unwrap(), scene);
        for (path, value) in [
            ("ior", 0.9),
            ("opacity", 1.1),
            ("absorption_distance", 0.0),
            ("subsurface_radius", 0.0),
        ] {
            assert!(scene
                .set_param(&format!("material.optics.{path}"), value)
                .is_err());
        }
        assert!(crate::clay_scene(&scene).material.optics.is_none());
    }
}
