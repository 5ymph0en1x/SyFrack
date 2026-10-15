use crate::params::{
    join, visit_rgb, visit_vec3, Bounds, Params, ANGLE, DIRECTION, KELVIN, POSITION, UNIT,
};
use glam::Vec3;
use serde::{Deserialize, Serialize};

pub const MAX_LIGHTS: usize = 96;

/// A light colour: either a black-body temperature or a linear RGB triple.
/// Never sRGB.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Color {
    Kelvin(f32),
    Linear([f32; 3]),
}

impl Color {
    /// Linear RGB, normalised so the brightest channel is 1 for Kelvin colours.
    pub fn to_linear(&self) -> [f32; 3] {
        match self {
            Color::Linear(c) => *c,
            Color::Kelvin(k) => kelvin_to_linear(*k),
        }
    }
}

/// Tanner Helland's black-body approximation, converted from sRGB to linear.
fn kelvin_to_linear(kelvin: f32) -> [f32; 3] {
    let t = kelvin.clamp(1000.0, 40000.0) / 100.0;
    let r = if t <= 66.0 {
        255.0
    } else {
        329.698_73 * (t - 60.0).powf(-0.133_204_76)
    };
    let g = if t <= 66.0 {
        99.470_8 * t.ln() - 161.119_57
    } else {
        288.122_17 * (t - 60.0).powf(-0.075_514_85)
    };
    let b = if t >= 66.0 {
        255.0
    } else if t <= 19.0 {
        0.0
    } else {
        138.517_73 * (t - 10.0).ln() - 305.044_8
    };
    let mut c = [r, g, b].map(|v| srgb_to_linear((v / 255.0).clamp(0.0, 1.0)));
    let max = c[0].max(c[1]).max(c[2]).max(1e-6);
    for v in &mut c {
        *v /= max;
    }
    c
}

pub fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Light {
    /// `direction` points from the surface toward the sun. `intensity` is the
    /// irradiance on a perpendicular surface. `angular_size` is the full
    /// angular diameter in degrees (the real sun is about 0.53).
    Sun {
        direction: Vec3,
        color: Color,
        intensity: f32,
        angular_size: f32,
    },
    /// An opaque rectangular emitter centred at `position`, emitting only
    /// along `normal`, `size` = (width, height). `intensity` is radiance.
    /// Camera/reflection rays see the emitting face; its back is black.
    Area {
        position: Vec3,
        normal: Vec3,
        size: [f32; 2],
        color: Color,
        intensity: f32,
    },
    /// Rectangular projector with a continuous angular radiance profile.
    /// Angles are cone half-angles in degrees; radiance is `intensity` on-axis.
    Projector {
        position: Vec3,
        normal: Vec3,
        size: [f32; 2],
        color: Color,
        intensity: f32,
        inner_angle: f32,
        outer_angle: f32,
    },
    /// Opaque capsule emitter, radiating outwards from its entire surface.
    /// The visible tube and its next-event samples use exactly the same shape.
    Tube {
        start: Vec3,
        end: Vec3,
        radius: f32,
        color: Color,
        intensity: f32,
    },
    /// Sky gradient. `rotation` (degrees) is reserved for HDR maps and is
    /// ignored for gradients.
    Environment {
        zenith: Color,
        horizon: Color,
        ground: Color,
        intensity: f32,
        rotation: f32,
    },
    /// Linear RGB equirectangular Radiance HDR or OpenEXR panorama.
    /// Relative paths are resolved against the scene file when loading.
    /// Rotation moves longitude zero (+X) toward +Z; north is +Y.
    Hdri {
        path: std::path::PathBuf,
        intensity: f32,
        rotation: f32,
    },
}

const SUN_INTENSITY: Bounds = Bounds::new(1e-3, 100000.0, &[(0.5, 20.0)]);
const ANGULAR_SIZE: Bounds = Bounds::new(0.01, 90.0, &[(0.5, 5.0)]);
const AREA_SIZE: Bounds = Bounds::new(0.001, 1000.0, &[(0.1, 3.0)]);
const AREA_INTENSITY: Bounds = Bounds::new(1e-3, 100000.0, &[(1.0, 50.0)]);
const ENV_INTENSITY: Bounds = Bounds::new(0.0, 100.0, &[(0.1, 2.0)]);

