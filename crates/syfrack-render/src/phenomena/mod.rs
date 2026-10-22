//! Cached simulation preparation and a single finite field representation shared
//! by the fixed GPU control engine and the specialized/wavefront transports.
mod creation;
pub mod fields;
mod flame;
mod gpu;
pub mod grid;
pub mod simulation;

use crate::Uniforms;
use anyhow::{ensure, Result};
use glam::Vec4;
use grid::Grid;
use rayon::prelude::*;
use std::{
    collections::VecDeque,
    hash::{Hash, Hasher},
    sync::{Arc, Mutex},
};
use syfrack_core::{scene::phenomena::*, Scene};

/// Background interactive work yields between bounded compute submissions while
/// an export owns the device. Cancelling an obsolete edit never publishes it.
#[derive(Clone, Default)]
pub struct PreparationControl {
    pub paused: Arc<std::sync::atomic::AtomicBool>,
    pub cancelled: Arc<std::sync::atomic::AtomicBool>,
}
impl PreparationControl {
    pub fn checkpoint(&self) -> Result<()> {
        use std::sync::atomic::Ordering;
        loop {
            ensure!(
                !self.cancelled.load(Ordering::Relaxed),
                "obsolete field preparation cancelled"
            );
            if !self.paused.load(Ordering::Relaxed) {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    }
}

#[derive(Default)]
pub struct PhenomenaCache {
    items: Mutex<VecDeque<(PreparationKey, Arc<PreparedPhenomena>)>>,
    gpu: Mutex<Option<Arc<gpu::Engine>>>,
    flame_gpu: Mutex<Option<Arc<flame::Engine>>>,
    simulations: Mutex<VecDeque<(SimulationKey, Grid, simulation::Diagnostics)>>,
}
#[derive(PartialEq)]
struct SimulationKey {
    simulation: Simulation,
    domain: FieldDomain,
    guides: Vec<ArtisticGuide>,
    seed: u64,
    time: f32,
    gpu: bool,
}
#[derive(PartialEq)]
struct PreparationKey {
    creation: bool,
    fields: Phenomena,
    geometry: syfrack_core::Graph,
    seed: u64,
    gpu: bool,
}
pub struct PreparedPhenomena {
    pub grid: Grid,
    pub extra: Grid,
    pub snapshots: Vec<Grid>,
    pub diagnostics: Vec<simulation::Diagnostics>,
    pub geometry_lipschitz: f32,
    pub displacement_lipschitz: f32,
    pub flames: Vec<FrozenSnapshot>,
    pub fallbacks: std::collections::BTreeMap<FieldId, Grid>,
}

pub fn resource_key(scene: &Scene, base: u64) -> u64 {
    if scene.phenomena.is_none() {
        return base;
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    base.hash(&mut h);
    for source in [
        include_str!("mod.rs"),
        include_str!("fields.rs"),
        include_str!("simulation.rs"),
        include_str!("grid.rs"),
        include_str!("gpu.rs"),
        include_str!("flame.rs"),
        include_str!("creation.rs"),
        include_str!("../shaders/flame_field.wgsl"),
        include_str!("../shaders/creation.wgsl"),
        include_str!("../shaders/pattern.wgsl"),
        include_str!("../shaders/phenomena_compute.wgsl"),
        include_str!("../../../syfrack-core/src/scene/phenomena/simulation.rs"),
    ] {
        crate::shader_source::transport_fingerprint(source.split("#[cfg(test)]").next().unwrap())
            .hash(&mut h);
    }
    h.finish()
}
impl PhenomenaCache {
    pub fn get(&self, scene: &Scene) -> Result<Option<Arc<PreparedPhenomena>>> {
        self.get_internal(None, scene, None)
    }
    pub fn get_gpu(
        &self,
        r: &crate::Renderer,
        scene: &Scene,
    ) -> Result<Option<Arc<PreparedPhenomena>>> {
        self.get_internal(Some(r), scene, None)
    }
    pub fn get_controlled(
        &self,
        r: &crate::Renderer,
        scene: &Scene,
        control: &PreparationControl,
    ) -> Result<Option<Arc<PreparedPhenomena>>> {
        self.get_internal(Some(r), scene, Some(control))
    }
    fn get_internal(
        &self,
        renderer: Option<&crate::Renderer>,
        scene: &Scene,
        control: Option<&PreparationControl>,
    ) -> Result<Option<Arc<PreparedPhenomena>>> {
        if let Some(c) = control {
            c.checkpoint()?;
        }
        let Some(p) = &scene.phenomena else {
            return Ok(None);
        };
        if p.fields.nodes.is_empty() && p.simulations.is_empty() {
            return Ok(None);
        }
        p.validate()?;
        let mut fields = p.clone();
        fields.editor = None;
        fields.timeline = Default::default();
        fields.film = None;
        fields.dispersion = None;
        fields.lens = None;
        let mut pending: Vec<_> = [
            p.bindings.rgb,
            p.bindings.color,
            p.bindings.emission,
            p.bindings.density,
            p.bindings.geometry,
            p.bindings.displacement,
            p.bindings.film_thickness,
        ]
        .into_iter()
        .flatten()
        .chain(p.simulations.iter().flat_map(|s| s.field_inputs()))
        .collect();
        let mut live = std::collections::BTreeSet::new();
        while let Some(id) = pending.pop() {
            if live.insert(id) {
                pending
                    .extend_from_slice(&p.fields.nodes.iter().find(|n| n.id == id).unwrap().inputs);
            }
        }
        fields.fields.nodes.retain(|n| live.contains(&n.id));
        for node in &mut fields.fields.nodes {
            node.name.clear();
            node.pos = [0.0; 2];
        }
        for simulation in &mut fields.simulations {
            simulation.name.clear();
        }
        fields.bindings = FieldBindings {
            rgb: p.bindings.rgb,
            color: p.bindings.color,
            emission: p.bindings.emission,
            density: p.bindings.density,
            geometry: p.bindings.geometry,
            displacement: p.bindings.displacement,
            film_thickness: p.bindings.film_thickness,
            ..Default::default()
        };
        let reads_geometry = fields.fields.nodes.iter().any(|n| {
            matches!(
                n.op,
                FieldOp::GeometryDistance | FieldOp::GeometryTrap { .. }
            )
        });
        let mut geometry = if reads_geometry {
            scene.fractal.graph().clone()
        } else {
            syfrack_core::Graph::new_minimal()
        };
        for node in &mut geometry.nodes {
            node.name.clear();
            node.pos = [0.0; 2];
        }
        let key = PreparationKey {
            creation: scene.creation.is_some(),
            fields,
            geometry,
            seed: scene.seed,
            gpu: renderer.is_some(),
        };
        if let Some((_, item)) = self
            .items
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|(k, _)| k == &key)
        {
            return Ok(Some(item.clone()));
        }
        let engine = if let Some(r) = renderer.filter(|_| {
            p.simulations.iter().any(|s| {
                s.frozen.is_none()
                    && matches!(
                        s.model,
                        SimulationModel::GrayScott { .. } | SimulationModel::GrossPitaevskii { .. }
                    )
            })
        }) {
            let mut cached = self.gpu.lock().unwrap_or_else(|p| p.into_inner());
            if cached.is_none() {
                *cached = Some(Arc::new(gpu::Engine::new(r)?));
            }
            cached.clone()
        } else {
            None
        };
        let flame_engine =
            if let Some(r) = renderer.filter(|_| p.flames.iter().any(|f| f.frozen.is_none())) {
                let mut cached = self.flame_gpu.lock().unwrap_or_else(|p| p.into_inner());
                if cached.is_none() {
                    *cached = Some(Arc::new(flame::Engine::new(r)?));
                }
                cached.clone()
            } else {
                None
            };
        let item = Arc::new(prepare(
            scene,
            p,
            renderer.zip(engine.as_ref()),
            renderer.zip(flame_engine.as_ref()),
            control,
            &self.simulations,
        )?);
        let mut items = self.items.lock().unwrap_or_else(|p| p.into_inner());
        // A concurrently prepared immutable snapshot has identical contents.
        if let Some((_, existing)) = items.iter().find(|(k, _)| k == &key) {
            return Ok(Some(existing.clone()));
        }
        items.push_back((key, item.clone()));
        while items.len() > 2 {
            items.pop_front();
        }
        Ok(Some(item))
    }
}
fn prepare(
    scene: &Scene,
    p: &Phenomena,
    gpu: Option<(&crate::Renderer, &Arc<gpu::Engine>)>,
    flame_gpu: Option<(&crate::Renderer, &Arc<flame::Engine>)>,
    control: Option<&PreparationControl>,
    source_cache: &Mutex<VecDeque<(SimulationKey, Grid, simulation::Diagnostics)>>,
) -> Result<PreparedPhenomena> {
    let atlas_bytes: u64 = p
        .flames
        .iter()
        .map(|f| u64::from(f.resolution).pow(2) * 16)
        .sum();
    let fallback_count = p
        .fields
        .nodes
        .iter()
        .filter(|n| {
            matches!(
                n.op,
                FieldOp::Gradient
                    | FieldOp::Divergence
                    | FieldOp::Curl
                    | FieldOp::GeometryDistance
                    | FieldOp::GeometryTrap { .. }
                    | FieldOp::QuaternionJulia { .. }
            ) || matches!(&n.op,FieldOp::Texture{texture} if texture.planar.is_none())
        })
        .count() as u64;
    let need_base = scene.creation.is_none()
        || p.bindings.geometry.is_some()
        || p.bindings.displacement.is_some();
    let base_cells = if need_base { p.domain.cells() } else { 4 };
    let solver_cells = if p.simulations.is_empty() {
        0
    } else {
        p.domain.cells() * (p.simulations.len() as u64 + 16)
    };
    ensure!(atlas_bytes*6+(solver_cells+p.domain.cells()*fallback_count)*16+base_cells*64<=u64::from(p.domain.memory_budget_mib)*1024*1024,
        "creation source working sets exceed the graph memory budget; reduce source resolution or increase the domain budget");
    let mut owned_p = p.clone();
    let mut flames = vec![];
    for f in &mut owned_p.flames {
        let frozen = if let Some(frozen) = &f.frozen {
            frozen.clone()
        } else {
            let (r, engine) = flame_gpu.ok_or_else(|| {
                anyhow::anyhow!("an unfrozen flame source requires GPU preparation")
            })?;
            flame::prepare(r, engine, f, control)?
        };
        f.frozen = Some(frozen.clone());
        flames.push(frozen);
    }
    let mut owned_scene = scene.clone();
    owned_scene.phenomena = Some(owned_p.clone());
    let scene = &owned_scene;
    let p = &owned_p;
    let mut snapshots = vec![];
    let mut diagnostics = vec![];
    for i in 0..p.simulations.len() {
        if let Some(c) = control {
            c.checkpoint()?;
        }
        let mut source = p.simulations[i].clone();
        source.name.clear();
        // A PDE without graph inputs does not change when a palette or wire
        // downstream changes. Cache only this provably independent case.
        let independent = source.field_inputs().next().is_none();
        let key = independent.then(|| SimulationKey {
            simulation: source,
            domain: p.domain.clone(),
            guides: p.guides.clone(),
            seed: scene.seed,
            time: p.time,
            gpu: gpu.is_some(),
        });
        if let Some(key) = &key {
            if let Some((_, grid, diagnostic)) = source_cache
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .find(|(k, _, _)| k == key)
            {
                snapshots.push(grid.clone());
                diagnostics.push(diagnostic.clone());
                continue;
            }
        }
        let (grid, diagnostic) = if let Some((r, engine)) = gpu.filter(|_| {
            p.simulations[i].frozen.is_none()
                && matches!(
                    p.simulations[i].model,
                    SimulationModel::GrayScott { .. } | SimulationModel::GrossPitaevskii { .. }
                )
        }) {
            gpu::simulate(r, engine, scene, p, i, &snapshots, control)?
        } else {
            simulation::prepare(scene, p, i, &snapshots)?
        };
        if let Some(key) = key {
            let mut cache = source_cache.lock().unwrap_or_else(|e| e.into_inner());
            cache.push_back((key, grid.clone(), diagnostic.clone()));
            while cache.len() > 2 {
                cache.pop_front();
            }
        }
        snapshots.push(grid);
        diagnostics.push(diagnostic);
    }
    if let Some(c) = control {
        c.checkpoint()?;
    }
    let mut base_domain = p.domain.clone();
    if !need_base {
        base_domain.resolution = [2, 2, 1];
    }
    let mut grid = Grid::zero(&base_domain);
    let mut extra = Grid::zero(&base_domain);
    let eval = fields::Evaluator::new(scene, p, &snapshots)?;
    let values: Vec<_> = (0..grid.data.len())
        .into_par_iter()
        .map(|i| -> Result<(Vec4, Vec4)> {
            let point = grid.point(i);
            let dynamic = scene.creation.is_some();
            let roots = [
                if dynamic { None } else { p.bindings.color },
                if dynamic { None } else { p.bindings.emission },
                if dynamic { None } else { p.bindings.density },
                p.bindings.geometry,
                p.bindings.displacement,
                if dynamic {
                    None
                } else {
                    p.bindings.film_thickness
                },
            ];
            let values = eval.sample_outputs(roots, point)?;
            let mut a = Vec4::new(values[0].x, values[1].x, values[2].x, values[3].x);
            let mut b = Vec4::new(values[4].x, values[5].x, 0.0, 0.0);
            let c = grid.coord(i);
            let boundary = (0..3).any(|axis| {
                p.domain.resolution[axis] > 1
                    && (c[axis] == 0 || c[axis] + 1 == p.domain.resolution[axis])
            });
            if boundary {
                b.x = 0.0;
            }
            a.y = a.y.max(0.0);
            a.z = a.z.max(0.0);
            Ok((a, b))
        })
        .collect::<Result<_>>()?;
    for ((a, b), (v, w)) in grid.data.iter_mut().zip(&mut extra.data).zip(values) {
        *a = v;
        *b = w;
    }
    let geometry_lipschitz = grid.lipschitz(3).max(1e-6);
    let displacement_lipschitz = extra.lipschitz(0);
    // Differential/geometry-derived nodes retain the finite-domain contract.
    // Analytic pattern and wave colour is evaluated natively at the actual
    // sample position, never upsampled from this coarse grid.
    let mut fallbacks = std::collections::BTreeMap::new();
    if scene.creation.is_some() {
        for n in &p.fields.nodes {
            if matches!(
                n.op,
                FieldOp::Gradient
                    | FieldOp::Divergence
                    | FieldOp::Curl
                    | FieldOp::GeometryDistance
                    | FieldOp::GeometryTrap { .. }
                    | FieldOp::QuaternionJulia { .. }
            ) || matches!(&n.op,FieldOp::Texture{texture} if texture.planar.is_none())
            {
                let mut field = Grid::zero(&p.domain);
                field.data = (0..field.data.len())
                    .into_par_iter()
                    .map(|i| eval.sample(n.id, field.point(i)))
                    .collect::<Result<_>>()?;
                fallbacks.insert(n.id, field);
            }
        }
    }
    Ok(PreparedPhenomena {
        grid,
        extra,
        snapshots,
        diagnostics,
        geometry_lipschitz,
        displacement_lipschitz,
        flames,
        fallbacks,
    })
}
/// Append two vec4 rows per cell to the existing parameter texture. This avoids
/// adding storage bindings to the legacy-GPU control and wavefront layouts.
pub fn append_parameters(
    scene: &Scene,
    prepared: Option<&PreparedPhenomena>,
    values: &mut Vec<f32>,
    uniforms: &mut Uniforms,
) -> Result<()> {
    let Some(_p) = &scene.phenomena else {
        return Ok(());
    };
    let Some(prepared) = prepared else {
        return Ok(());
    };
    while values.len() % 4 != 0 {
        values.push(0.0);
    }
    let offset = (values.len() / 4) as u32;
    uniforms.phenomena.grid = [
        offset,
        prepared.grid.domain.resolution[0],
        prepared.grid.domain.resolution[1],
        prepared.grid.domain.resolution[2],
    ];
    for (a, b) in prepared.grid.data.iter().zip(&prepared.extra.data) {
        values.extend_from_slice(&a.to_array());
        values.extend_from_slice(&b.to_array());
    }
    uniforms.phenomena.center[3] = prepared.geometry_lipschitz;
    uniforms.phenomena.half[3] = prepared.displacement_lipschitz;
    creation::append_program(scene, prepared, values, uniforms)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quaternion_empty_margin_cannot_become_a_box_face() {
        let scene = presets::scene("quaternion").unwrap();
        let prepared = PhenomenaCache::default().get(&scene).unwrap().unwrap();
        assert!(prepared.grid.data.iter().any(|v| v.w < 0.0));
        let margin = prepared
            .grid
            .data
            .iter()
            .enumerate()
            .filter(|(i, _)| {
                let c = prepared.grid.coord(*i);
                (0..3).any(|a| c[a] == 0 || c[a] + 1 == prepared.grid.domain.resolution[a])
            })
            .map(|(_, v)| v.w / prepared.geometry_lipschitz)
            .fold(f32::INFINITY, f32::min);
        assert!(
            margin > 0.05,
            "false near-surface in empty margin: {margin}"
        );
    }
    #[test]
    fn unbound_undefined_math_is_not_evaluated_by_render_preparation() {
        let mut s = Scene::example();
        let mut p = Phenomena::default();
        p.domain.resolution = [4, 4, 4];
        p.fields.nodes = vec![
            FieldNode {
                id: 0,
                name: "live".into(),
                op: FieldOp::Scalar { value: 0.25 },
                inputs: vec![],
                pos: [0.0; 2],
            },
            FieldNode {
                id: 1,
                name: "negative".into(),
                op: FieldOp::Scalar { value: -1.0 },
                inputs: vec![],
                pos: [0.0; 2],
            },
            FieldNode {
                id: 2,
                name: "unused square root".into(),
                op: FieldOp::Power { exponent: 0.5 },
                inputs: vec![1],
                pos: [0.0; 2],
            },
        ];
        p.bindings.color = Some(0);
        s.phenomena = Some(p);
        let prepared = PhenomenaCache::default().get(&s).unwrap().unwrap();
        assert!(prepared.grid.data.iter().all(|v| v.x == 0.25));
    }

    #[test]
    fn presentation_edits_reuse_the_same_prepared_values_and_simulation_changes_do_not() {
        let mut s = presets::scene("interference").unwrap();
        s.phenomena.as_mut().unwrap().domain.resolution = [8, 8, 8];
        let cache = PhenomenaCache::default();
        let a = cache.get(&s).unwrap().unwrap();
        s.look.exposure_ev = 2.0;
        s.camera.focal_mm = 60.0;
        let p = s.phenomena.as_mut().unwrap();
        p.bindings.emission_gain = 20.0;
        p.bindings.palette.stops[0] = [0.2, 0.3, 0.4];
        p.fields.nodes[0].name = "renamed".into();
        p.fields.nodes[0].pos = [100.0, 40.0];
        assert!(Arc::ptr_eq(&a, &cache.get(&s).unwrap().unwrap()));
        s.phenomena.as_mut().unwrap().time = 0.3;
        assert!(!Arc::ptr_eq(&a, &cache.get(&s).unwrap().unwrap()));
    }
    #[test]
    fn trilinear_gradient_bound_contains_every_cell_derivative() {
        let mut d = FieldDomain::default();
        d.resolution = [9, 9, 9];
        let mut g = Grid::zero(&d);
        for i in 0..g.data.len() {
            let p = g.point(i);
            g.data[i].x = 2.0 * p.x + 3.0 * p.y - p.z;
        }
        assert!((g.lipschitz(0) - 14.0f32.sqrt()).abs() < 1e-6);
        let point = glam::Vec3::new(0.23, -0.61, 0.72);
        assert!((g.sample(point).x - (2.0 * point.x + 3.0 * point.y - point.z)).abs() < 1e-6);
    }
}
