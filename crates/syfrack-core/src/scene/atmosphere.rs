use crate::params::{join, visit_rgb, visit_u32, visit_vec3, Bounds, Params, POSITION, UNIT};
use glam::Vec3;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Atmosphere {
    pub fog_density: f32,
    pub fog_color: [f32; 3],
    /// Height at which the density is `fog_density`; it decays above with `fog_falloff`.
    pub fog_height: f32,
    pub fog_falloff: f32,
    /// Henyey-Greenstein anisotropy, -0.99..0.99.
    pub anisotropy: f32,
    /// Extra darkening of enclosed cavities, 0..1.
    pub cavity_darkness: f32,
    #[serde(default)]
    pub seascape: Option<super::Seascape>,
    /// Opaque horizontal ground, intersected analytically outside the form graph.
    #[serde(default)]
    pub ground: Option<GroundPlane>,
    /// A vertical circular opening into a procedural, independently lit landscape.
    #[serde(default)]
    pub portal: Option<PortalWorld>,
    /// Bounded, self-emitting fractal gas and absorbing dust.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nebula: Option<super::Nebula>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroundPlane {
    pub height: f32,
    /// Zero uses the global material; 1..=8 selects a named finish.
    pub finish: u32,
    #[serde(default)]
    pub dunes: Option<Dunes>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dunes {
    pub amplitude: f32,
    pub wavelength: f32,
    pub wind_direction: f32,
    pub seed: u32,
}

impl Default for Dunes {
    fn default() -> Self {
        Self {
            amplitude: 2.8,
            wavelength: 38.0,
            wind_direction: 25.0,
            seed: 101,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortalWorld {
    pub center: Vec3,
    pub radius: f32,
    pub scale: f32,
    pub elevation: f32,
    pub color: [f32; 3],
    #[serde(default = "default_portal_energy")]
    pub energy: f32,
    #[serde(default)]
    pub time: f32,
    #[serde(default = "default_portal_turbulence")]
    pub turbulence: f32,
}

fn default_portal_energy() -> f32 {
    2.0
}
fn default_portal_turbulence() -> f32 {
    1.0
}

impl Default for PortalWorld {
    fn default() -> Self {
        Self {
            center: Vec3::new(0.0, 12.0, 0.0),
            radius: 10.0,
            scale: 4.0,
            elevation: 75.0,
            color: [0.025, 0.22, 1.0],
            energy: default_portal_energy(),
            time: 0.0,
            turbulence: default_portal_turbulence(),
        }
    }
}

impl Atmosphere {
    pub fn none() -> Self {
        Self {
            fog_density: 0.0,
            fog_color: [1.0, 1.0, 1.0],
            fog_height: 0.0,
            fog_falloff: 1.0,
            anisotropy: 0.0,
            cavity_darkness: 0.0,
            seascape: None,
            ground: None,
            portal: None,
            nebula: None,
        }
    }
}

const DENSITY: Bounds = Bounds::new(0.0, 10.0, &[(0.0, 0.3)]);
const HEIGHT: Bounds = Bounds::new(-100.0, 100.0, &[(-2.0, 2.0)]);
const FALLOFF: Bounds = Bounds::new(0.0, 10.0, &[(0.1, 2.0)]);
const ANISOTROPY: Bounds = Bounds::new(-0.99, 0.99, &[(0.0, 0.7)]);
const CAVITY: Bounds = Bounds::new(0.0, 1.0, &[(0.0, 0.8)]);

impl Params for Atmosphere {
    fn visit_params(&mut self, prefix: &str, f: &mut dyn FnMut(&str, &mut f32, Bounds)) {
        if let Some(nebula) = &mut self.nebula {
            nebula.visit_params(&join(prefix, "nebula"), f);
        }
        if let Some(sea) = &mut self.seascape {
            sea.visit_params(&join(prefix, "seascape"), f);
        }
        if let Some(ground) = &mut self.ground {
            f(&join(prefix, "ground.height"), &mut ground.height, HEIGHT);
            visit_u32(
                &join(prefix, "ground.finish"),
                &mut ground.finish,
                super::graph::MATERIAL_INDEX,
                f,
            );
            if let Some(d) = &mut ground.dunes {
                f(
                    &join(prefix, "ground.dunes.amplitude"),
                    &mut d.amplitude,
                    Bounds::new(0.0, 20.0, &[(0.0, 6.0)]),
                );
                f(
                    &join(prefix, "ground.dunes.wavelength"),
                    &mut d.wavelength,
                    Bounds::new(5.0, 200.0, &[(10.0, 80.0)]),
                );
                f(
                    &join(prefix, "ground.dunes.wind_direction"),
                    &mut d.wind_direction,
                    super::graph::ANGLE_DEG,
                );
                visit_u32(
                    &join(prefix, "ground.dunes.seed"),
                    &mut d.seed,
                    Bounds::new(0.0, 65535.0, &[(0.0, 65535.0)]),
                    f,
                );
            }
        }
        if let Some(p) = &mut self.portal {
            visit_vec3(&join(prefix, "portal.center"), &mut p.center, POSITION, f);
            f(
                &join(prefix, "portal.radius"),
                &mut p.radius,
                Bounds::new(0.01, 100.0, &[(1.0, 20.0)]),
            );
            f(
                &join(prefix, "portal.scale"),
                &mut p.scale,
                Bounds::new(0.1, 20.0, &[(1.0, 8.0)]),
            );
            f(
                &join(prefix, "portal.elevation"),
                &mut p.elevation,
                Bounds::new(10.0, 300.0, &[(30.0, 150.0)]),
            );
            visit_rgb(&join(prefix, "portal.color"), &mut p.color, UNIT, f);
            f(
                &join(prefix, "portal.energy"),
                &mut p.energy,
                Bounds::new(0.0, 100.0, &[(0.0, 10.0)]),
            );
            f(
                &join(prefix, "portal.time"),
                &mut p.time,
                Bounds::new(0.0, 10000.0, &[(0.0, 60.0)]),
            );
            f(
                &join(prefix, "portal.turbulence"),
                &mut p.turbulence,
                Bounds::new(0.0, 4.0, &[(0.0, 2.0)]),
            );
        }
        f(&join(prefix, "fog_density"), &mut self.fog_density, DENSITY);
        visit_rgb(&join(prefix, "fog_color"), &mut self.fog_color, UNIT, f);
        f(&join(prefix, "fog_height"), &mut self.fog_height, HEIGHT);
        f(&join(prefix, "fog_falloff"), &mut self.fog_falloff, FALLOFF);
        f(
            &join(prefix, "anisotropy"),
            &mut self.anisotropy,
            ANISOTROPY,
        );
        f(
            &join(prefix, "cavity_darkness"),
            &mut self.cavity_darkness,
            CAVITY,
        );
    }
}