impl Params for Color {
    fn visit_params(&mut self, prefix: &str, f: &mut dyn FnMut(&str, &mut f32, Bounds)) {
        match self {
            Color::Kelvin(k) => f(&join(prefix, "kelvin"), k, KELVIN),
            Color::Linear(c) => visit_rgb(prefix, c, UNIT, f),
        }
    }
}

impl Params for Light {
    fn visit_params(&mut self, prefix: &str, f: &mut dyn FnMut(&str, &mut f32, Bounds)) {
        match self {
            Light::Sun {
                direction,
                color,
                intensity,
                angular_size,
            } => {
                let p = join(prefix, "sun");
                visit_vec3(&join(&p, "direction"), direction, DIRECTION, f);
                color.visit_params(&join(&p, "color"), f);
                f(&join(&p, "intensity"), intensity, SUN_INTENSITY);
                f(&join(&p, "angular_size"), angular_size, ANGULAR_SIZE);
            }
            Light::Area {
                position,
                normal,
                size,
                color,
                intensity,
            }
            | Light::Projector {
                position,
                normal,
                size,
                color,
                intensity,
                ..
            } => {
                let p = join(prefix, "area");
                visit_vec3(&join(&p, "position"), position, POSITION, f);
                visit_vec3(&join(&p, "normal"), normal, DIRECTION, f);
                f(&join(&p, "size.w"), &mut size[0], AREA_SIZE);
                f(&join(&p, "size.h"), &mut size[1], AREA_SIZE);
                color.visit_params(&join(&p, "color"), f);
                f(&join(&p, "intensity"), intensity, AREA_INTENSITY);
            }
            Light::Tube {
                start,
                end,
                radius,
                color,
                intensity,
            } => {
                let p = join(prefix, "tube");
                visit_vec3(&join(&p, "start"), start, POSITION, f);
                visit_vec3(&join(&p, "end"), end, POSITION, f);
                f(&join(&p, "radius"), radius, AREA_SIZE);
                color.visit_params(&join(&p, "color"), f);
                f(&join(&p, "intensity"), intensity, AREA_INTENSITY);
            }
            Light::Environment {
                zenith,
                horizon,
                ground,
                intensity,
                rotation,
            } => {
                let p = join(prefix, "environment");
                zenith.visit_params(&join(&p, "zenith"), f);
                horizon.visit_params(&join(&p, "horizon"), f);
                ground.visit_params(&join(&p, "ground"), f);
                f(&join(&p, "intensity"), intensity, ENV_INTENSITY);
                f(&join(&p, "rotation"), rotation, ANGLE);
            }
            Light::Hdri {
                intensity,
                rotation,
                ..
            } => {
                let p = join(prefix, "hdri");
                f(&join(&p, "intensity"), intensity, ENV_INTENSITY);
                f(&join(&p, "rotation"), rotation, ANGLE);
            }
        }
        if let Light::Projector {
            inner_angle,
            outer_angle,
            ..
        } = self
        {
            let p = join(prefix, "area");
            let inner_bounds = Bounds::new(0.1, *outer_angle - 0.01, &[(5.0, 15.0)]);
            f(&join(&p, "inner_angle"), inner_angle, inner_bounds);
            let outer_bounds = Bounds::new(*inner_angle + 0.01, 89.0, &[(15.0, 35.0)]);
            f(&join(&p, "outer_angle"), outer_angle, outer_bounds);
        }
    }
}

#[cfg(test)]
mod projector_tests {
    use super::*;
    #[test]
    fn projectors_roundtrip_and_validate_a_nonempty_smooth_angular_profile() {
        let mut scene = crate::Scene::example();
        scene.lights.push(Light::Projector {
            position: Vec3::new(1.0, 2.0, 3.0),
            normal: Vec3::NEG_Z,
            size: [0.8, 1.2],
            color: Color::Linear([0.01, 0.2, 1.0]),
            intensity: 1800.0,
            inner_angle: 10.0,
            outer_angle: 18.0,
        });
        assert_eq!(crate::Scene::from_ron(&scene.to_ron()).unwrap(), scene);
        let i = scene.lights.len() - 1;
        scene
            .set_param(&format!("lights[{i}].area.outer_angle"), 20.0)
            .unwrap();
        assert!(scene
            .set_param(&format!("lights[{i}].area.inner_angle"), 20.0)
            .is_err());
        if let Light::Projector {
            inner_angle,
            outer_angle,
            ..
        } = &mut scene.lights[i]
        {
            *inner_angle = 20.0;
            *outer_angle = 10.0;
        }
        assert!(scene.validate().is_err());
    }
}
