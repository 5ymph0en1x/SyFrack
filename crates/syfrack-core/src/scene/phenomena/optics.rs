use crate::SceneError;
use glam::Vec2;
use serde::{Deserialize, Serialize};

/// Cauchy B in square micrometres. The material's existing IOR is n(550 nm).
/// Paths sample three wavelength bands, retaining the selected band at bounces.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ChromaticDispersion {
    pub cauchy_b: f32,
}
impl Default for ChromaticDispersion {
    fn default() -> Self {
        Self { cauchy_b: 0.004 }
    }
}
impl ChromaticDispersion {
    pub fn validate(&self) -> Result<(), SceneError> {
        if !self.cauchy_b.is_finite() || !(0.0..=0.02).contains(&self.cauchy_b) {
            return Err(SceneError::Graph(
                "Cauchy B must be between 0 and 0.02 square micrometres".into(),
            ));
        }
        Ok(())
    }
    pub fn index(&self, n550: f32, wavelength: f32) -> f32 {
        n550 + self.cauchy_b * (1.0 / (wavelength * 0.001).powi(2) - 1.0 / 0.55f32.powi(2))
    }
}

/// CIE 1931 fit, Wyman, Sloan & Shirley (JCGT 2013), equation 4.
pub fn spectral_xyz(w: f32) -> glam::Vec3 {
    let g = |center: f32, left: f32, right: f32| {
        let t = (w - center) * if w < center { left } else { right };
        (-0.5 * t * t).exp()
    };
    glam::Vec3::new(
        0.362 * g(442.0, 0.0624, 0.0374) + 1.056 * g(599.8, 0.0264, 0.0323)
            - 0.065 * g(501.1, 0.0490, 0.0382),
        0.821 * g(568.8, 0.0213, 0.0247) + 0.286 * g(530.9, 0.0613, 0.0322),
        1.217 * g(437.0, 0.0845, 0.0278) + 0.681 * g(459.0, 0.0385, 0.0725),
    )
}
fn xyz_rgb(v: glam::Vec3) -> glam::Vec3 {
    glam::Vec3::new(
        3.2406 * v.x - 1.5372 * v.y - 0.4986 * v.z,
        -0.9689 * v.x + 1.8758 * v.y + 0.0415 * v.z,
        0.0557 * v.x - 0.2040 * v.y + 1.0570 * v.z,
    )
}

