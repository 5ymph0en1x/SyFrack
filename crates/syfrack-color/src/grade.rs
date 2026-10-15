use syfrack_core::{luminance, HdrImage, Zone};

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Shadow / midtone / highlight weights for a linear luminance. They sum to 1.
/// Luminance is compressed with l / (1 + l) so HDR values map into [0, 1).
pub fn zone_weights(l: f32) -> [f32; 3] {
    let lt = l.max(0.0) / (1.0 + l.max(0.0));
    let shadows = 1.0 - smoothstep(0.0, 0.35, lt);
    let highlights = smoothstep(0.45, 0.9, lt);
    let mid = (1.0 - shadows - highlights).max(0.0);
    let sum = shadows + mid + highlights;
    [shadows / sum, mid / sum, highlights / sum]
}

/// Three-zone grading in linear light: per zone, rgb * (1 + gain) + offset,
/// blended by the zone weights.
pub fn apply(img: &mut HdrImage, shadows: &Zone, midtones: &Zone, highlights: &Zone) {
    let zones = [shadows, midtones, highlights];
    img.map_rgb(|c| {
        let w = zone_weights(luminance(c));
        let mut out = c;
        for ch in 0..3 {
            let mut v = 0.0;
            for (z, zone) in zones.iter().enumerate() {
                v += w[z] * (c[ch] * (1.0 + zone.gain) + zone.offset[ch]);
            }
            out[ch] = v.max(0.0);
        }
        out
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use syfrack_core::{HdrImage, Zone};

    fn two_tone() -> HdrImage {
        let mut img = HdrImage::new(2, 1);
        img.set(0, 0, [0.02, 0.02, 0.02, 1.0]);
        img.set(1, 0, [4.0, 4.0, 4.0, 1.0]);
        img
    }

    #[test]
    fn neutral_zones_are_identity() {
        let mut img = two_tone();
        let z = Zone::neutral();
        apply(&mut img, &z, &z, &z);
        assert_eq!(img, two_tone());
    }

    #[test]
    fn weights_sum_to_one_and_partition_the_range() {
        for l in [0.0, 0.01, 0.1, 0.5, 1.0, 10.0, 1000.0] {
            let w = zone_weights(l);
            assert!((w[0] + w[1] + w[2] - 1.0).abs() < 1e-5, "l {l}");
            assert!(w.iter().all(|x| *x >= 0.0));
        }
        assert!(zone_weights(0.0)[0] > 0.99);
        assert!(zone_weights(1000.0)[2] > 0.99);
    }

    #[test]
    fn shadow_gain_lifts_only_the_dark_pixel() {
        let mut img = two_tone();
        let lift = Zone {
            offset: [0.0; 3],
            gain: 1.0,
        };
        apply(&mut img, &lift, &Zone::neutral(), &Zone::neutral());
        assert!(
            (img.get(0, 0)[0] - 0.04).abs() < 2e-3,
            "dark doubled: {:?}",
            img.get(0, 0)
        );
        assert!(
            (img.get(1, 0)[0] - 4.0).abs() < 1e-3,
            "bright untouched: {:?}",
            img.get(1, 0)
        );
    }

    #[test]
    fn highlight_offset_tints_only_the_bright_pixel() {
        let mut img = two_tone();
        let warm = Zone {
            offset: [0.1, 0.0, 0.0],
            gain: 0.0,
        };
        apply(&mut img, &Zone::neutral(), &Zone::neutral(), &warm);
        assert!(img.get(1, 0)[0] > img.get(1, 0)[2]);
        assert!((img.get(0, 0)[0] - img.get(0, 0)[2]).abs() < 1e-3);
    }
}
