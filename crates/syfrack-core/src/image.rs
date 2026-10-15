/// Linear, scene-referred RGBA image, f32 per channel.
/// Pixel (x, y) lives at `data[y * width + x]`; y grows downward.
#[derive(Clone, Debug, PartialEq)]
pub struct HdrImage {
    pub width: u32,
    pub height: u32,
    /// Scene/display-linear RGB with straight (unassociated) linear alpha.
    /// GPU accumulation's sample counter is converted before constructing this image.
    pub data: Vec<[f32; 4]>,
}

impl HdrImage {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![[0.0, 0.0, 0.0, 1.0]; (width as usize) * (height as usize)],
        }
    }

    pub fn from_data(width: u32, height: u32, data: Vec<[f32; 4]>) -> Self {
        assert_eq!(
            data.len(),
            (width as usize) * (height as usize),
            "pixel count mismatch"
        );
        Self {
            width,
            height,
            data,
        }
    }

    #[inline]
    pub fn index(&self, x: u32, y: u32) -> usize {
        debug_assert!(x < self.width && y < self.height);
        (y as usize) * (self.width as usize) + (x as usize)
    }

    #[inline]
    pub fn get(&self, x: u32, y: u32) -> [f32; 4] {
        self.data[self.index(x, y)]
    }

    #[inline]
    pub fn set(&mut self, x: u32, y: u32, v: [f32; 4]) {
        let i = self.index(x, y);
        self.data[i] = v;
    }

    pub fn pixel_count(&self) -> usize {
        self.data.len()
    }

    /// Applies `f` to the RGB part of every pixel, keeping alpha.
    pub fn map_rgb(&mut self, f: impl Fn([f32; 3]) -> [f32; 3]) {
        for p in &mut self.data {
            let c = f([p[0], p[1], p[2]]);
            p[0] = c[0];
            p[1] = c[1];
            p[2] = c[2];
        }
    }
}

/// Rec. 709 luminance of a linear RGB triple.
#[inline]
pub fn luminance(rgb: [f32; 3]) -> f32 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_image_is_black_with_alpha_one() {
        let img = HdrImage::new(3, 2);
        assert_eq!(img.data.len(), 6);
        assert_eq!(img.get(2, 1), [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn set_then_get_roundtrips() {
        let mut img = HdrImage::new(4, 4);
        img.set(1, 2, [0.5, 0.25, 2.0, 1.0]);
        assert_eq!(img.get(1, 2), [0.5, 0.25, 2.0, 1.0]);
        assert_eq!(img.get(2, 1), [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn map_rgb_leaves_alpha_alone() {
        let mut img = HdrImage::new(2, 1);
        img.set(0, 0, [1.0, 2.0, 3.0, 0.5]);
        img.map_rgb(|c| [c[0] * 2.0, c[1] * 2.0, c[2] * 2.0]);
        assert_eq!(img.get(0, 0), [2.0, 4.0, 6.0, 0.5]);
    }

    #[test]
    fn luminance_is_rec709() {
        assert!((luminance([1.0, 1.0, 1.0]) - 1.0).abs() < 1e-6);
        assert!((luminance([0.0, 1.0, 0.0]) - 0.7152).abs() < 1e-6);
    }
}
