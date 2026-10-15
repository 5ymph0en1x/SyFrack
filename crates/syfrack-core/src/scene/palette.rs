//! Linear-light palettes shared by surface and volumetric fractal textures.
use crate::params::{visit_rgb, Bounds, Params, UNIT};
use crate::SceneError;
use serde::{Deserialize, Serialize};

pub const MAX_PALETTE_STOPS: usize = 16;

/// Stable caption keys; localize with `syfrack_i18n::text` when displaying them.
pub const NEON_NAMES: [&str; 5] = ["Cyberpunk", "Arcade", "Aurore", "Plasma", "Acide"];
/// Artist-facing presets are authored as display sRGB, then stored in linear light.
pub fn neon_palette(index: usize) -> Ramp {
    let hex: &[u32] = match index {
        1 => &[
            0x030313, 0x2200ff, 0x00aaff, 0x00fff0, 0xeeffff, 0xffed00, 0xff2a00, 0xff007e,
            0xef00ff,
        ],
        2 => &[
            0x020516, 0x1600a8, 0x003cff, 0x00caff, 0x00ffca, 0xf0fff6, 0x90ff00, 0x00a881,
            0x1900dd,
        ],
        3 => &[
            0x030216, 0x3700ff, 0xba00ff, 0xff00be, 0xff1155, 0xff7900, 0xffee66, 0xffffff,
            0x00eaff,
        ],
        4 => &[
            0x020714, 0x0900cc, 0x003dff, 0x00cfff, 0x00ffad, 0xccff00, 0xfaffbb, 0xff5900,
            0xff00c8,
        ],
        _ => &[
            0x020411, 0x3813f5, 0x00d9ff, 0xd8fcff, 0xf5ff00, 0xff4510, 0xff005b, 0xef00ff,
            0x4c00ea,
        ],
    };
    let linear = |v: u32| {
        let s = v as f32 / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    Ramp {
        stops: hex
            .iter()
            .map(|c| {
                [
                    linear((c >> 16) & 255),
                    linear((c >> 8) & 255),
                    linear(c & 255),
                ]
            })
            .collect(),
        cyclic: true,
        frequency: 13.0,
        phase: 0.17,
        interpolation: PaletteInterpolation::Smooth,
        ..Ramp::default()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaletteInterpolation {
    #[default]
    Linear,
    Smooth,
}

fn one() -> f32 {
    1.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ramp {
    pub stops: Vec<[f32; 3]>,
    /// Empty means evenly spaced. Otherwise one strictly increasing position per stop.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub positions: Vec<f32>,
    #[serde(default)]
    pub cyclic: bool,
    #[serde(default = "one")]
    pub frequency: f32,
    #[serde(default)]
    pub phase: f32,
    #[serde(default = "one")]
    pub gamma: f32,
    #[serde(default)]
    pub interpolation: PaletteInterpolation,
}

impl Default for Ramp {
    fn default() -> Self {
        Self {
            stops: vec![[0.0; 3], [1.0; 3]],
            positions: Vec::new(),
            cyclic: false,
            frequency: 1.0,
            phase: 0.0,
            gamma: 1.0,
            interpolation: PaletteInterpolation::Linear,
        }
    }
}

impl Ramp {
    pub fn position(&self, i: usize) -> f32 {
        self.positions.get(i).copied().unwrap_or_else(|| {
            i as f32
                / if self.cyclic {
                    self.stops.len()
                } else {
                    self.stops.len().saturating_sub(1).max(1)
                } as f32
        })
    }

    /// Input coordinates may be negative or exceed one. Cyclic palettes close the seam.
    pub fn sample(&self, t: f32) -> [f32; 3] {
        let n = self.stops.len();
        if n == 0 {
            return [0.0; 3];
        }
        if n == 1 {
            return self.stops[0];
        }
        let value = t * self.frequency + self.phase;
        let value = if self.cyclic {
            value.rem_euclid(1.0)
        } else {
            value.clamp(0.0, 1.0)
        };
        let value = if self.gamma == 1.0 {
            value
        } else {
            value.powf(self.gamma)
        };
        // Keep the original arithmetic for legacy palettes.
        if self.positions.is_empty() && !self.cyclic {
            let x = value * (n - 1) as f32;
            let i = (x.floor() as usize).min(n - 2);
            return self.mix(i, i + 1, x - i as f32);
        }
        let first = self.position(0);
        let last = self.position(n - 1);
        if self.cyclic && (value < first || value >= last) {
            let x = if value < first { value + 1.0 } else { value };
            return self.mix(n - 1, 0, (x - last) / (1.0 + first - last).max(1e-8));
        }
        if value <= first {
            return self.stops[0];
        }
        if value >= last {
            return self.stops[n - 1];
        }
        let i = (0..n - 1)
            .find(|&i| value < self.position(i + 1))
            .unwrap_or(n - 2);
        self.mix(
            i,
            i + 1,
            (value - self.position(i)) / (self.position(i + 1) - self.position(i)),
        )
    }

    fn mix(&self, a: usize, b: usize, f: f32) -> [f32; 3] {
        let f = f.clamp(0.0, 1.0);
        let f = match self.interpolation {
            PaletteInterpolation::Linear => f,
            PaletteInterpolation::Smooth => f * f * (3.0 - 2.0 * f),
        };
        std::array::from_fn(|c| self.stops[a][c] + (self.stops[b][c] - self.stops[a][c]) * f)
    }

    pub fn validate(&self, path: &str) -> Result<(), SceneError> {
        if !(2..=MAX_PALETTE_STOPS).contains(&self.stops.len()) {
            return Err(SceneError::InvalidRamp {
                path: path.into(),
                stops: self.stops.len(),
            });
        }
        if !self.positions.is_empty()
            && (self.positions.len() != self.stops.len()
                || self
                    .positions
                    .iter()
                    .any(|p| !p.is_finite() || !(0.0..=1.0).contains(p))
                || self.positions.windows(2).any(|p| p[0] >= p[1])
                || (self.cyclic && self.positions.last().is_some_and(|p| *p >= 1.0)))
        {
            return Err(SceneError::Graph(format!(
                "{path}: palette positions must be strictly increasing in [0, 1], one per colour"
            )));
        }
        crate::params::validate_params(self, path)
    }
}

impl Params for Ramp {
    fn visit_params(&mut self, prefix: &str, f: &mut dyn FnMut(&str, &mut f32, Bounds)) {
        for (i, stop) in self.stops.iter_mut().enumerate() {
            visit_rgb(&format!("{prefix}[{i}]"), stop, UNIT, f);
        }
        // Position order is validated as a whole by the editor, not through individual knobs.
        f(
            &format!("{prefix}.frequency"),
            &mut self.frequency,
            Bounds::new(0.01, 256.0, &[(0.25, 32.0)]),
        );
        f(
            &format!("{prefix}.phase"),
            &mut self.phase,
            Bounds::new(-256.0, 256.0, &[(-1.0, 1.0)]),
        );
        f(
            &format!("{prefix}.gamma"),
            &mut self.gamma,
            Bounds::new(0.05, 20.0, &[(0.25, 4.0)]),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_palette_and_cyclic_seam() {
        let legacy: Ramp = ron::from_str("(stops: [(0, 0, 0), (1, 1, 1)])").unwrap();
        assert_eq!(legacy.sample(0.25), [0.25; 3]);
        assert_eq!(legacy.sample(-1.0), [0.0; 3]);
        let cyclic = Ramp {
            cyclic: true,
            stops: vec![[1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            ..Ramp::default()
        };
        assert_eq!(cyclic.sample(0.25), [0.5, 0.0, 0.5]);
        assert_eq!(cyclic.sample(-0.25), cyclic.sample(0.75));
        assert_eq!(cyclic.sample(1.0), cyclic.sample(0.0));
        assert!((cyclic.sample(1.0 - 1e-6)[0] - cyclic.sample(1e-6)[0]).abs() < 1e-5);
    }
    #[test]
    fn positioned_palette_mapping_and_validation() {
        let mut r = Ramp {
            positions: vec![0.1, 0.2, 0.8],
            cyclic: true,
            stops: vec![[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            ..Ramp::default()
        };
        assert_eq!(r.sample(0.2), [0.0, 1.0, 0.0]);
        assert!((r.sample(0.95)[0] - 0.5).abs() < 1e-6);
        r.frequency = 2.0;
        r.phase = -0.1;
        assert!((r.sample(0.15)[1] - 1.0).abs() < 1e-6);
        r.validate("palette").unwrap();
        r.positions[1] = 0.1;
        assert!(r.validate("palette").is_err());
        r.positions.clear();
        r.frequency = f32::NAN;
        assert!(r.validate("palette").is_err());
    }
}
