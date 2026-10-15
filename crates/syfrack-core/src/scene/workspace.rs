//! A flat image canvas and a spatial scene have different rendering and navigation.
use crate::params::{join, Bounds, Params};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dimension {
    TwoD,
    #[default]
    ThreeD,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Canvas2D {
    pub center: [f32; 2],
    pub zoom: f32,
    pub rotation: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flame: Option<super::Flame>,
}

impl Default for Canvas2D {
    fn default() -> Self {
        Self {
            center: [0.0; 2],
            zoom: 1.0,
            rotation: 0.0,
            flame: None,
        }
    }
}

impl Canvas2D {
    /// Screen-normalized coordinates, with a horizontal span of six world units.
    pub fn point(&self, uv: [f32; 2], aspect: f32) -> glam::Vec2 {
        let p = glam::Vec2::new(uv[0] - 0.5, (0.5 - uv[1]) / aspect) * (6.0 / self.zoom);
        let (s, c) = self.rotation.to_radians().sin_cos();
        glam::Vec2::from_array(self.center) + glam::Vec2::new(c * p.x - s * p.y, s * p.x + c * p.y)
    }
}

impl Params for Canvas2D {
    fn visit_params(&mut self, prefix: &str, f: &mut dyn FnMut(&str, &mut f32, Bounds)) {
        if let Some(flame) = &mut self.flame {
            flame.visit_params(&join(prefix, "flame"), f);
        }
        for (i, v) in self.center.iter_mut().enumerate() {
            f(
                &format!("{prefix}.center[{i}]"),
                v,
                Bounds::new(-100.0, 100.0, &[(-3.0, 3.0)]),
            );
        }
        f(
            &join(prefix, "zoom"),
            &mut self.zoom,
            Bounds::new(0.01, 10000.0, &[(0.25, 64.0)]),
        );
        f(
            &join(prefix, "rotation"),
            &mut self.rotation,
            Bounds::new(-180.0, 180.0, &[(-90.0, 90.0)]),
        );
    }
}
