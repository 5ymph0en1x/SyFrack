//! Procedural sky and optional intersectable, absorbing ocean.
use crate::params::{join, visit_rgb, Bounds, Params, UNIT};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Seascape {
    pub sea_level: f32,
    pub sky_zenith: [f32; 3],
    pub sky_horizon: [f32; 3],
    pub water_color: [f32; 3],
    pub intensity: f32,
    pub wave_strength: f32,
    pub wave_scale: f32,
    pub clouds: f32,
    pub haze: f32,
    /// None preserves the distant backdrop used by older scenes.
    #[serde(default)]
    pub surface: Option<OceanSurface>,
}

/// A displaced dielectric interface above a flat seabed. Distances are metres;
/// absorption/scattering coefficients are inverse metres. Time selects a
/// reproducible wave state (there is no wall-clock animation during rendering).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OceanSurface {
    pub wave_height: f32,
    pub wavelength: f32,
    pub wind_direction: f32,
    pub crest_sharpness: f32,
    pub time: f32,
    pub ripple_strength: f32,
    pub roughness: f32,
    pub foam_amount: f32,
    pub shore_width: f32,
    pub depth: f32,
    pub ior: f32,
    pub absorption: [f32; 3],
    pub scattering: f32,
    pub foam_color: [f32; 3],
    pub seabed_color: [f32; 3],
}

impl Default for OceanSurface {
    fn default() -> Self {
        Self {
            wave_height: 0.45,
            wavelength: 8.0,
            wind_direction: 25.0,
            crest_sharpness: 0.55,
            time: 0.0,
            ripple_strength: 0.12,
            roughness: 0.12,
            foam_amount: 0.5,
            shore_width: 0.65,
            depth: 12.0,
            ior: 1.333,
            absorption: [0.28, 0.065, 0.035],
            scattering: 0.025,
            foam_color: [0.88, 0.93, 0.91],
            seabed_color: [0.19, 0.23, 0.20],
        }
    }
}

impl Params for OceanSurface {
    fn visit_params(&mut self, prefix: &str, f: &mut dyn FnMut(&str, &mut f32, Bounds)) {
        for (name, value, bounds) in [
            (
                "wave_height",
                &mut self.wave_height,
                Bounds::new(0.0, 4.0, &[(0.0, 1.0)]),
            ),
            (
                "wavelength",
                &mut self.wavelength,
                Bounds::new(1.0, 100.0, &[(2.0, 20.0)]),
            ),
            (
                "wind_direction",
                &mut self.wind_direction,
                Bounds::new(-180.0, 180.0, &[]),
            ),
            ("crest_sharpness", &mut self.crest_sharpness, UNIT),
            (
                "time",
                &mut self.time,
                Bounds::new(-10000.0, 10000.0, &[(0.0, 30.0)]),
            ),
            ("ripple_strength", &mut self.ripple_strength, UNIT),
            (
                "roughness",
                &mut self.roughness,
                Bounds::new(0.02, 0.5, &[(0.02, 0.2)]),
            ),
            ("foam_amount", &mut self.foam_amount, UNIT),
            (
                "shore_width",
                &mut self.shore_width,
                Bounds::new(0.0, 10.0, &[(0.0, 2.0)]),
            ),
            (
                "depth",
                &mut self.depth,
                Bounds::new(3.0, 200.0, &[(3.0, 30.0)]),
            ),
            ("ior", &mut self.ior, Bounds::new(1.0, 2.0, &[(1.30, 1.38)])),
            (
                "scattering",
                &mut self.scattering,
                Bounds::new(0.0, 2.0, &[(0.0, 0.1)]),
            ),
        ] {
            f(&join(prefix, name), value, bounds);
        }
        visit_rgb(
            &join(prefix, "absorption"),
            &mut self.absorption,
            Bounds::new(0.0, 10.0, &[(0.0, 0.5)]),
            f,
        );
        visit_rgb(&join(prefix, "foam_color"), &mut self.foam_color, UNIT, f);
        visit_rgb(
            &join(prefix, "seabed_color"),
            &mut self.seabed_color,
            UNIT,
            f,
        );
    }
}

