//! Brush geometry lives in original-image coordinates, so a crop or rotation
//! never detaches a painted feature from the source image.
use anyhow::{ensure, Result};
use egui::{Color32, ColorImage};
use serde::{Deserialize, Serialize};
use syfrack_color::source::ImageAdjustments;

pub const MAX_POINTS: usize = 4096;
pub const MAX_STROKES: usize = 256;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrushStroke {
    pub points: Vec<[f32; 2]>,
    /// Radius as a fraction of the original image's shorter edge.
    pub radius: f32,
    pub erase: bool,
}

pub fn validate(strokes: &[BrushStroke]) -> Result<()> {
    ensure!(
        strokes.len() <= MAX_STROKES
            && strokes.iter().map(|s| s.points.len()).sum::<usize>() <= MAX_POINTS,
        "mosaic priority brush is too large"
    );
    ensure!(
        strokes.iter().all(|s| !s.points.is_empty()
            && s.radius.is_finite()
            && s.radius > 0.0
            && s.radius <= 1.0
            && s.points
                .iter()
                .flatten()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))),
        "invalid mosaic priority stroke"
    );
    Ok(())
}

pub struct Geometry {
    crop: [f32; 4],
    rotation: u32,
    pub original_short: f32,
    pub adjusted_size: [f32; 2],
}
impl Geometry {
    pub fn new(raw: [u32; 2], image: &ImageAdjustments) -> Self {
        let [w, h] = raw.map(|n| n as f32);
        let [l, t, r, b] = image.crop;
        let x0 = (l * w).floor().min(w - 1.0);
        let y0 = (t * h).floor().min(h - 1.0);
        let x1 = (r * w).ceil().min(w).max(x0 + 1.0);
        let y1 = (b * h).ceil().min(h).max(y0 + 1.0);
        let size = [x1 - x0, y1 - y0];
        Self {
            crop: [x0 / w, y0 / h, x1 / w, y1 / h],
            rotation: image.rotation,
            original_short: w.min(h),
            adjusted_size: if image.rotation % 2 == 0 {
                size
            } else {
                [size[1], size[0]]
            },
        }
    }
    pub fn to_original(&self, uv: [f32; 2]) -> [f32; 2] {
        let [x, y] = uv;
        let [u, v] = match self.rotation {
            1 => [y, 1.0 - x],
            2 => [1.0 - x, 1.0 - y],
            3 => [1.0 - y, x],
            _ => [x, y],
        };
        let [l, t, r, b] = self.crop;
        [
            (l + u * (r - l)).clamp(0.0, 1.0),
            (t + v * (b - t)).clamp(0.0, 1.0),
        ]
    }
    pub fn to_adjusted(&self, p: [f32; 2]) -> [f32; 2] {
        let [l, t, r, b] = self.crop;
        let [u, v] = [(p[0] - l) / (r - l), (p[1] - t) / (b - t)];
        match self.rotation {
            1 => [1.0 - v, u],
            2 => [1.0 - u, 1.0 - v],
            3 => [v, 1.0 - u],
            _ => [u, v],
        }
    }
}

/// Rasterize continuous segments, rather than mouse events, to avoid holes when
/// dragging quickly. Soft brush edges and erase follow the same ordered strokes.
pub fn rasterize(
    raw: [u32; 2],
    image: &ImageAdjustments,
    size: [u32; 2],
    strokes: &[BrushStroke],
) -> Vec<f32> {
    let [w, h] = size;
    let geometry = Geometry::new(raw, image);
    let scale = (w as f32 / geometry.adjusted_size[0]).min(h as f32 / geometry.adjusted_size[1]);
    let mut mask = vec![0.0f32; w as usize * h as usize];
    for stroke in strokes {
        let points: Vec<_> = stroke
            .points
            .iter()
            .map(|p| {
                let uv = geometry.to_adjusted(*p);
                egui::vec2(uv[0] * w as f32, uv[1] * h as f32)
            })
            .collect();
        let radius = (stroke.radius * geometry.original_short * scale).max(0.5);
        for i in 0..points.len() {
            let a = points[i.saturating_sub(1)];
            let b = points[i];
            let min = a.min(b) - egui::Vec2::splat(radius);
            let max = a.max(b) + egui::Vec2::splat(radius);
            let ab = b - a;
            for y in (min.y.floor().max(0.0) as u32)..(max.y.ceil().clamp(0.0, h as f32) as u32) {
                for x in (min.x.floor().max(0.0) as u32)..(max.x.ceil().clamp(0.0, w as f32) as u32)
                {
                    let ap = egui::vec2(x as f32 + 0.5, y as f32 + 0.5) - a;
                    let t = (ap.dot(ab) / ab.length_sq().max(1e-12)).clamp(0.0, 1.0);
                    let amount =
                        ((radius - (ap - ab * t).length()) / (radius * 0.3)).clamp(0.0, 1.0);
                    let value = &mut mask[(y * w + x) as usize];
                    *value = if stroke.erase {
                        value.min(1.0 - amount)
                    } else {
                        value.max(amount)
                    };
                }
            }
        }
    }
    mask
}

pub fn overlay(mask: &[f32], size: [usize; 2]) -> ColorImage {
    ColorImage::new(
        size,
        mask.iter()
            .map(|v| Color32::from_rgba_unmultiplied(255, 190, 25, (*v * 100.0) as u8))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn brush_follows_crop_rotation_and_erase_has_no_drag_gaps() {
        for rotation in 0..4 {
            let image = ImageAdjustments {
                crop: [0.25, 0.1, 0.9, 0.8],
                rotation,
                ..Default::default()
            };
            let g = Geometry::new([200, 100], &image);
            for uv in [[0.1, 0.2], [0.5, 0.5], [0.9, 0.7]] {
                let roundtrip = g.to_adjusted(g.to_original(uv));
                assert!((roundtrip[0] - uv[0]).abs() < 1e-6 && (roundtrip[1] - uv[1]).abs() < 1e-6);
            }
            let mut strokes = vec![BrushStroke {
                points: vec![g.to_original([0.1, 0.5]), g.to_original([0.9, 0.5])],
                radius: 0.08,
                erase: false,
            }];
            let mask = rasterize([200, 100], &image, [100, 100], &strokes);
            for x in 12..88 {
                assert!(mask[50 * 100 + x] > 0.9);
            }
            strokes.push(BrushStroke {
                points: vec![g.to_original([0.5, 0.5])],
                radius: 0.08,
                erase: true,
            });
            let mask = rasterize([200, 100], &image, [100, 100], &strokes);
            assert_eq!(mask[50 * 100 + 50], 0.0);
            assert!(mask[50 * 100 + 20] > 0.9);
        }
    }
}
