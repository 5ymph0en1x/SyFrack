//! Source-independent recipes for adaptive fractal mosaics.
use glam::Vec2;
use serde::{Deserialize, Serialize};

use crate::PlanarFractal;

pub const MAX_MOSAIC_PIXELS: u64 = 16_777_216;
pub const MAX_MOSAIC_TILES: usize = 16_384;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Region {
    pub fn area(self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MosaicTile {
    pub region: Region,
    pub fractal: PlanarFractal,
    pub channel: usize,
    pub feature_range: [f32; 2],
    /// Four uniformly spaced stops, in premultiplied display-linear sRGB/alpha.
    pub palette: [[f32; 4]; 4],
    /// Linear RGBA fitting error on the analysis grid, before supersampling.
    pub error: f32,
}

impl MosaicTile {
    /// Normalised local coordinates, independent of output resolution.
    pub fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        let sample = self
            .fractal
            .sample(Vec2::new(3.0 * (u - 0.5), 3.0 * (v - 0.5)));
        let feature = sample.traps[self.channel];
        let t = ((feature - self.feature_range[0])
            / (self.feature_range[1] - self.feature_range[0]).max(1e-6))
        .clamp(0.0, 1.0)
            * 3.0;
        let i = (t.floor() as usize).min(2);
        let f = t - i as f32;
        let mut p = [0.0; 4];
        for (c, value) in p.iter_mut().enumerate() {
            *value = self.palette[i][c] * (1.0 - f) + self.palette[i + 1][c] * f;
        }
        p
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mosaic {
    pub version: u32,
    pub width: u32,
    pub height: u32,
    pub tiles: Vec<MosaicTile>,
}

impl Mosaic {
    /// Check untrusted recipes, including exact, non-overlapping coverage.
    pub fn validate(&self) -> Result<(), &'static str> {
        use crate::Params;
        let pixels = u64::from(self.width) * u64::from(self.height);
        if self.version != 1 || pixels == 0 || pixels > MAX_MOSAIC_PIXELS {
            return Err("unsupported mosaic version or dimensions");
        }
        if self.tiles.is_empty() || self.tiles.len() > MAX_MOSAIC_TILES {
            return Err("invalid mosaic region count");
        }
        let mut covered = vec![false; pixels as usize];
        for tile in &self.tiles {
            let r = tile.region;
            if r.width == 0
                || r.height == 0
                || u64::from(r.x) + u64::from(r.width) > u64::from(self.width)
                || u64::from(r.y) + u64::from(r.height) > u64::from(self.height)
            {
                return Err("invalid mosaic region bounds");
            }
            let mut valid = true;
            tile.fractal
                .clone()
                .visit_params("fractal", &mut |_, v, b| {
                    valid &= v.is_finite() && *v >= b.min && *v <= b.max;
                });
            if !valid
                || tile.channel >= 4
                || !tile.error.is_finite()
                || tile.error < 0.0
                || tile.feature_range.iter().any(|v| !v.is_finite())
                || tile.feature_range[0] > tile.feature_range[1]
                || tile.palette.iter().any(|p| {
                    p.iter().any(|v| !v.is_finite() || *v < 0.0 || *v > 1.0)
                        || p[..3].iter().any(|v| *v > p[3])
                })
            {
                return Err("invalid mosaic fractal or palette");
            }
            for y in r.y..r.y + r.height {
                for x in r.x..r.x + r.width {
                    let cell = &mut covered
                        [(u64::from(y) * u64::from(self.width) + u64::from(x)) as usize];
                    if *cell {
                        return Err("overlapping mosaic regions");
                    }
                    *cell = true;
                }
            }
        }
        if covered.iter().any(|v| !v) {
            return Err("incomplete mosaic coverage");
        }
        Ok(())
    }
}