fn mul(a: Vec2, b: Vec2) -> Vec2 {
    Vec2::new(a.x * b.x - a.y * b.y, a.x * b.y + a.y * b.x)
}
fn div(a: Vec2, b: Vec2) -> Vec2 {
    mul(a, Vec2::new(b.x, -b.y)) / b.length_squared().max(1e-30)
}
fn sqrt(z: Vec2) -> Vec2 {
    let r = z.length();
    Vec2::new(
        ((r + z.x) * 0.5).max(0.0).sqrt(),
        ((r - z.x) * 0.5).max(0.0).sqrt().copysign(z.y),
    )
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThinFilm {
    pub thickness_nm: f32,
    pub ior: f32,
    pub substrate_ior: f32,
    pub substrate_extinction: f32,
    pub strength: f32,
    pub thickness_range_nm: f32,
    pub dispersion_b: f32,
}
impl Default for ThinFilm {
    fn default() -> Self {
        Self {
            thickness_nm: 420.0,
            ior: 1.38,
            substrate_ior: 1.5,
            substrate_extinction: 0.0,
            strength: 1.0,
            thickness_range_nm: 250.0,
            dispersion_b: 0.0,
        }
    }
}
impl ThinFilm {
    pub fn validate(&self) -> Result<(), SceneError> {
        if ![
            self.thickness_nm,
            self.ior,
            self.substrate_ior,
            self.substrate_extinction,
            self.strength,
            self.thickness_range_nm,
            self.dispersion_b,
        ]
        .iter()
        .all(|v| v.is_finite())
            || !(0.0..=10000.0).contains(&self.thickness_nm)
            || !(1.0..=4.0).contains(&self.ior)
            || self.substrate_ior <= 0.0
            || self.substrate_extinction < 0.0
            || !(0.0..=1.0).contains(&self.strength)
            || self.thickness_range_nm < 0.0
            || !(0.0..=1.0).contains(&self.dispersion_b)
        {
            return Err(SceneError::Graph("invalid thin-film optics".into()));
        }
        Ok(())
    }
    /// Airy amplitude sum for a single lossless film, unpolarized incident light.
    /// Substrate supports a complex refractive index n+i*k. Units: nanometres.
    pub fn reflectance(&self, cos_incidence: f32, wavelength_nm: f32, thickness_nm: f32) -> f32 {
        self.reflectance_between(
            cos_incidence,
            wavelength_nm,
            thickness_nm,
            1.0,
            [self.substrate_ior, self.substrate_extinction],
        )
    }
    /// Arbitrary transparent incident medium; complex substrate. Complex Snell
    /// angles retain evanescent propagation and frustrated total reflection.
    pub fn reflectance_between(
        &self,
        cosine: f32,
        wavelength: f32,
        thickness: f32,
        n0: f32,
        substrate: [f32; 2],
    ) -> f32 {
        let c0 = cosine.abs().clamp(1e-6, 1.0);
        let s0 = 1.0 - c0 * c0;
        let wavelength = wavelength.max(100.0);
        let n1 = self.ior + self.dispersion_b / (wavelength * 0.001).powi(2);
        let n2 = Vec2::from_array(substrate);
        if n2.y == 0.0 && n0 * n0 * s0 >= n2.x * n2.x {
            return 1.0;
        }
        let c1 = sqrt(Vec2::new(1.0 - n0 * n0 * s0 / (n1 * n1), 0.0));
        let c2 = sqrt(Vec2::X - div(Vec2::new(n0 * n0 * s0, 0.0), mul(n2, n2)));
        let q1 = c1 * n1;
        let q2 = mul(n2, c2);
        let delta = c1 * (4.0 * std::f32::consts::PI * n1 * thickness.max(0.0) / wavelength);
        let phase = Vec2::new(delta.x.cos(), delta.x.sin()) * (-delta.y).exp();
        let mut sum = 0.0;
        for pol in 0..2 {
            let (r01, r12) = if pol == 0 {
                (
                    div(Vec2::new(n0 * c0, 0.0) - q1, Vec2::new(n0 * c0, 0.0) + q1),
                    div(q1 - q2, q1 + q2),
                )
            } else {
                (
                    div(
                        Vec2::new(n1 * c0, 0.0) - c1 * n0,
                        Vec2::new(n1 * c0, 0.0) + c1 * n0,
                    ),
                    div(mul(n2, c1) - c2 * n1, mul(n2, c1) + c2 * n1),
                )
            };
            let amplitude = div(r01 + mul(r12, phase), Vec2::X + mul(mul(r01, r12), phase));
            sum += amplitude.length_squared();
        }
        (0.5 * sum).clamp(0.0, 1.0)
    }
    /// Equal-energy illumination, 41 midpoint bands over 380–790 nm. White
    /// normalization and gamut clipping preserve an energy-bounded RGB BRDF.
    pub fn rgb(&self, cos_incidence: f32, thickness_nm: f32) -> [f32; 3] {
        let mut xyz = glam::Vec3::ZERO;
        let mut white = glam::Vec3::ZERO;
        for i in 0..41 {
            let wavelength = 385.0 + i as f32 * 10.0;
            let response = spectral_xyz(wavelength);
            white += response;
            xyz += response * self.reflectance(cos_incidence, wavelength, thickness_nm);
        }
        (xyz_rgb(xyz) / xyz_rgb(white))
            .clamp(glam::Vec3::ZERO, glam::Vec3::ONE)
            .to_array()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn layer_is_reciprocal_and_handles_evanescent_tunnelling() {
        let film = ThinFilm::default();
        let c0 = 0.8f32;
        let c2 = (1.0 - (1.0 - c0 * c0) / (1.5 * 1.5)).sqrt();
        let forward = film.reflectance_between(c0, 550.0, 420.0, 1.0, [1.5, 0.0]);
        let reverse = film.reflectance_between(c2, 550.0, 420.0, 1.5, [1.0, 0.0]);
        assert!((forward - reverse).abs() < 2e-6);
        let gap = ThinFilm {
            ior: 1.0,
            ..Default::default()
        };
        let zero = gap.reflectance_between(0.5, 550.0, 0.0, 1.5, [1.5, 0.0]);
        let thin = gap.reflectance_between(0.5, 550.0, 100.0, 1.5, [1.5, 0.0]);
        let thick = gap.reflectance_between(0.5, 550.0, 10000.0, 1.5, [1.5, 0.0]);
        assert!(zero < 1e-6);
        assert!(thin > zero && thin < 1.0);
        assert!(thick > 0.9999);
        assert_eq!(
            film.reflectance_between(0.2, 550.0, 420.0, 1.5, [1.0, 0.0]),
            1.0
        );
    }

    #[test]
    fn zero_film_reduces_to_substrate_fresnel() {
        let f = ThinFilm::default();
        for wavelength in [380.0, 550.0, 780.0] {
            assert!((f.reflectance(1.0, wavelength, 0.0) - 0.04).abs() < 1e-6);
        }
    }
    #[test]
    fn film_obeys_energy_bound_and_changes_with_angle_and_thickness() {
        let f = ThinFilm::default();
        for c in [0.0, 0.1, 0.5, 1.0] {
            for d in [0.0, 100.0, 420.0, 900.0] {
                let r = f.reflectance(c, 470.0, d);
                assert!(r.is_finite() && (0.0..=1.0).contains(&r));
            }
        }
        assert_ne!(f.rgb(0.5, 420.0), f.rgb(1.0, 420.0));
        assert_ne!(f.rgb(0.5, 100.0), f.rgb(0.5, 420.0));
    }
}
