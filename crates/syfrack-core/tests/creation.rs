use syfrack_core::{scene::phenomena::*, *};

#[test]
fn migration_preserves_authored_names_hidden_patterns_and_pending_work() {
    let mut source = presets::scene("interference").unwrap();
    let p = source.phenomena.as_mut().unwrap();
    p.fields.nodes[0].name = "Coordonnées stellaires".into();
    p.fields.nodes[1].name = "Onde personnelle".into();
    let mut draft = p.fields.clone();
    draft.nodes[1].inputs.clear();
    p.editor = Some(draft);
    let scene = source.render_scene().unwrap();
    let g = scene.creation.as_ref().unwrap();
    assert!(g.nodes.iter().any(|n| n.name == "Coordonnées stellaires"));
    assert!(g.nodes.iter().any(|n| n.name == "Onde personnelle"));
    let draft = scene
        .creation_draft
        .as_ref()
        .expect("preserve incomplete legacy work");
    assert!(draft.nodes.iter().any(|n| n.name == "Onde personnelle"));
    assert!(draft.compile(&scene).is_err());
    let restored = Scene::from_ron(&scene.to_ron()).unwrap();
    assert_eq!(restored.creation_draft, scene.creation_draft);
    let mut flame = Scene::example_2d();
    let texture = flame.material.texture.clone().unwrap();
    flame.canvas.flame = Some(Flame::default());
    let scene = flame.render_scene().unwrap();
    let g = scene.creation.unwrap();
    assert!(g
        .nodes
        .iter()
        .any(|n| matches!(&n.kind,CreationKind::Texture(t) if **t==texture)));
    assert!(g
        .nodes
        .iter()
        .any(|n| matches!(n.kind, CreationKind::Flame(_))));
    let p = scene.phenomena.unwrap();
    assert!(
        !p.fields
            .nodes
            .iter()
            .any(|n| matches!(n.op, FieldOp::Texture { .. })),
        "latent pattern must not silently alter the image"
    );
}

