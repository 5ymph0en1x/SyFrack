use super::FieldId;
use crate::SceneError;
use glam::Vec3;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Simulation {
    pub name: String,
    pub model: SimulationModel,
    pub steps: u32,
    pub dt: f32,
    pub seed: u64,
    pub seeds: Vec<SeedBlob>,
    pub potential: Option<FieldId>,
    pub source: Option<FieldId>,
    pub mask: Option<FieldId>,
    pub frozen: Option<FrozenSnapshot>,
}
impl Default for Simulation {
    fn default() -> Self {
        Self {
            name: "Morphogenèse".into(),
            model: SimulationModel::GrayScott {
                diffusion: [0.0016, 0.0008],
                feed: 0.035,
                kill: 0.062,
            },
            steps: 256,
            dt: 0.2,
            seed: 7,
            seeds: vec![SeedBlob::default()],
            potential: None,
            source: None,
            mask: None,
            frozen: None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenSnapshot {
    pub resolution: [u32; 3],
    #[serde(with = "snapshot_values")]
    pub values: std::sync::Arc<[[f32; 4]]>,
}
impl PartialEq for FrozenSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.resolution == other.resolution
            && (std::sync::Arc::ptr_eq(&self.values, &other.values) || self.values == other.values)
    }
}
mod snapshot_values {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    pub fn serialize<S: Serializer>(
        values: &std::sync::Arc<[[f32; 4]]>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        values.as_ref().serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<std::sync::Arc<[[f32; 4]]>, D::Error> {
        Ok(Vec::<[f32; 4]>::deserialize(d)?.into())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SimulationModel {
    GrayScott {
        diffusion: [f32; 2],
        feed: f32,
        kill: f32,
    },
    /// Dimensionless nonlinear Schrödinger equation, kinetic=-dispersion*Laplacian.
    GrossPitaevskii {
        dispersion: f32,
        interaction: f32,
        damping: f32,
    },
    /// Growth probability proportional to |grad potential|^eta.
    DielectricGrowth {
        eta: f32,
        relaxation_steps: u32,
        branches_per_step: u32,
    },
    /// Paired, disjoint circles define classical Schottky generators and inverses.
    Schottky {
        pairs: Vec<CirclePair>,
        points: u32,
        burn_in: u32,
    },
    /// Dirichlet eigenmode of the masked, finite-difference Laplacian.
    Resonance { mode: u32, iterations: u32 },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CirclePair {
    pub source: [f32; 2],
    pub target: [f32; 2],
    pub radius: f32,
    pub twist: f32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SeedBlob {
    pub center: Vec3,
    pub radius: f32,
    pub amplitude: f32,
    pub phase: f32,
    pub winding: i32,
}
impl Default for SeedBlob {
    fn default() -> Self {
        Self {
            center: Vec3::ZERO,
            radius: 0.3,
            amplitude: 1.0,
            phase: 0.0,
            winding: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GuideKind {
    Source,
    Suppress,
    Protect,
    Attractor,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtisticGuide {
    pub center: Vec3,
    pub radius: f32,
    pub strength: f32,
    pub kind: GuideKind,
}
impl ArtisticGuide {
    pub fn validate(&self) -> Result<(), SceneError> {
        if !self.center.is_finite()
            || !self.radius.is_finite()
            || self.radius <= 0.0
            || !self.strength.is_finite()
            || self.strength < 0.0
        {
            return Err(SceneError::Graph("invalid composition guide".into()));
        }
        Ok(())
    }
    pub fn weight(&self, p: Vec3) -> f32 {
        let t = (1.0 - p.distance(self.center) / self.radius).clamp(0.0, 1.0);
        self.strength * t * t * (3.0 - 2.0 * t)
    }
}
impl Simulation {
    pub fn channels(&self) -> u32 {
        match self.model {
            SimulationModel::GrayScott { .. } | SimulationModel::DielectricGrowth { .. } => 2,
            SimulationModel::GrossPitaevskii { .. } => 4,
            _ => 1,
        }
    }
    pub fn field_inputs(&self) -> impl Iterator<Item = FieldId> + '_ {
        [self.potential, self.source, self.mask]
            .into_iter()
            .flatten()
    }
    pub fn validate(&self) -> Result<(), SceneError> {
        let fail = |s: &str| SceneError::Graph(format!("simulation {}: {s}", self.name));
        if self.potential.is_some()
            && !matches!(
                self.model,
                SimulationModel::GrayScott { .. } | SimulationModel::GrossPitaevskii { .. }
            )
        {
            return Err(fail("this model has no potential input"));
        }
        if self.source.is_some()
            && matches!(
                self.model,
                SimulationModel::Schottky { .. } | SimulationModel::Resonance { .. }
            )
        {
            return Err(fail("this model has no source input"));
        }
        if self.mask.is_some() && matches!(self.model, SimulationModel::Schottky { .. }) {
            return Err(fail(
                "compose the Schottky density with a mask in the field graph",
            ));
        }
        let finite = |v: &[f32]| v.iter().all(|x| x.is_finite());
        if let Some(snapshot) = &self.frozen {
            let cells = snapshot
                .resolution
                .iter()
                .map(|&n| u64::from(n))
                .product::<u64>();
            if cells != snapshot.values.len() as u64
                || snapshot.resolution.iter().any(|&n| n == 0 || n > 256)
                || snapshot.values.iter().any(|v| !finite(v))
            {
                return Err(fail("invalid frozen state"));
            }
        }
        if self.steps > 16384
            || !self.dt.is_finite()
            || self.dt <= 0.0
            || self.dt > 10.0
            || self.seeds.len() > 256
        {
            return Err(fail("invalid time or seed budget"));
        }
        for s in &self.seeds {
            if !s.center.is_finite()
                || !finite(&[s.radius, s.amplitude, s.phase])
                || s.radius <= 0.0
                || s.amplitude < 0.0
                || s.winding.unsigned_abs() > 32
            {
                return Err(fail("invalid seed blob"));
            }
        }
        match &self.model {
            SimulationModel::GrayScott {
                diffusion,
                feed,
                kill,
            } => {
                if !finite(&[diffusion[0], diffusion[1], *feed, *kill])
                    || diffusion.iter().any(|&x| x <= 0.0)
                    || !(0.0..=1.0).contains(feed)
                    || !(0.0..=1.0).contains(kill)
                {
                    return Err(fail("invalid reaction/diffusion coefficients"));
                }
            }
            SimulationModel::GrossPitaevskii {
                dispersion,
                interaction,
                damping,
            } => {
                if !finite(&[*dispersion, *interaction, *damping])
                    || *dispersion <= 0.0
                    || *damping < 0.0
                    || *damping > 1.0
                {
                    return Err(fail("invalid nonlinear Schrödinger coefficients"));
                }
            }
            SimulationModel::DielectricGrowth {
                eta,
                relaxation_steps,
                branches_per_step,
            } => {
                if !eta.is_finite()
                    || !(0.0..=16.0).contains(eta)
                    || !(1..=512).contains(relaxation_steps)
                    || !(1..=64).contains(branches_per_step)
                {
                    return Err(fail("invalid growth budget"));
                }
            }
            SimulationModel::Schottky {
                pairs,
                points,
                burn_in,
            } => {
                if !(1..=8).contains(&pairs.len())
                    || *points > 4_000_000
                    || *points == 0
                    || *burn_in > 4096
                {
                    return Err(fail("invalid Schottky orbit budget"));
                }
                let mut circles = vec![];
                for p in pairs {
                    if !finite(&[
                        p.source[0],
                        p.source[1],
                        p.target[0],
                        p.target[1],
                        p.radius,
                        p.twist,
                    ]) || p.radius <= 0.0
                    {
                        return Err(fail("invalid Schottky circle"));
                    }
                    circles.push((p.source, p.radius));
                    circles.push((p.target, p.radius));
                }
                for (i, (a, r)) in circles.iter().enumerate() {
                    for (b, s) in &circles[..i] {
                        if (a[0] - b[0]).hypot(a[1] - b[1]) <= r + s {
                            return Err(fail("Schottky defining circles must be disjoint"));
                        }
                    }
                }
            }
            SimulationModel::Resonance { mode, iterations } => {
                if *mode > 15 || !(8..=4096).contains(iterations) {
                    return Err(fail("invalid eigenmode budget"));
                }
            }
        }
        Ok(())
    }
}
