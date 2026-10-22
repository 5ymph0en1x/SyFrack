use glam::Vec3;
use syfrack_core::scene::phenomena::*;
use syfrack_core::{Phenomena, Scene};
fn n(id: u32, op: FieldOp, inputs: Vec<u32>) -> FieldNode {
    FieldNode {
        id,
        name: format!("field {id}"),
        op,
        inputs,
        pos: [0.0, 0.0],
    }
}
#[test]
fn field_graph_checks_types_missing_inputs_duplicates_and_cycles() {
    let mut p = Phenomena::default();
    p.fields.nodes = vec![
        n(8, FieldOp::Intensity, vec![4]),
        n(
            4,
            FieldOp::PlaneWave {
                wavevector: Vec3::X,
                frequency: 1.0,
                phase: 0.0,
            },
            vec![2],
        ),
        n(2, FieldOp::Position, vec![]),
    ];
    p.bindings.color = Some(8);
    p.validate().unwrap();
    assert_eq!(p.fields.order().unwrap(), vec![2, 4, 8]);
    p.fields.nodes[0].op = FieldOp::Gradient;
    assert!(p.validate().is_err());
    p.fields.nodes[0].op = FieldOp::Intensity;
    p.fields.nodes[1].inputs = vec![42];
    assert!(p.validate().is_err());
    p.fields.nodes[1].inputs = vec![8];
    assert!(p.validate().is_err());
}
#[test]
fn simulation_feedback_requires_explicit_state_and_cannot_cycle_through_fields() {
    let mut p = Phenomena::default();
    p.simulations.push(Simulation::default());
    p.fields.nodes = vec![
        n(0, FieldOp::Position, vec![]),
        n(
            1,
            FieldOp::Snapshot {
                simulation: 0,
                channel: 0,
            },
            vec![0],
        ),
    ];
    p.validate().unwrap();
    p.simulations[0].potential = Some(1);
    assert!(p.validate().is_err());
}
#[test]
fn old_scenes_stay_without_phenomena_and_new_scenes_roundtrip() {
    let mut s = Scene::example();
    assert!(s.phenomena.is_none());
    s.phenomena = Some(Phenomena::default());
    let p = s.phenomena.as_mut().unwrap();
    p.film = Some(ThinFilm::default());
    p.lens = Some(RelativisticLens::default());
    s.validate().unwrap();
    assert_eq!(s, Scene::from_ron(&s.to_ron()).unwrap());
    s.set_param("phenomena.film.thickness_nm", 760.0).unwrap();
    assert_eq!(s.phenomena.unwrap().film.unwrap().thickness_nm, 760.0);
}
#[test]
fn timeline_has_canonical_boundaries_and_restores_an_immutable_scene() {
    let mut s = Scene::example();
    let mut p = Phenomena::default();
    p.timeline.tracks.push(AnimationTrack {
        path: "look.exposure_ev".into(),
        interpolation: Interpolation::Smooth,
        keys: vec![
            Keyframe {
                time: 0.0,
                value: 0.0,
            },
            Keyframe {
                time: 2.0,
                value: 2.0,
            },
        ],
    });
    s.phenomena = Some(p);
    assert_eq!(Phenomena::scene_at(&s, 1.0).unwrap().look.exposure_ev, 1.0);
    assert_eq!(s.look.exposure_ev, 0.0);
    assert_eq!(Phenomena::scene_at(&s, 3.0).unwrap().look.exposure_ev, 2.0);
}
#[test]
fn variant_branches_preserve_parent_and_are_reproducible() {
    let s = Scene::example();
    let paths = vec!["look.exposure_ev".into(), "camera.focal_mm".into()];
    let a = VariantAtlas::variations(&s, &paths, 8, 0.1, 7).unwrap();
    let b = VariantAtlas::variations(&s, &paths, 8, 0.1, 7).unwrap();
    assert_eq!(a, b);
    assert_ne!(a[0], a[1]);
    let mut atlas = VariantAtlas::default();
    let root = atlas.insert(None, "root".into(), s.clone()).unwrap();
    atlas
        .insert(Some(root), "child".into(), a[0].clone())
        .unwrap();
    assert_eq!(atlas.branches[0].scene, s);
    assert!(atlas.insert(Some(999), "orphan".into(), s).is_err());
}
#[test]
fn schottky_generators_reject_intersecting_defining_circles() {
    let mut s = Simulation::default();
    s.model = SimulationModel::Schottky {
        pairs: vec![CirclePair {
            source: [-1.0, 0.0],
            target: [1.0, 0.0],
            radius: 0.3,
            twist: 0.0,
        }],
        points: 1000,
        burn_in: 16,
    };
    s.validate().unwrap();
    if let SimulationModel::Schottky { pairs, .. } = &mut s.model {
        pairs[0].radius = 1.0;
    }
    assert!(s.validate().is_err());
}

