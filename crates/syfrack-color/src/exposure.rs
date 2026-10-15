use syfrack_core::{luminance, HdrImage};

/// Multiplies the image by 2^ev.
pub fn apply(img: &mut HdrImage, ev: f32) {
    let g = 2f32.powf(ev);
    img.map_rgb(|c| [c[0] * g, c[1] * g, c[2] * g]);
}

/// EV that brings the geometric-mean luminance to `target`. The geometric
/// mean ignores a few blown-out pixels, which arithmetic mean would not.
/// Non-positive and non-finite pixels are ignored; an all-black image returns 0.
pub fn auto_ev(img: &HdrImage, target: f32) -> f32 {
    let mut sum = 0.0f64;
    let mut n = 0.0f64;
    for p in &img.data {
        let l = luminance([p[0], p[1], p[2]]);
        if l.is_finite() && l > 0.0 && p[3].is_finite() && p[3] > 0.0 {
            let weight = p[3].min(1.0) as f64;
            sum += (l as f64).ln() * weight;
            n += weight;
        }
    }
    if n == 0.0 {
        return 0.0;
    }
    let mean = (sum / n).exp() as f32;
    (target / mean.max(1e-6)).log2()
}

#[cfg(test)]
mod tests {
    use super::*;
    use syfrack_core::HdrImage;

    fn flat(v: f32) -> HdrImage {
        let mut img = HdrImage::new(8, 8);
        img.map_rgb(|_| [v, v, v]);
        img
    }

    #[test]
    fn one_ev_doubles() {
        let mut img = flat(0.25);
        apply(&mut img, 1.0);
        assert!((img.get(3, 3)[0] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn auto_ev_hits_the_target_on_a_flat_image() {
        let img = flat(0.09);
        let ev = auto_ev(&img, 0.18);
        assert!((ev - 1.0).abs() < 0.01, "ev {ev}");
        let mut img = img;
        apply(&mut img, ev);
        assert!((img.get(0, 0)[1] - 0.18).abs() < 1e-3);
    }

    #[test]
    fn invisible_rgb_cannot_change_auto_exposure() {
        let mut image = HdrImage::from_data(
            2,
            1,
            vec![[0.18, 0.18, 0.18, 1.0], [10000.0, 1.0, 1.0, 0.0]],
        );
        assert!(auto_ev(&image, 0.18).abs() < 1e-6);
        image.data[0][3] = 0.0;
        assert_eq!(auto_ev(&image, 0.18), 0.0);
    }

    #[test]
    fn auto_ev_uses_geometric_mean_so_one_hot_pixel_barely_counts() {
        let mut img = flat(0.18);
        img.set(0, 0, [1000.0, 1000.0, 1000.0, 1.0]);
        let ev = auto_ev(&img, 0.18);
        assert!(ev.abs() < 0.3, "ev {ev}");
    }
}
