//! Native orbit-density images. These artistic fields are not photon transport.
use crate::params::{join, Bounds, Params};
use crate::{Ramp, SceneError};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum FlameVariation {
    Linear,
    Sinusoidal,
    Spherical,
    Swirl,
    Horseshoe,
    Disc,
    Bubble,
    Julian { power: u32, distance: f32 },
    Curl { c1: f32, c2: f32 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlameTerm {
    pub amount: f32,
    pub variation: FlameVariation,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FlameTransform {
    pub weight: f32,
    /// x'=a*x+b*y+c, y'=d*x+e*y+f.
    pub affine: [f32; 6],
    pub terms: Vec<FlameTerm>,
    pub post: [f32; 6],
    pub color: f32,
    pub color_speed: f32,
}

impl Default for FlameTransform {
    fn default() -> Self {
        Self {
            weight: 1.0,
            affine: [0.7, -0.3, 0.2, 0.3, 0.7, 0.0],
            terms: vec![FlameTerm {
                amount: 1.0,
                variation: FlameVariation::Linear,
            }],
            post: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            color: 0.5,
            color_speed: 0.5,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Flame {
    pub transforms: Vec<FlameTransform>,
    pub final_transform: Option<FlameTransform>,
    pub palette: Ramp,
    /// A fixed supersampled histogram; reconstruction leaves its stored moments intact.
    pub supersampling: u32,
    /// Retained orbit points per pixel and session sample, before clipping.
    pub points_per_sample: u32,
    pub brightness: f32,
    pub density_log: f32,
    pub filter_radius: f32,
    pub background: [f32; 3],
}

impl Default for Flame {
    fn default() -> Self {
        Self {
            transforms: (0..3)
                .map(|i| FlameTransform {
                    affine: [
                        0.5,
                        0.0,
                        [-0.8, 0.8, 0.0][i],
                        0.0,
                        0.5,
                        [-0.5, -0.5, 0.8][i],
                    ],
                    color: i as f32 / 2.0,
                    ..FlameTransform::default()
                })
                .collect(),
            final_transform: None,
            palette: Ramp::default(),
            supersampling: 2,
            points_per_sample: 4,
            brightness: 1.0,
            density_log: 8.0,
            filter_radius: 1.2,
            background: [0.0; 3],
        }
    }
}

impl Params for FlameTransform {
    fn visit_params(&mut self, prefix: &str, f: &mut dyn FnMut(&str, &mut f32, Bounds)) {
        f(
            &join(prefix, "weight"),
            &mut self.weight,
            Bounds::new(0.0, 100.0, &[(0.0, 4.0)]),
        );
        for (name, values) in [("affine", &mut self.affine), ("post", &mut self.post)] {
            for (i, v) in values.iter_mut().enumerate() {
                f(
                    &format!("{prefix}.{name}[{i}]"),
                    v,
                    Bounds::new(-20.0, 20.0, &[(-2.0, 2.0)]),
                );
            }
        }
        for (i, t) in self.terms.iter_mut().enumerate() {
            let p = format!("{prefix}.terms[{i}]");
            f(
                &join(&p, "amount"),
                &mut t.amount,
                Bounds::new(-8.0, 8.0, &[(-1.0, 2.0)]),
            );
            match &mut t.variation {
                FlameVariation::Julian { power, distance } => {
                    crate::params::visit_u32(
                        &join(&p, "power"),
                        power,
                        Bounds::new(1.0, 16.0, &[(1.0, 6.0)]),
                        f,
                    );
                    f(
                        &join(&p, "distance"),
                        distance,
                        Bounds::new(-4.0, 4.0, &[(-1.0, 2.0)]),
                    );
                }
                FlameVariation::Curl { c1, c2 } => {
                    f(&join(&p, "c1"), c1, Bounds::new(-4.0, 4.0, &[(-1.0, 1.0)]));
                    f(&join(&p, "c2"), c2, Bounds::new(-4.0, 4.0, &[(-1.0, 1.0)]));
                }
                _ => {}
            }
        }
        for (name, v) in [
            ("color", &mut self.color),
            ("color_speed", &mut self.color_speed),
        ] {
            f(&join(prefix, name), v, crate::params::UNIT);
        }
    }
}

impl Params for Flame {
    fn visit_params(&mut self, prefix: &str, f: &mut dyn FnMut(&str, &mut f32, Bounds)) {
        for (i, t) in self.transforms.iter_mut().enumerate() {
            t.visit_params(&format!("{prefix}.transforms[{i}]"), f);
        }
        if let Some(t) = &mut self.final_transform {
            t.visit_params(&join(prefix, "final_transform"), f);
        }
        self.palette.visit_params(&join(prefix, "palette"), f);
        crate::params::visit_u32(
            &join(prefix, "supersampling"),
            &mut self.supersampling,
            Bounds::new(1.0, 3.0, &[(1.0, 2.0)]),
            f,
        );
        crate::params::visit_u32(
            &join(prefix, "points_per_sample"),
            &mut self.points_per_sample,
            Bounds::new(1.0, 32.0, &[(1.0, 8.0)]),
            f,
        );
        for (name, v, bounds) in [
            (
                "brightness",
                &mut self.brightness,
                Bounds::new(0.0, 100.0, &[(0.1, 4.0)]),
            ),
            (
                "density_log",
                &mut self.density_log,
                Bounds::new(0.0, 100.0, &[(0.0, 20.0)]),
            ),
            (
                "filter_radius",
                &mut self.filter_radius,
                Bounds::new(0.3, 3.0, &[(0.4, 2.0)]),
            ),
        ] {
            f(&join(prefix, name), v, bounds);
        }
        crate::params::visit_rgb(
            &join(prefix, "background"),
            &mut self.background,
            crate::params::UNIT,
            f,
        );
    }
}

impl Flame {
    pub fn validate(&self) -> Result<(), SceneError> {
        if !(1..=16).contains(&self.transforms.len())
            || self.transforms.iter().map(|t| t.weight).sum::<f32>() <= 0.0
        {
            return Err(SceneError::Graph(
                syfrack_i18n::tr!("flamme : 1 à 16 transformations, poids total positif").into(),
            ));
        }
        for t in self.transforms.iter().chain(self.final_transform.iter()) {
            if t.terms.is_empty() || t.terms.len() > 16 || t.terms.iter().all(|v| v.amount == 0.0) {
                return Err(SceneError::Graph(
                    syfrack_i18n::tr!("flamme : 1 à 16 variations, contribution non nulle").into(),
                ));
            }
        }
        self.palette.validate("canvas.flame.palette")?;
        crate::params::validate_params(self, "canvas.flame")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn flame_roundtrip_and_invalid_orbit_inputs() {
        let mut scene = crate::Scene::example_2d();
        scene.canvas.flame = Some(Flame::default());
        assert_eq!(crate::Scene::from_ron(&scene.to_ron()).unwrap(), scene);
        scene.validate().unwrap();
        for t in &mut scene.canvas.flame.as_mut().unwrap().transforms {
            t.weight = 0.0;
        }
        assert!(scene.validate().is_err());
        scene.canvas.flame.as_mut().unwrap().transforms[0].weight = 1.0;
        scene.canvas.flame.as_mut().unwrap().transforms[0].terms[0].variation =
            FlameVariation::Julian {
                power: 0,
                distance: 1.0,
            };
        assert!(scene.validate().is_err());
        let mut old = crate::Scene::example_2d();
        old.canvas.flame = None;
        assert!(crate::Scene::from_ron(&old.to_ron())
            .unwrap()
            .canvas
            .flame
            .is_none());
    }
}
