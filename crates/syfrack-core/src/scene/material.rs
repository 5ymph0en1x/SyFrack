use crate::params::{join, visit_rgb, visit_u32, Bounds, Params, UNIT};
use crate::scene::{FractalTexture, SceneError};
use serde::{Deserialize, Serialize};

pub use super::palette::{PaletteInterpolation, Ramp, MAX_PALETTE_STOPS};

/// How the albedo is modulated by the geometry. `Trap` reads a trap slot of
/// the form graph (0..=3) or a pseudo-slot: 4 = normalised iteration count,
/// 5 = surface normal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Coloring {
    None,
    Trap { slot: u32, ramp: Ramp },
    Orbit { channels: OrbitColoring, ramp: Ramp },
}

/// A signed mix of four orbit measurements, iterations and surface orientation.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OrbitColoring {
    pub traps: [f32; 4],
    pub iterations: f32,
    pub normal: f32,
    /// Signed log remapping. Zero preserves the linear channel mix.
    pub logarithmic: f32,
}
impl Default for OrbitColoring {
    fn default() -> Self {
        Self {
            traps: [1.0, 0.0, 0.0, 0.0],
            iterations: 0.0,
            normal: 0.0,
            logarithmic: 0.0,
        }
    }
}
impl OrbitColoring {
    pub fn value(&self, traps: [f32; 4], iterations: f32, normal_y: f32) -> f32 {
        let v = self
            .traps
            .iter()
            .zip(traps)
            .map(|(w, t)| w * t)
            .sum::<f32>()
            + self.iterations * iterations
            + self.normal * (normal_y * 0.5 + 0.5);
        if self.logarithmic == 0.0 {
            v
        } else {
            v.signum() * (1.0 + self.logarithmic * v.abs()).ln() / (1.0 + self.logarithmic).ln()
        }
    }
}
impl Params for OrbitColoring {
    fn visit_params(&mut self, prefix: &str, f: &mut dyn FnMut(&str, &mut f32, Bounds)) {
        let weight = Bounds::new(-16.0, 16.0, &[(-2.0, 2.0)]);
        for (i, w) in self.traps.iter_mut().enumerate() {
            f(&format!("{prefix}.traps[{i}]"), w, weight);
        }
        f(&join(prefix, "iterations"), &mut self.iterations, weight);
        f(&join(prefix, "normal"), &mut self.normal, weight);
        f(
            &join(prefix, "logarithmic"),
            &mut self.logarithmic,
            Bounds::new(0.0, 100.0, &[(0.0, 20.0)]),
        );
    }
}

/// Trap slots 0..=3, then the pseudo-slots 4 (iterations) and 5 (normal).
pub const COLORING_SLOT: Bounds = Bounds::new(0.0, 5.0, &[(0.0, 3.0)]);

/// Two surface finishes interpolated by a geometry trap, independently of colour.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceVariation {
    pub slot: u32,
    pub roughness: [f32; 2],
    pub metallic: [f32; 2],
}

impl Default for SurfaceVariation {
    fn default() -> Self {
        Self {
            slot: 0,
            roughness: [0.2, 0.4],
            metallic: [1.0, 0.0],
        }
    }
}

impl Params for SurfaceVariation {
    fn visit_params(&mut self, prefix: &str, f: &mut dyn FnMut(&str, &mut f32, Bounds)) {
        visit_u32(&join(prefix, "slot"), &mut self.slot, COLORING_SLOT, f);
        for i in 0..2 {
            f(
                &format!("{prefix}.roughness[{i}]"),
                &mut self.roughness[i],
                UNIT,
            );
            f(
                &format!("{prefix}.metallic[{i}]"),
                &mut self.metallic[i],
                UNIT,
            );
        }
    }
}

impl Coloring {
    /// The slot the shader reads: 0..=5, or 0xffff for no colouring.
    pub fn slot(&self) -> u32 {
        match self {
            Coloring::None => 0xffff,
            Coloring::Trap { slot, .. } => *slot,
            Coloring::Orbit { .. } => 6,
        }
    }

    pub fn ramp(&self) -> Option<&Ramp> {
        match self {
            Coloring::None => None,
            Coloring::Trap { ramp, .. } | Coloring::Orbit { ramp, .. } => Some(ramp),
        }
    }