impl Default for Seascape {
    fn default() -> Self {
        Self {
            sea_level: -1.2,
            sky_zenith: [0.12, 0.10, 0.28],
            sky_horizon: [1.15, 0.38, 0.16],
            water_color: [0.025, 0.075, 0.11],
            intensity: 1.0,
            wave_strength: 0.22,
            wave_scale: 0.65,
            clouds: 0.35,
            haze: 0.35,
            surface: None,
        }
    }
}

impl Params for Seascape {
    fn visit_params(&mut self, prefix: &str, f: &mut dyn FnMut(&str, &mut f32, Bounds)) {
        f(
            &join(prefix, "sea_level"),
            &mut self.sea_level,
            Bounds::new(-100.0, 100.0, &[(-5.0, 0.0)]),
        );
        let radiance = Bounds::new(0.0, 10.0, &[(0.0, 2.0)]);
        visit_rgb(
            &join(prefix, "sky_zenith"),
            &mut self.sky_zenith,
            radiance,
            f,
        );
        visit_rgb(
            &join(prefix, "sky_horizon"),
            &mut self.sky_horizon,
            radiance,
            f,
        );
        visit_rgb(&join(prefix, "water_color"), &mut self.water_color, UNIT, f);
        f(
            &join(prefix, "intensity"),
            &mut self.intensity,
            Bounds::new(0.0, 10.0, &[(0.2, 2.0)]),
        );
        f(
            &join(prefix, "wave_strength"),
            &mut self.wave_strength,
            UNIT,
        );
        f(
            &join(prefix, "wave_scale"),
            &mut self.wave_scale,
            Bounds::new(0.01, 10.0, &[(0.1, 2.0)]),
        );
        f(&join(prefix, "clouds"), &mut self.clouds, UNIT);
        f(&join(prefix, "haze"), &mut self.haze, UNIT);
        if let Some(surface) = &mut self.surface {
            surface.visit_params(&join(prefix, "surface"), f);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Scene;

    #[test]
    fn seascape_is_optional_validated_and_saved() {
        let mut s = Scene::example();
        assert!(Scene::from_ron(&s.to_ron().replace("seascape: None,", ""))
            .unwrap()
            .atmosphere
            .seascape
            .is_none());
        s.atmosphere.seascape = Some(Seascape::default());
        s.validate().unwrap();
        s.set_param("atmosphere.seascape.sea_level", -2.5).unwrap();
        assert!(s.set_param("atmosphere.seascape.wave_scale", 0.0).is_err());
        assert_eq!(Scene::from_ron(&s.to_ron()).unwrap(), s);
        assert!(crate::clay_scene(&s).atmosphere.seascape.is_none());
    }

    #[test]
    fn ocean_is_backwards_compatible_and_rejects_invalid_optics() {
        let mut s = Scene::example();
        s.atmosphere.seascape = Some(Seascape::default());
        let old = s.to_ron().replace("surface: None,", "");
        assert!(Scene::from_ron(&old)
            .unwrap()
            .atmosphere
            .seascape
            .unwrap()
            .surface
            .is_none());
        s.atmosphere.seascape.as_mut().unwrap().surface = Some(OceanSurface::default());
        s.validate().unwrap();
        assert_eq!(Scene::from_ron(&s.to_ron()).unwrap(), s);
        assert!(s.set_param("atmosphere.seascape.surface.ior", 0.9).is_err());
        assert!(s
            .set_param("atmosphere.seascape.surface.absorption.r", -0.1)
            .is_err());
        s.atmosphere
            .seascape
            .as_mut()
            .unwrap()
            .surface
            .as_mut()
            .unwrap()
            .wavelength = f32::NAN;
        assert!(s.validate().is_err());
    }

    #[test]
    fn partial_ocean_settings_use_documented_defaults() {
        let sea: OceanSurface = ron::from_str("(wave_height:0.8,time:12.0)").unwrap();
        assert_eq!(sea.wave_height, 0.8);
        assert_eq!(sea.wavelength, OceanSurface::default().wavelength);
        assert!(ron::from_str::<OceanSurface>("(wav_height:1.0)").is_err());
    }
}
