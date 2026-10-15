//! A volumetric material texture evaluated by the same graph engine as the form.
use crate::params::{join, visit_u32, visit_vec3, Bounds, Params, POSITION, UNIT};
use crate::scene::{graph, Graph, OrbitColoring, Ramp, SceneError};
use glam::{EulerRot, Quat, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum TextureChannel {
    Distance,
    Trap { slot: u32 },
    Iterations,
    Orbit { channels: OrbitColoring },
}

/// A staggered pattern of semicircular fans on the world XZ plane.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FanMapping {
    pub period: [f32; 2],
    pub rays: u32,
}

impl Default for FanMapping {
    fn default() -> Self {
        Self {
            period: [2.6, 2.0],
            rays: 18,
        }
    }
}

/// Select a region using a geometry trap; unwritten traps have value one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextureMask {
    pub slot: u32,
    pub value: f32,
    pub width: f32,
}

impl Default for TextureMask {
    fn default() -> Self {
        Self {
            slot: 1,
            value: 0.0,
            width: 0.05,
        }
    }
}

impl TextureChannel {
    /// 0..=3 traps, 4 iterations, 6 signed distance (5 is reserved for normals).
    pub fn slot(self) -> u32 {
        match self {
            Self::Trap { slot } => slot,
            Self::Iterations => 4,
            Self::Distance => 6,
            Self::Orbit { .. } => 7,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FractalTexture {
    pub graph: Graph,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planar: Option<crate::PlanarFractal>,
    pub channel: TextureChannel,
    pub scale: f32,
    pub offset: Vec3,
    /// Euler XYZ, in degrees, like the form's Rotation node.
    pub rotation: Vec3,
    pub octaves: u32,
    pub contrast: f32,
    pub ramp: Ramp,
    /// Blend between the original albedo and albedo multiplied by the ramp.
    pub strength: f32,
    /// Blend between the material's roughness and the texture's scalar value.
    pub roughness: f32,
    /// Shading relief only; the distance field and silhouette stay intact.
    pub bump: f32,
    /// Parallax depth in texture coordinates. Zero keeps the original sampling.
    #[serde(default)]
    pub parallax: f32,
    #[serde(default = "default_parallax_steps")]
    pub parallax_steps: u32,
    #[serde(default)]
    pub fan: Option<FanMapping>,
    #[serde(default)]
    pub mask: Option<TextureMask>,
}

fn default_parallax_steps() -> u32 {
    8
}

impl Default for FractalTexture {
    fn default() -> Self {
        Self {
            graph: Graph::bundled("mandelbox").expect("bundled mandelbox"),
            planar: None,
            channel: TextureChannel::Trap { slot: 0 },
            scale: 3.0,
            offset: Vec3::new(0.37, 0.19, 0.53),
            rotation: Vec3::ZERO,
            octaves: 2,
            contrast: 2.0,
            ramp: Ramp {
                stops: vec![[0.12, 0.22, 0.27], [0.86, 0.6, 0.22]],
                ..Ramp::default()
            },
            strength: 1.0,
            roughness: 0.35,
            bump: 0.15,
            parallax: 0.0,
            parallax_steps: default_parallax_steps(),
            fan: None,
            mask: None,
        }
    }
}

pub const TEXTURE_SCALE: Bounds = Bounds::new(0.01, 100.0, &[(0.25, 12.0)]);
pub const TEXTURE_OCTAVES: Bounds = Bounds::new(1.0, 4.0, &[(1.0, 3.0)]);
pub const TEXTURE_CONTRAST: Bounds = Bounds::new(0.1, 10.0, &[(0.5, 4.0)]);
pub const PARALLAX_STEPS: Bounds = Bounds::new(4.0, 32.0, &[(4.0, 16.0)]);

impl FractalTexture {
    pub fn iteration_budget(&self) -> u32 {
        self.planar
            .as_ref()
            .map_or_else(|| self.graph.iteration_product(), |p| p.iterations)
    }
    pub fn validate(&self, prefix: &str) -> Result<(), SceneError> {
        self.graph
            .validate()
            .map_err(|e| SceneError::Graph(format!("{prefix}: {e}")))?;
        self.ramp.validate(&join(prefix, "ramp"))?;
        crate::params::validate_params(self, prefix)
    }

    /// CPU counterpart of the shader's texture_value, useful for inspection.
    pub fn value_at(&self, p: Vec3) -> f32 {
        let a = self.rotation * (std::f32::consts::PI / 180.0);
        let mut q = Quat::from_euler(EulerRot::XYZ, a.x, a.y, a.z) * p + self.offset;
        let mut polar = None;
        if let Some(fan) = &self.fan {
            let row = (q.z / fan.period[1]).floor();
            let x = (q.x + row.rem_euclid(2.0) * fan.period[0] * 0.5).rem_euclid(fan.period[0])
                - fan.period[0] * 0.5;
            let z = q.z.rem_euclid(fan.period[1]) + 0.03;
            let angle = x.atan2(z);
            let radius = x.hypot(z);
            polar = Some((angle, radius, fan.rays));
            q = Vec3::new(angle * fan.rays as f32 / std::f32::consts::PI, q.y, radius);
            q *= self.scale;
        } else {
            // Preserve the original placement convention for existing scenes.
            q = Quat::from_euler(EulerRot::XYZ, a.x, a.y, a.z) * (p * self.scale) + self.offset;
        }
        let mut total = 0.0;
        let mut weight = 1.0;
        let mut weights = 0.0;
        for _ in 0..self.octaves {
            let s = self
                .planar
                .as_ref()
                .map_or_else(|| self.graph.evaluate(q), |p| p.sample(q.truncate()));
            let v = match self.channel {
                TextureChannel::Trap { slot } => s.traps[slot.min(3) as usize],
                TextureChannel::Iterations => {
                    s.iterations as f32 / self.iteration_budget().max(1) as f32
                }
                TextureChannel::Distance => 0.5 + 0.5 * s.distance / (1.0 + s.distance.abs()),
                TextureChannel::Orbit { channels } => channels.value(
                    s.traps,
                    s.iterations as f32 / self.iteration_budget().max(1) as f32,
                    0.0,
                ),
            };
            // Invalid arithmetic in a user graph must not poison the material.
            total += if v.is_finite() {
                if matches!(self.channel, TextureChannel::Orbit { .. }) {
                    v
                } else {
                    v.clamp(0.0, 1.0)
                }
            } else {
                0.5
            } * weight;
            weights += weight;
            q = q * 2.0 + Vec3::new(0.73, 1.17, 0.91);
            weight *= 0.5;
        }
        let fractal = total / weights;
        let value = if let Some((angle, radius, rays)) = polar {
            let spoke = (1.0 - (angle * rays as f32 + 0.16 * (fractal - 0.5)).sin().abs()).powi(4);
            let arc = (1.0
                - (radius * self.scale * std::f32::consts::PI + 0.35 * fractal)
                    .sin()
                    .abs())
            .powi(6);
            0.16 + 0.28 * fractal + 0.42 * spoke + 0.22 * arc
        } else {
            fractal
        };
        let value = (value - 0.5) * self.contrast + 0.5;
        if matches!(self.channel, TextureChannel::Orbit { .. }) {
            value
        } else {
            value.clamp(0.0, 1.0)
        }
    }
}

impl Params for FractalTexture {
    fn visit_params(&mut self, prefix: &str, f: &mut dyn FnMut(&str, &mut f32, Bounds)) {
        if let Some(planar) = &mut self.planar {
            planar.visit_params(&join(prefix, "planar"), f);
        }
        if let TextureChannel::Orbit { channels } = &mut self.channel {
            channels.visit_params(&join(prefix, "orbit"), f);
        }

        self.graph.visit_params(&join(prefix, "graph"), f);
        if let TextureChannel::Trap { slot } = &mut self.channel {
            visit_u32(&join(prefix, "channel.trap.slot"), slot, graph::SLOT, f);
        }
        f(&join(prefix, "scale"), &mut self.scale, TEXTURE_SCALE);
        visit_vec3(&join(prefix, "offset"), &mut self.offset, POSITION, f);
        visit_vec3(
            &join(prefix, "rotation"),
            &mut self.rotation,
            graph::ANGLE_DEG,
            f,
        );
        visit_u32(
            &join(prefix, "octaves"),
            &mut self.octaves,
            TEXTURE_OCTAVES,
            f,
        );
        f(
            &join(prefix, "contrast"),
            &mut self.contrast,
            TEXTURE_CONTRAST,
        );
        self.ramp.visit_params(&join(prefix, "ramp"), f);
        f(&join(prefix, "strength"), &mut self.strength, UNIT);
        f(&join(prefix, "roughness"), &mut self.roughness, UNIT);
        f(&join(prefix, "bump"), &mut self.bump, UNIT);
        f(&join(prefix, "parallax"), &mut self.parallax, UNIT);
        visit_u32(
            &join(prefix, "parallax_steps"),
            &mut self.parallax_steps,
            PARALLAX_STEPS,
            f,
        );
        if let Some(fan) = &mut self.fan {
            for i in 0..2 {
                f(
                    &format!("{prefix}.fan.period[{i}]"),
                    &mut fan.period[i],
                    graph::PERIOD,
                );
            }
            visit_u32(
                &join(prefix, "fan.rays"),
                &mut fan.rays,
                Bounds::new(4.0, 64.0, &[(8.0, 24.0)]),
                f,
            );
        }
        if let Some(mask) = &mut self.mask {
            visit_u32(&join(prefix, "mask.slot"), &mut mask.slot, graph::SLOT, f);
            f(&join(prefix, "mask.value"), &mut mask.value, UNIT);
            f(
                &join(prefix, "mask.width"),
                &mut mask.width,
                Bounds::new(0.001, 1.0, &[(0.01, 0.1)]),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Scene;

    #[test]
    fn textures_round_trip_and_older_scenes_stay_untextured() {
        let mut scene = Scene::example();
        let old = scene.to_ron();
        scene.material.texture = Some(FractalTexture::default());
        scene.validate().unwrap();
        assert_eq!(Scene::from_ron(&scene.to_ron()).unwrap(), scene);
        let legacy_texture = scene
            .to_ron()
            .replace("parallax: 0.0,", "")
            .replace("parallax_steps: 8,", "");
        let restored = Scene::from_ron(&legacy_texture).unwrap();
        assert_eq!(restored.material.texture.as_ref().unwrap().parallax, 0.0);
        assert_eq!(
            restored.material.texture.as_ref().unwrap().parallax_steps,
            8
        );
        let old = old.replace("texture: None,", "");
        assert!(Scene::from_ron(&old).unwrap().material.texture.is_none());
    }

    #[test]
    fn validates_the_texture_graph_ramp_and_parameters() {
        let mut scene = Scene::example();
        scene.material.texture = Some(FractalTexture::default());
        scene.set_param("material.texture.scale", 7.0).unwrap();
        scene.set_param("material.texture.offset.x", -2.0).unwrap();
        assert!(scene.set_param("material.texture.octaves", 5.0).is_err());
        assert!(scene
            .set_param("material.texture.parallax_steps", 33.0)
            .is_err());
        scene.set_param("material.texture.parallax", 0.2).unwrap();
        assert!(scene
            .set_param("material.texture.channel.trap.slot", 4.0)
            .is_err());
        scene.material.texture.as_mut().unwrap().graph.links.clear();
        assert!(scene.validate_editable().is_err());
        scene.material.texture = Some(FractalTexture::default());
        scene.material.texture.as_mut().unwrap().ramp.stops.clear();
        assert!(scene.validate().is_err());
    }

    #[test]
    fn fractal_values_are_spatial_deterministic_and_bounded() {
        let t = FractalTexture::default();
        let values: Vec<_> = (0..40)
            .map(|i| t.value_at(Vec3::new(i as f32 * 0.047, 0.31, -0.2)))
            .collect();
        assert!(values
            .iter()
            .all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
        let (min, max) = values
            .iter()
            .fold((1.0f32, 0.0f32), |(lo, hi), &v| (lo.min(v), hi.max(v)));
        assert!(max - min > 0.05, "texture must vary: {min}..{max}");
        assert_eq!(t.value_at(Vec3::ONE), t.value_at(Vec3::ONE));
    }

    #[test]
    fn fans_repeat_in_staggered_rows_and_keep_fractal_detail() {
        let mut t = FractalTexture::default();
        t.fan = Some(FanMapping::default());
        t.rotation = Vec3::ZERO;
        t.offset = Vec3::ZERO;
        t.scale = 1.0;
        t.contrast = 1.0;
        let p = Vec3::new(0.37, 0.0, 0.71);
        assert!((t.value_at(p) - t.value_at(p + Vec3::new(2.6, 0.0, 0.0))).abs() < 1e-5);
        assert!((t.value_at(p) - t.value_at(p + Vec3::new(0.0, 0.0, 4.0))).abs() < 1e-5);
        let values: Vec<_> = (0..100)
            .map(|i| t.value_at(Vec3::new(i as f32 * 0.026, 0.0, 0.8)))
            .collect();
        assert!(values
            .iter()
            .all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
        assert!(
            values.iter().copied().fold(0.0f32, f32::max)
                - values.iter().copied().fold(1.0f32, f32::min)
                > 0.15
        );
        let mut scene = Scene::example();
        t.mask = Some(TextureMask::default());
        scene.material.texture = Some(t);
        scene.validate().unwrap();
        assert!(scene
            .set_param("material.texture.fan.period[0]", 0.0)
            .is_err());
        assert!(scene.set_param("material.texture.mask.slot", 4.0).is_err());
        assert_eq!(Scene::from_ron(&scene.to_ron()).unwrap(), scene);
    }
}
