use glam::Vec3;
use syfrack_core::{
    Coloring, Fractal, Graph, Link, Measure, Node, NodeKind, OrbitColoring, Ramp, Scene, TrapMode,
};

pub fn orbit_graph() -> Graph {
    let kinds = vec![
        NodeKind::Input,
        NodeKind::Iterate {
            iterations: 3,
            bailout: 100.0,
            body: vec![2, 3, 4, 5, 6],
        },
        NodeKind::Translate { offset: Vec3::X },
        NodeKind::Trap {
            slot: 0,
            measure: Measure::Norm,
            scale: 10.0,
            mode: TrapMode::Minimum,
        },
        NodeKind::Trap {
            slot: 1,
            measure: Measure::Norm,
            scale: 10.0,
            mode: TrapMode::Maximum,
        },
        NodeKind::Trap {
            slot: 2,
            measure: Measure::Norm,
            scale: 10.0,
            mode: TrapMode::Last,
        },
        NodeKind::Trap {
            slot: 3,
            measure: Measure::AngleXY,
            scale: 1.0,
            mode: TrapMode::Last,
        },
        NodeKind::Sphere { radius: 1.0 },
        NodeKind::Output,
    ];
    Graph {
        nodes: kinds
            .into_iter()
            .enumerate()
            .map(|(i, kind)| Node {
                id: i as u32,
                name: kind.label().into(),
                kind,
                pos: [0.0; 2],
                enabled: true,
            })
            .collect(),
        links: [(0, 1), (1, 7), (7, 8)]
            .into_iter()
            .map(|(a, b)| Link {
                from: (a, 0),
                to: (b, 0),
            })
            .collect(),
        safety: 1.0,
        bounding_radius_hint: 5.0,
    }
}

#[test]
fn orbit_accumulation_and_forme_roundtrip() {
    let graph = orbit_graph();
    graph.validate().unwrap();
    assert_eq!(graph.evaluate(Vec3::X).traps, [0.2, 0.4, 0.4, 0.5]);
    let source = syfrack_core::graph::language::format(&graph);
    assert!(source.contains("angle_xy"));
    assert!(source.contains("accumulation=last"));
    let back = syfrack_core::graph::language::compile(&source, Some(&graph)).unwrap();
    assert_eq!(back, graph);
    // Traps observe the orbit but never change the distance field.
    let mut plain = graph.clone();
    for node in &mut plain.nodes {
        if matches!(node.kind, NodeKind::Trap { .. }) {
            node.enabled = false;
        }
    }
    assert_eq!(
        graph.evaluate(Vec3::X).distance,
        plain.evaluate(Vec3::X).distance
    );
}

#[test]
fn rich_palette_and_signed_orbit_mix_survive_scene_roundtrip() {
    let mut scene = Scene::example();
    scene.fractal = Fractal::Graph(orbit_graph());
    let channels = OrbitColoring {
        traps: [1.0, -3.0, 0.5, 0.25],
        iterations: 0.3,
        normal: 0.1,
        logarithmic: 4.0,
    };
    let value = channels.value([0.2, 0.4, 0.4, 0.5], 1.0, 0.0);
    assert!(value.is_finite() && value < 0.0);
    scene.material.coloring = Coloring::Orbit {
        channels,
        ramp: Ramp {
            stops: vec![[0.2, 0.4, 0.8]; 16],
            cyclic: true,
            frequency: 13.0,
            phase: -0.25,
            ..Ramp::default()
        },
    };
    scene.validate().unwrap();
    assert_eq!(Scene::from_ron(&scene.to_ron()).unwrap(), scene);
    scene
        .set_param("material.coloring.orbit.channels.traps[1]", -0.75)
        .unwrap();
    scene
        .set_param("material.coloring.orbit.ramp.frequency", 21.0)
        .unwrap();
    assert!(scene
        .set_param("material.coloring.orbit.channels.logarithmic", -1.0)
        .is_err());
    assert!(scene
        .set_param("material.coloring.orbit.ramp.gamma", 0.0)
        .is_err());
}