fn mixed() -> (Scene, CreationGraph) {
    let s = Scene::example_2d();
    let mut g = CreationGraph::default();
    g.settings.domain.resolution = [8, 8, 1];
    let p = g.add(CreationKind::Field(FieldOp::Position), [0.0, 0.0]);
    let motif = g.add(
        CreationKind::Pattern {
            fractal: PlanarFractal::default(),
            channels: OrbitColoring::default(),
        },
        [200.0, 0.0],
    );
    let flame = g.add(
        CreationKind::Flame(FlameField {
            resolution: 32,
            points: 4096,
            ..Default::default()
        }),
        [200.0, 200.0],
    );
    let wave = g.add(
        CreationKind::Field(FieldOp::PlaneWave {
            wavevector: [4.0, 2.0, 0.0].into(),
            frequency: 1.0,
            phase: 0.2,
        }),
        [200.0, 400.0],
    );
    let phase = g.add(CreationKind::Field(FieldOp::Phase), [400.0, 400.0]);
    let palette = g.add(CreationKind::Palette(Ramp::default()), [400.0, 0.0]);
    let product = g.add(CreationKind::Field(FieldOp::Multiply), [600.0, 100.0]);
    let modulation = g.add(CreationKind::Field(FieldOp::Scale), [800.0, 100.0]);
    let output = g.add(CreationKind::Output, [1000.0, 100.0]);
    for (from, to) in [
        ((p, 0), (motif, 0)),
        ((p, 0), (flame, 0)),
        ((p, 0), (wave, 0)),
        ((wave, 0), (phase, 0)),
        ((motif, 0), (palette, 0)),
        ((palette, 0), (product, 0)),
        ((flame, 0), (product, 1)),
        ((product, 0), (modulation, 0)),
        ((phase, 0), (modulation, 1)),
        ((modulation, 0), (output, 1)),
    ] {
        g.connect(from, to).unwrap();
    }
    (s, g)
}
#[test]
fn generators_are_connected_data_not_renderer_modes() {
    let (s, g) = mixed();
    let resolved = g.compile(&s).unwrap();
    resolved.validate().unwrap();
    assert!(resolved.canvas.flame.is_none());
    let p = resolved.phenomena.unwrap();
    assert!(p.bindings.rgb.is_some());
    assert_eq!(p.flames.len(), 1);
    for predicate in [
        |op: &FieldOp| matches!(op, FieldOp::Planar { .. }),
        |op: &FieldOp| matches!(op, FieldOp::Flame { .. }),
        |op: &FieldOp| matches!(op, FieldOp::PlaneWave { .. }),
        |op: &FieldOp| matches!(op, FieldOp::Multiply),
    ] {
        assert!(p.fields.nodes.iter().any(|n| predicate(&n.op)));
    }
}
#[test]
fn rejects_bad_socket_cycles_and_duplicate_destinations() {
    let (s, mut g) = mixed();
    let output = g
        .nodes
        .iter()
        .find(|n| matches!(n.kind, CreationKind::Output))
        .unwrap()
        .id;
    let scalar = g.add(
        CreationKind::Field(FieldOp::Scalar { value: 0.5 }),
        [0.0; 2],
    );
    assert!(g.connect((scalar, 0), (output, 1)).is_err());
    let a = g.add(CreationKind::Field(FieldOp::Add), [0.0; 2]);
    let b = g.add(CreationKind::Field(FieldOp::Add), [0.0; 2]);
    g.connect((a, 0), (b, 0)).unwrap();
    assert!(g.connect((b, 0), (a, 0)).is_err());
    g.compile(&s).unwrap(); // unfinished, unused cards do not invalidate the image
    let l = g.links[0].clone();
    g.links.push(l);
    assert!(g.types().is_err());
}
#[test]
fn geometry_is_lowered_without_numerical_changes() {
    for name in [
        "mandelbox",
        "mandelbulb",
        "menger",
        "menger-kaleido",
        "boite-bulbe",
    ] {
        let s = Scene::example_for(name).unwrap();
        let g = CreationGraph::from_scene(&s);
        let compiled = g.compile(&s).unwrap();
        for i in 0..128 {
            let p = glam::Vec3::new(
                (i as f32 * 0.73).sin() * 2.1,
                (i as f32 * 0.38).cos() * 1.7,
                (i as f32 * 0.59).sin() * 1.9,
            );
            let a = s.fractal.evaluate(p);
            let b = compiled.fractal.evaluate(p);
            assert_eq!(a.distance.to_bits(), b.distance.to_bits(), "{name} {p:?}");
            assert_eq!(a.traps, b.traps);
        }
    }
}
#[test]
fn every_natural_model_migrates_to_the_scene_output() {
    for (name, _) in presets::PRESETS {
        let s = presets::scene(name).unwrap();
        let g = CreationGraph::from_scene(&s);
        let r = g.compile(&s).unwrap_or_else(|e| panic!("{name}: {e}"));
        if s.phenomena.as_ref().unwrap().bindings.geometry.is_some() {
            assert!(
                r.phenomena.as_ref().unwrap().bindings.geometry.is_some(),
                "{name}"
            );
        }
        if !s.phenomena.as_ref().unwrap().simulations.is_empty() {
            assert!(
                !r.phenomena.as_ref().unwrap().simulations.is_empty(),
                "{name}"
            );
        }
    }
}
#[test]
fn canonical_roundtrip_has_one_authoritative_graph() {
    let (mut s, g) = mixed();
    s.creation = Some(g);
    s.unify().unwrap();
    let text = s.to_ron();
    assert!(!text.contains("phenomena:"));
    assert!(!text.contains("fractal: Graph("));
    let restored = Scene::from_ron(&text).unwrap();
    assert_eq!(restored, s);
    restored.validate().unwrap();
    let raw: Scene = ron::from_str(&text).unwrap();
    assert_eq!(raw.render_scene().unwrap(), s);
}
#[test]
fn native_material_texture_remains_exact_and_has_a_graph_connection() {
    let mut s = Scene::example();
    s.material.texture = Some(FractalTexture::default());
    let g = CreationGraph::from_scene(&s);
    let compiled = g.compile(&s).unwrap();
    assert_eq!(compiled.material.texture, s.material.texture);
    let out = g
        .nodes
        .iter()
        .find(|n| matches!(n.kind, CreationKind::Output))
        .unwrap()
        .id;
    assert!(g.input(out, 7).is_some());
}
#[test]
fn a_pattern_retains_mapping_octaves_and_orbit_channels() {
    let mut s = Scene::example_2d();
    let t = s.material.texture.as_mut().unwrap();
    t.octaves = 3;
    t.contrast = 1.4;
    t.rotation = [10.0, 25.0, 19.0].into();
    t.offset = [0.3, -0.2, 0.1].into();
    let compiled = CreationGraph::from_scene(&s).compile(&s).unwrap();
    let p = compiled.phenomena.unwrap();
    let source = p
        .fields
        .nodes
        .iter()
        .find_map(|n| {
            if let FieldOp::Texture { texture } = &n.op {
                Some(texture)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(source.as_ref(), s.material.texture.as_ref().unwrap());
}
#[test]
fn disabled_forms_and_implicit_loop_bodies_survive_migration() {
    let mut s = Scene::example_for("mandelbox").unwrap();
    s.fractal
        .graph_mut()
        .nodes
        .iter_mut()
        .find(|n| matches!(n.kind, NodeKind::BoxFold { .. }))
        .unwrap()
        .enabled = false;
    let r = CreationGraph::from_scene(&s).compile(&s).unwrap();
    for i in 0..80 {
        let p = glam::Vec3::new(i as f32 * 0.037 - 1.5, 0.3, 0.7);
        assert_eq!(s.fractal.evaluate(p), r.fractal.evaluate(p));
    }
}
#[test]
fn snapshot_readers_keep_their_own_coordinates_and_timeline_paths() {
    let mut s = presets::scene("vortex").unwrap();
    // Build a small independent state with two readers at different positions.
    let p = s.phenomena.as_mut().unwrap();
    p.domain.resolution = [8, 8, 1];
    p.fields.nodes.clear();
    p.simulations = vec![Simulation {
        steps: 0,
        ..Default::default()
    }];
    p.bindings = Default::default();
    p.fields.nodes = vec![
        FieldNode {
            id: 0,
            name: "P".into(),
            op: FieldOp::Position,
            inputs: vec![],
            pos: [0.0; 2],
        },
        FieldNode {
            id: 1,
            name: "Q".into(),
            op: FieldOp::Vector {
                value: [0.4, 0.2, 0.0].into(),
            },
            inputs: vec![],
            pos: [0.0; 2],
        },
        FieldNode {
            id: 2,
            name: "A".into(),
            op: FieldOp::Snapshot {
                simulation: 0,
                channel: 0,
            },
            inputs: vec![0],
            pos: [0.0; 2],
        },
        FieldNode {
            id: 3,
            name: "B".into(),
            op: FieldOp::Snapshot {
                simulation: 0,
                channel: 0,
            },
            inputs: vec![1],
            pos: [0.0; 2],
        },
        FieldNode {
            id: 4,
            name: "sum".into(),
            op: FieldOp::Add,
            inputs: vec![2, 3],
            pos: [0.0; 2],
        },
    ];
    p.bindings.color = Some(4);
    p.timeline.tracks = vec![AnimationTrack {
        path: "phenomena.simulations[0].dt".into(),
        interpolation: Interpolation::Linear,
        keys: vec![
            Keyframe {
                time: 0.0,
                value: 0.1,
            },
            Keyframe {
                time: 1.0,
                value: 0.2,
            },
        ],
    }];
    let g = CreationGraph::from_scene(&s);
    assert_eq!(
        g.nodes
            .iter()
            .filter(|n| matches!(n.kind, CreationKind::SampleState { .. }))
            .count(),
        2
    );
    s.creation = Some(g);
    s.unify().unwrap();
    let p = s.phenomena.as_ref().unwrap();
    let readers: Vec<_> = p
        .fields
        .nodes
        .iter()
        .filter(|n| matches!(n.op, FieldOp::Snapshot { .. }))
        .collect();
    assert_eq!(readers.len(), 2);
    assert_ne!(readers[0].inputs, readers[1].inputs);
    let animated = Phenomena::scene_at(&s, 0.5).unwrap();
    assert!((animated.phenomena.unwrap().simulations[0].dt - 0.15).abs() < 1e-6);
}
#[test]
fn rejects_hidden_resources_and_form_replacement_id_collisions() {
    let (s, mut g) = mixed();
    g.settings.flames.push(FlameField::default());
    assert!(g.compile(&s).is_err());
    g.settings.flames.clear();
    g.replace_form(Scene::example_for("mandelbulb").unwrap().fractal.graph());
    g.types().unwrap();
    g.compile(&s).unwrap();
    let ids: std::collections::BTreeSet<_> = g.nodes.iter().map(|n| n.id).collect();
    assert_eq!(ids.len(), g.nodes.len());
}