    pub fn ramp_mut(&mut self) -> Option<&mut Ramp> {
        match self {
            Coloring::None => None,
            Coloring::Trap { ramp, .. } | Coloring::Orbit { ramp, .. } => Some(ramp),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Material {
    pub albedo: [f32; 3],
    pub roughness: f32,
    pub metallic: f32,
    /// Dielectric reflectance at normal incidence (F0). 0.04 is typical.
    pub specular: f32,
    pub coloring: Coloring,
    /// Optional independent fractal graph sampled throughout 3D space.
    #[serde(default)]
    pub texture: Option<FractalTexture>,
    #[serde(default)]
    pub surface: Option<SurfaceVariation>,
    /// Named finishes assigned with MaterialTag nodes; index zero is the global surface.
    #[serde(default)]
    pub finishes: Vec<Finish>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optics: Option<super::Optics>,
}

pub const MAX_FINISHES: usize = 8;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finish {
    pub name: String,
    pub albedo: [f32; 3],
    pub roughness: f32,
    pub metallic: f32,
    pub specular: f32,
    /// Linear emitted radiance, independent of surface colour.
    #[serde(default)]
    pub emission: [f32; 3],
    #[serde(default)]
    pub detail: Option<FinishDetail>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optics: Option<super::Optics>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FinishPattern {
    VeinedStone,
    Patina,
    BrushedMetal,
    Basalt,
    Ripples,
    Rosettes,
    /// Mineral grain, weathering and a sea-level wet/dry transition.
    CoastalRock,
    /// Wind-aligned, filtered sand ripples and mineral grains.
    Sand,
    /// Irregular, wind-streaked mineral dust deposited on upward-facing surfaces.
    Dust,
    /// Aggregate, pores and horizontal formwork marks, filtered in world space.
    Concrete,
    /// Formwork concrete with long rain streaks, mineral bloom and exposed pores.
    WeatheredConcrete,
    /// Porous paving interrupted by smooth, irregular rain puddles.
    WetConcrete,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinishDetail {
    pub pattern: FinishPattern,
    pub seed: u32,
    pub scale: f32,
    pub strength: f32,
    /// Height in scene units, used for filtered shading normals.
    pub bump: f32,
    pub roughness_delta: f32,
    pub tint: [f32; 3],
}

impl Default for FinishDetail {
    fn default() -> Self {
        Self {
            pattern: FinishPattern::VeinedStone,
            seed: 7,
            scale: 2.0,
            strength: 0.25,
            bump: 0.004,
            roughness_delta: 0.15,
            tint: [0.3, 0.27, 0.2],
        }
    }
}

impl Params for FinishDetail {
    fn visit_params(&mut self, prefix: &str, f: &mut dyn FnMut(&str, &mut f32, Bounds)) {
        visit_u32(
            &join(prefix, "seed"),
            &mut self.seed,
            Bounds::new(0.0, 65535.0, &[(0.0, 65535.0)]),
            f,
        );
        f(
            &join(prefix, "scale"),
            &mut self.scale,
            Bounds::new(0.01, 200.0, &[(0.1, 40.0)]),
        );
        f(&join(prefix, "strength"), &mut self.strength, UNIT);
        f(
            &join(prefix, "bump"),
            &mut self.bump,
            Bounds::new(-1.0, 1.0, &[(-0.05, 0.05)]),
        );
        f(
            &join(prefix, "roughness_delta"),
            &mut self.roughness_delta,
            Bounds::new(-1.0, 1.0, &[(-0.2, 0.3)]),
        );
        visit_rgb(&join(prefix, "tint"), &mut self.tint, UNIT, f);
    }
}

impl Default for Finish {
    fn default() -> Self {
        Self {
            name: syfrack_i18n::tr!("Nouveau matériau").into(),
            albedo: [0.5; 3],
            roughness: 0.4,
            metallic: 0.0,
            specular: 0.04,
            emission: [0.0; 3],
            detail: None,
            optics: None,
        }
    }
}

impl Params for Finish {
    fn visit_params(&mut self, prefix: &str, f: &mut dyn FnMut(&str, &mut f32, Bounds)) {
        if let Some(optics) = &mut self.optics {
            optics.visit_params(&join(prefix, "optics"), f);
        }
        visit_rgb(&join(prefix, "albedo"), &mut self.albedo, ALBEDO, f);
        f(&join(prefix, "roughness"), &mut self.roughness, ROUGHNESS);
        f(&join(prefix, "metallic"), &mut self.metallic, UNIT);
        f(&join(prefix, "specular"), &mut self.specular, SPECULAR);
        visit_rgb(
            &join(prefix, "emission"),
            &mut self.emission,
            Bounds::new(0.0, 64.0, &[(0.0, 8.0)]),
            f,
        );
        if let Some(detail) = &mut self.detail {
            detail.visit_params(&join(prefix, "detail"), f);
        }
    }
}

const ALBEDO: Bounds = Bounds::new(0.0, 1.0, &[(0.05, 0.95)]);
const ROUGHNESS: Bounds = Bounds::new(0.0, 1.0, &[(0.05, 0.9)]);
const SPECULAR: Bounds = Bounds::new(0.0, 0.2, &[(0.02, 0.08)]);

impl Material {
    pub fn validate_ramps(&self, prefix: &str) -> Result<(), SceneError> {
        if self.finishes.len() > MAX_FINISHES {
            return Err(SceneError::Graph(syfrack_i18n::tr_format!(
                "au maximum {MAX_FINISHES} matériaux de branche",
                MAX_FINISHES = MAX_FINISHES
            )));
        }
        if let Some(texture) = &self.texture {
            texture.validate(&join(prefix, "texture"))?;
        }
        let p = join(prefix, "coloring");
        match &self.coloring {
            Coloring::None => Ok(()),
            Coloring::Trap { ramp, .. } => ramp.validate(&join(&p, "trap.ramp")),
            Coloring::Orbit { ramp, .. } => ramp.validate(&join(&p, "orbit.ramp")),
        }
    }
}

impl Params for Material {
    fn visit_params(&mut self, prefix: &str, f: &mut dyn FnMut(&str, &mut f32, Bounds)) {
        if let Some(optics) = &mut self.optics {
            optics.visit_params(&join(prefix, "optics"), f);
        }
        visit_rgb(&join(prefix, "albedo"), &mut self.albedo, ALBEDO, f);
        f(&join(prefix, "roughness"), &mut self.roughness, ROUGHNESS);
        f(&join(prefix, "metallic"), &mut self.metallic, UNIT);
        f(&join(prefix, "specular"), &mut self.specular, SPECULAR);
        if let Some(texture) = &mut self.texture {
            texture.visit_params(&join(prefix, "texture"), f);
        }
        if let Some(surface) = &mut self.surface {
            surface.visit_params(&join(prefix, "surface"), f);
        }
        for (i, finish) in self.finishes.iter_mut().enumerate() {
            finish.visit_params(&format!("{prefix}.finishes[{i}]"), f);
        }
        let p = join(prefix, "coloring");
        match &mut self.coloring {
            Coloring::None => {}
            Coloring::Trap { slot, ramp } => {
                visit_u32(&join(&p, "trap.slot"), slot, COLORING_SLOT, f);
                ramp.visit_params(&join(&p, "trap.ramp"), f);
            }
            Coloring::Orbit { channels, ramp } => {
                channels.visit_params(&join(&p, "orbit.channels"), f);
                ramp.visit_params(&join(&p, "orbit.ramp"), f);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Scene;

    fn ramp() -> Ramp {
        Ramp {
            stops: vec![[0.0; 3], [1.0; 3]],
            ..Ramp::default()
        }
    }

    #[test]
    fn slots_and_ramps() {
        assert_eq!(Coloring::None.slot(), 0xffff);
        assert_eq!(
            Coloring::Trap {
                slot: 2,
                ramp: ramp()
            }
            .slot(),
            2
        );
        assert!(Coloring::None.ramp().is_none());
        let mut c = Coloring::Trap {
            slot: 1,
            ramp: ramp(),
        };
        c.ramp_mut().unwrap().stops.push([0.5; 3]);
        assert_eq!(c.ramp().unwrap().stops.len(), 3);
    }

    #[test]
    fn a_trap_coloring_validates_its_slot_and_ramp() {
        let mut s = Scene::example();
        s.material.coloring = Coloring::Trap {
            slot: 5,
            ramp: ramp(),
        };
        s.validate().unwrap();
        s.material.coloring = Coloring::Trap {
            slot: 6,
            ramp: ramp(),
        };
        assert!(s.validate().is_err());
        s.material.coloring = Coloring::Trap {
            slot: 0,
            ramp: Ramp {
                stops: vec![[0.0; 3]],
                ..Ramp::default()
            },
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn surface_variation_validates_and_old_materials_keep_their_finish() {
        let mut s = Scene::example();
        let old = s.to_ron().replace("surface: None,", "");
        assert!(Scene::from_ron(&old).unwrap().material.surface.is_none());
        s.material.surface = Some(SurfaceVariation::default());
        s.set_param("material.surface.roughness[1]", 0.15).unwrap();
        assert!(s.set_param("material.surface.slot", 6.0).is_err());
        assert!(s.set_param("material.surface.metallic[0]", 1.1).is_err());
        assert_eq!(Scene::from_ron(&s.to_ron()).unwrap(), s);
    }
}
