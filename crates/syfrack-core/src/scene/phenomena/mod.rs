//! Typed, bounded natural fields and reproducible simulation snapshots.
//! The unified creation compiler lowers its typed fields to this transport IR.
pub mod exploration;
pub mod fields;
pub mod optics;
mod params;
pub mod presets;
pub mod relativity;
pub mod simulation;
pub mod sources;

pub use exploration::*;
pub use fields::*;
pub use optics::*;
pub use relativity::*;
pub use simulation::*;
pub use sources::*;

use crate::{Scene, SceneError};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Phenomena {
    pub fields: FieldGraph,
    pub domain: FieldDomain,
    pub simulations: Vec<Simulation>,
    pub flames: Vec<FlameField>,
    pub bindings: FieldBindings,
    pub time: f32,
    pub film: Option<ThinFilm>,
    pub dispersion: Option<ChromaticDispersion>,
    pub lens: Option<RelativisticLens>,
    pub timeline: Timeline,
    pub guides: Vec<ArtisticGuide>,
    /// Incomplete editor work is saved separately from the last valid field DAG.
    pub editor: Option<FieldGraph>,
}
impl Default for Phenomena {
    fn default() -> Self {
        Self {
            fields: FieldGraph::default(),
            domain: FieldDomain::default(),
            simulations: vec![],
            flames: vec![],
            bindings: FieldBindings::default(),
            time: 0.0,
            film: None,
            dispersion: None,
            lens: None,
            timeline: Timeline::default(),
            guides: vec![],
            editor: None,
        }
    }
}
impl Phenomena {
    pub fn validate(&self) -> Result<(), SceneError> {
        let error = |s: &str| SceneError::Graph(format!("phenomena: {s}"));
        if !self.time.is_finite() || self.time < 0.0 {
            return Err(error("invalid frozen time"));
        }
        self.domain.validate()?;
        let types = self.fields.validate(&self.simulations)?;
        if self.flames.len() > 8 {
            return Err(error("at most eight flame resources"));
        }
        for f in &self.flames {
            f.validate()?;
        }
        for n in &self.fields.nodes {
            if let FieldOp::Flame { source, .. } = n.op {
                if source as usize >= self.flames.len() {
                    return Err(error("missing flame resource"));
                }
            }
        }
        self.bindings.validate(&types)?;
        if self.simulations.len() > 8 {
            return Err(error("at most eight simulation snapshots"));
        }
        for (index, s) in self.simulations.iter().enumerate() {
            s.validate()?;
            if s.frozen
                .as_ref()
                .is_some_and(|f| f.resolution != self.domain.resolution)
            {
                return Err(error("frozen state resolution differs from domain; unfreeze before changing the grid"));
            }
            for id in s.field_inputs() {
                if types.get(&id) != Some(&FieldType::Scalar) {
                    return Err(error("simulation inputs must be scalar fields"));
                }
                let mut pending = vec![id];
                let mut seen = std::collections::BTreeSet::new();
                while let Some(id) = pending.pop() {
                    if !seen.insert(id) {
                        continue;
                    }
                    let node = self.fields.nodes.iter().find(|n| n.id == id).unwrap();
                    if matches!(node.op,FieldOp::Snapshot{simulation,..} if simulation as usize>=index)
                    {
                        return Err(error("simulation depends on its own or a future snapshot"));
                    }
                    pending.extend_from_slice(&node.inputs);
                }
            }
        }
        if let Some(film) = &self.film {
            film.validate()?;
        }
        if let Some(d) = &self.dispersion {
            d.validate()?;
        }
        if let Some(lens) = &self.lens {
            lens.validate()?;
        }
        self.timeline.validate()?;
        if self.guides.len() > 256 {
            return Err(error("at most 256 composition guides"));
        }
        for g in &self.guides {
            g.validate()?;
        }
        Ok(())
    }
    /// Resolve animation into an immutable scene. Saved exports keep the time.
    pub fn scene_at(scene: &Scene, time: f32) -> Result<Scene, SceneError> {
        let mut result = scene.clone();
        if let Some(g) = &scene.creation {
            if !time.is_finite() || time < 0.0 {
                return Err(SceneError::Graph("invalid timeline time".into()));
            }
            g.settings.timeline.validate()?;
            for (path, value) in g.settings.timeline.values_at(time) {
                result.set_param(&path, value)?;
            }
            result.creation.as_mut().unwrap().settings.time = time;
            result.unify()?;
            return Ok(result);
        }
        let Some(p) = &scene.phenomena else {
            return Ok(result);
        };
        if !time.is_finite() || time < 0.0 {
            return Err(SceneError::Graph("invalid timeline time".into()));
        }
        p.timeline.validate()?;
        let changes = p.timeline.values_at(time);
        for (path, value) in changes {
            result.set_param(&path, value)?;
        }
        if let Some(p) = &mut result.phenomena {
            p.time = time;
        }
        result.validate()?;
        Ok(result)
    }
}