#[test]
fn imported_atlas_rejects_duplicate_dangling_and_cyclic_lineages() {
    let mut a = VariantAtlas::default();
    a.insert(None, "root".into(), Scene::example()).unwrap();
    a.insert(Some(0), "child".into(), Scene::example()).unwrap();
    a.validate().unwrap();
    let mut b = a.clone();
    b.branches[1].id = 0;
    assert!(b.validate().is_err());
    let mut b = a.clone();
    b.branches[0].parent = Some(1);
    assert!(b.validate().is_err());
    a.next_id = 1;
    assert!(a.validate().is_err());
}
#[test]
fn frozen_states_are_portable_and_drafts_do_not_replace_active_fields() {
    let mut s = presets::scene("morphogenesis").unwrap();
    let p = s.phenomena.as_mut().unwrap();
    p.domain.resolution = [2, 2, 1];
    p.simulations[0].frozen = Some(FrozenSnapshot {
        resolution: [2, 2, 1],
        values: vec![[0.7, 0.2, 0.0, 0.0]; 4].into(),
    });
    p.editor = Some(FieldGraph {
        nodes: vec![n(3, FieldOp::Gradient, vec![999])],
    });
    s.validate().unwrap();
    assert_eq!(s, Scene::from_ron(&s.to_ron()).unwrap());
    s.phenomena.as_mut().unwrap().domain.resolution[0] = 3;
    assert!(s.validate().is_err());
}
#[test]
fn cauchy_dispersion_has_correct_reference_and_blue_bending_order() {
    let d = ChromaticDispersion::default();
    assert!((d.index(1.5, 550.0) - 1.5).abs() < 1e-6);
    assert!(d.index(1.5, 460.0) > d.index(1.5, 610.0));
    assert!(ThinFilm::default()
        .rgb(1.0, 0.0)
        .iter()
        .all(|x| (*x - 0.04).abs() < 1e-5));
}

#[test]
fn bundled_reaction_diffusion_fits_the_stable_work_budget_at_its_actual_cell_spacing() {
    let s = presets::scene("morphogenesis").unwrap();
    let p = s.phenomena.unwrap();
    let sim = &p.simulations[0];
    let SimulationModel::GrayScott { diffusion, .. } = sim.model else {
        panic!()
    };
    let mut sum = 0.0;
    for axis in 0..3 {
        if p.domain.resolution[axis] > 1 {
            let spacing = 2.0 * p.domain.half[axis] / (p.domain.resolution[axis] - 1) as f32;
            sum += 1.0 / (spacing * spacing);
        }
    }
    let subdivisions = (sim.dt * (2.0 * diffusion[0].max(diffusion[1]) * sum + 4.0) / 0.2)
        .ceil()
        .max(1.0) as u64;
    assert!(subdivisions * u64::from(sim.steps) * p.domain.cells() < 1_000_000_000);
}

#[test]
fn field_parameter_paths_follow_node_identity_across_deletions() {
    let mut s = Scene::example();
    let mut p = Phenomena::default();
    p.fields.nodes = vec![
        n(42, FieldOp::Scalar { value: 2.0 }, vec![]),
        n(7, FieldOp::Scalar { value: 3.0 }, vec![]),
    ];
    s.phenomena = Some(p);
    s.set_param("phenomena.fields.nodes[42].value", 4.0)
        .unwrap();
    s.phenomena.as_mut().unwrap().fields.nodes.remove(1);
    s.set_param("phenomena.fields.nodes[42].value", 5.0)
        .unwrap();
    assert_eq!(
        s.phenomena.unwrap().fields.nodes[0].op,
        FieldOp::Scalar { value: 5.0 }
    );
}

#[test]
fn model_specific_inputs_are_not_silently_ignored() {
    let mut s = Simulation {
        model: SimulationModel::Resonance {
            mode: 0,
            iterations: 32,
        },
        potential: Some(0),
        ..Default::default()
    };
    assert!(s.validate().is_err());
    s.potential = None;
    s.source = Some(0);
    assert!(s.validate().is_err());
    s.source = None;
    s.mask = Some(0);
    s.validate().unwrap();
}
