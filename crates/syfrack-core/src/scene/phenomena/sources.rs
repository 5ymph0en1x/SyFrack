//! Bounded GPU orbit-density resources sampled by ordinary field nodes.
use super::FrozenSnapshot;
use crate::{Flame, SceneError};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlameChannel {
    Color,
    Density,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FlameField {
    pub flame: Flame,
    pub center: [f32; 2],
    pub half: [f32; 2],
    pub angle: f32,
    pub resolution: u32,
    /// Fixed resource budget, independent of image tiles and render samples.
    pub points: u32,
    pub seed: u64,
    pub frozen: Option<FrozenSnapshot>,
}
impl Default for FlameField {
    fn default() -> Self {
        Self {
            flame: Flame::default(),
            center: [0.0; 2],
            half: [3.0; 2],
            angle: 0.0,
            resolution: 1024,
            points: 4_194_304,
            seed: 7,
            frozen: None,
        }
    }
}
impl FlameField {
    pub fn validate(&self) -> Result<(), SceneError> {
        self.flame.validate()?;
        if !(32..=2048).contains(&self.resolution)
            || !(1..=67_108_863).contains(&self.points)
            || !self
                .center
                .iter()
                .chain(&self.half)
                .chain([&self.angle])
                .all(|v| v.is_finite())
            || self.half.iter().any(|v| *v <= 0.0)
        {
            return Err(SceneError::Graph(
                "invalid flame field domain or finite GPU point budget".into(),
            ));
        }
        if let Some(f) = &self.frozen {
            if f.resolution != [self.resolution, self.resolution, 1]
                || f.values.len() != (self.resolution as usize).pow(2)
                || f.values
                    .iter()
                    .flatten()
                    .any(|v| !v.is_finite() || *v < 0.0)
            {
                return Err(SceneError::Graph("invalid frozen flame atlas".into()));
            }
        }
        Ok(())
    }
}
impl crate::Params for FlameField {
    fn visit_params(&mut self, prefix: &str, f: &mut dyn FnMut(&str, &mut f32, crate::Bounds)) {
        use crate::params::{join, visit_u32, Bounds};
        self.flame.visit_params(&join(prefix, "flame"), f);
        for axis in 0..2 {
            f(
                &join(prefix, &format!("center[{axis}]")),
                &mut self.center[axis],
                Bounds::new(-10000.0, 10000.0, &[]),
            );
            f(
                &join(prefix, &format!("half[{axis}]")),
                &mut self.half[axis],
                Bounds::new(0.0001, 10000.0, &[]),
            );
        }
        f(
            &join(prefix, "angle"),
            &mut self.angle,
            Bounds::new(-180.0, 180.0, &[]),
        );
        visit_u32(
            &join(prefix, "resolution"),
            &mut self.resolution,
            Bounds::new(32.0, 2048.0, &[]),
            f,
        );
        visit_u32(
            &join(prefix, "points"),
            &mut self.points,
            // Largest f32-representable integer below the validation ceiling.
            // Merely inspecting another parameter must not clamp an authored
            // high-quality source back to the old 16M-point limit.
            Bounds::new(1.0, 67_108_860.0, &[]),
            f,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Params;

    #[test]
    fn high_quality_point_budget_survives_parameter_visits() {
        let mut source = FlameField {
            points: 50_331_648,
            ..Default::default()
        };
        source.validate().unwrap();
        crate::params::validate_params(&source, "source").unwrap();
        source.visit_params("source", &mut |_, _, _| {});
        assert_eq!(source.points, 50_331_648);
        source.visit_params("source", &mut |path, value, _| {
            if path == "source.flame.filter_radius" {
                *value = 0.6;
            }
        });
        assert_eq!(source.points, 50_331_648);
        assert_eq!(source.flame.filter_radius, 0.6);
    }
}
