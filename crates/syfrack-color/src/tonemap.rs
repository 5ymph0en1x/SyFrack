use syfrack_core::{luminance, HdrImage, Tonemap};

// AgX (Troy Sobotka), polynomial approximation by Benjamin Wrensch.
// Row-major: v' = M * v. (The GLSL reference lists these as column-major mat3 literals.)
pub(crate) const AGX_IN: [[f32; 3]; 3] = [
    [0.842_479_06, 0.078_433_6, 0.079_223_745],
    [0.042_328_24, 0.878_468_64, 0.079_166_13],
    [0.042_375_655, 0.078_433_6, 0.879_143],
];
pub(crate) const AGX_OUT: [[f32; 3]; 3] = [
    [1.196_879, -0.098_020_88, -0.099_029_74],
    [-0.052_896_85, 1.151_903_1, -0.098_961_18],
    [-0.052_971_635, -0.098_043_45, 1.151_073_7],
];
pub(crate) const MIN_EV: f32 = -12.473_934;
pub(crate) const MAX_EV: f32 = 4.026_069;

fn mat_mul(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

fn agx_sigmoid(x: f32) -> f32 {
    let x2 = x * x;
    let x4 = x2 * x2;
    15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x
        - 0.00232
}

/// AgX filmic tone mapping. Input scene-linear, output display-linear [0, 1].
/// The look `pow(v * slope + offset, power)` runs in AgX log space.
pub fn agx(c: [f32; 3], slope: f32, power: f32, offset: f32) -> [f32; 3] {
    let mut v = mat_mul(&AGX_IN, c.map(|x| x.max(1e-10)));
    v = v.map(|x| ((x.log2() - MIN_EV) / (MAX_EV - MIN_EV)).clamp(0.0, 1.0));
    v = v.map(agx_sigmoid);
    v = v.map(|x| (x * slope + offset).max(0.0).powf(power));
    v = mat_mul(&AGX_OUT, v);
    // AgX output is display-encoded (approximately gamma 2.2); return display-linear
    // so the encoder applies the real sRGB / P3 transfer once.
    v.map(|x| x.clamp(0.0, 1.0).powf(2.2))
}

/// Extended Reinhard on luminance; `white` is the scene value that maps to 1.
pub fn reinhard(c: [f32; 3], white: f32) -> [f32; 3] {
    let l = luminance(c).max(1e-6);
    let ln = l * (1.0 + l / (white * white)) / (1.0 + l);
    let k = ln / l;
    c.map(|x| (x * k).clamp(0.0, 1.0))
}

pub fn apply(img: &mut HdrImage, tm: &Tonemap) {
    match *tm {
        Tonemap::Direct => img.map_rgb(|c| c.map(|v| v.clamp(0.0, 1.0))),
        Tonemap::Agx {
            slope,
            power,
            offset,
        } => img.map_rgb(|c| agx(c, slope, power, offset)),
        Tonemap::Reinhard { white } => img.map_rgb(|c| reinhard(c, white)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syfrack_core::luminance;

    #[test]
    fn agx_keeps_greys_neutral() {
        for v in [0.001, 0.01, 0.18, 1.0, 10.0, 100.0] {
            let o = agx([v, v, v], 1.0, 1.0, 0.0);
            assert!(
                (o[0] - o[1]).abs() < 0.01 && (o[1] - o[2]).abs() < 0.01,
                "{v}: {o:?}"
            );
        }
    }

    #[test]
    fn agx_is_monotone_and_bounded() {
        let mut prev = -1.0;
        for i in 0..200 {
            let v = 10f32.powf(-4.0 + i as f32 * 0.03);
            let o = luminance(agx([v, v, v], 1.0, 1.0, 0.0));
            assert!(o >= prev - 1e-6, "not monotone at {v}");
            assert!((0.0..=1.0).contains(&o));
            prev = o;
        }
    }

    #[test]
    fn agx_mid_grey_lands_near_display_mid() {
        let o = luminance(agx([0.18; 3], 1.0, 1.0, 0.0));
        assert!(o > 0.12 && o < 0.30, "0.18 -> {o}");
    }

    #[test]
    fn agx_power_raises_contrast() {
        let soft = luminance(agx([0.02; 3], 1.0, 1.0, 0.0));
        let hard = luminance(agx([0.02; 3], 1.0, 1.4, 0.0));
        assert!(hard < soft);
    }

    #[test]
    fn reinhard_halves_unit_white() {
        let o = reinhard([1.0; 3], 1e6);
        assert!((o[0] - 0.5).abs() < 1e-3);
        let w = reinhard([4.0; 3], 4.0);
        assert!((w[0] - 1.0).abs() < 1e-3, "white point maps to 1");
    }
}
