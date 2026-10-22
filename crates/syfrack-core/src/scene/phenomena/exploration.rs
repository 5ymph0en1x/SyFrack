//! Immutable parameter branches and explicitly timed interpolation.
use crate::{Scene, SceneError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Interpolation {
    Step,
    #[default]
    Linear,
    Smooth,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    pub time: f32,
    pub value: f32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnimationTrack {
    pub path: String,
    pub interpolation: Interpolation,
    pub keys: Vec<Keyframe>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Timeline {
    pub tracks: Vec<AnimationTrack>,
}
impl Timeline {
    pub fn validate(&self) -> Result<(), SceneError> {
        let mut paths = BTreeSet::new();
        if self.tracks.len() > 256 {
            return Err(SceneError::Graph("too many timeline tracks".into()));
        }
        for t in &self.tracks {
            if t.path.is_empty()
                || !paths.insert(&t.path)
                || t.keys.is_empty()
                || t.keys.len() > 4096
            {
                return Err(SceneError::Graph("invalid animation track".into()));
            }
            let mut previous = -1.0;
            for k in &t.keys {
                if !k.time.is_finite() || !k.value.is_finite() || k.time < 0.0 || k.time <= previous
                {
                    return Err(SceneError::Graph(
                        "keyframes must have distinct increasing finite times".into(),
                    ));
                }
                previous = k.time;
            }
        }
        Ok(())
    }
    pub fn values_at(&self, time: f32) -> Vec<(String, f32)> {
        self.tracks
            .iter()
            .map(|t| {
                let keys = &t.keys;
                let upper = keys.partition_point(|k| k.time <= time);
                let value = if upper == 0 {
                    keys[0].value
                } else if upper == keys.len() {
                    keys[upper - 1].value
                } else {
                    let a = &keys[upper - 1];
                    let b = &keys[upper];
                    let mut f = (time - a.time) / (b.time - a.time);
                    match t.interpolation {
                        Interpolation::Step => f = 0.0,
                        Interpolation::Smooth => f = f * f * (3.0 - 2.0 * f),
                        Interpolation::Linear => {}
                    }
                    a.value + (b.value - a.value) * f
                };
                (t.path.clone(), value)
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VariantBranch {
    pub id: u64,
    pub parent: Option<u64>,
    pub label: String,
    pub scene: Scene,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VariantAtlas {
    pub next_id: u64,
    pub branches: Vec<VariantBranch>,
}
impl VariantAtlas {
    /// Validate imported lineages before opening any contained scene. Parents
    /// precede their children, so cycles and dangling references are impossible.
    pub fn validate(&self) -> Result<(), SceneError> {
        if self.branches.len() > 512 {
            return Err(SceneError::Graph(
                "variant atlas exceeds 512 saved branches".into(),
            ));
        }
        let mut seen = BTreeSet::new();
        for branch in &self.branches {
            if branch.id >= self.next_id
                || seen.contains(&branch.id)
                || branch.parent.is_some_and(|id| !seen.contains(&id))
            {
                return Err(SceneError::Graph(
                    "invalid variant lineage or next id".into(),
                ));
            }
            branch.scene.validate()?;
            seen.insert(branch.id);
        }
        Ok(())
    }
    pub fn insert(
        &mut self,
        parent: Option<u64>,
        label: String,
        scene: Scene,
    ) -> Result<u64, SceneError> {
        self.validate()?;
        scene.validate()?;
        if parent.is_some_and(|p| !self.branches.iter().any(|b| b.id == p)) {
            return Err(SceneError::Graph("variant parent does not exist".into()));
        }
        if self.branches.len() >= 512 {
            return Err(SceneError::Graph(
                "variant atlas exceeds 512 saved branches".into(),
            ));
        }
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| SceneError::Graph("variant id overflow".into()))?;
        self.branches.push(VariantBranch {
            id,
            parent,
            label,
            scene,
        });
        Ok(id)
    }
    /// Mutate only selected continuous parameters; a rejected edit never changes the parent.
    pub fn variations(
        scene: &Scene,
        paths: &[String],
        count: usize,
        strength: f32,
        seed: u64,
    ) -> Result<Vec<Scene>, SceneError> {
        if count > 64 || !strength.is_finite() || !(0.0..=1.0).contains(&strength) {
            return Err(SceneError::Graph("invalid variation budget".into()));
        }
        let infos = scene.params();
        let mut selected = vec![];
        for path in paths {
            selected.push(
                infos
                    .iter()
                    .find(|p| &p.path == path)
                    .ok_or_else(|| SceneError::UnknownPath(path.clone()))?,
            );
        }
        let mut rng = seed;
        let mut result = vec![];
        for _ in 0..count {
            let mut copy = scene.clone();
            for p in &selected {
                rng = rng.wrapping_add(0x9e3779b97f4a7c15);
                let mut z = rng;
                z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
                z ^= z >> 31;
                let jitter = ((z >> 40) as f32 / 16777216.0) * 2.0 - 1.0;
                let span = (p.bounds.max - p.bounds.min).min(p.value.abs().max(0.1) * 2.0);
                let v = (p.value + jitter * span * strength).clamp(p.bounds.min, p.bounds.max);
                copy.set_param(&p.path, v)?;
            }
            copy.validate()?;
            result.push(copy);
        }
        Ok(result)
    }
}
