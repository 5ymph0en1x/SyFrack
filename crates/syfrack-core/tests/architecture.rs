use glam::Vec3;
use syfrack_core::{Finish, FinishDetail, FinishPattern, Graph, Link, Node, NodeKind, Scene};

#[test]
fn large_prepared_instance_hierarchy_matches_a_brute_force_union() {
    use glam::{EulerRot, Quat};
    use syfrack_core::{graph::eval::PreparedGraph, Placement};
    let placements: Vec<_> = (0..4097)
        .map(|i| Placement {
            position: Vec3::new(
                (i % 47) as f32 * 1.3 - 30.0,
                ((i / 47) % 47) as f32 * 1.1 - 20.0,
                (i / 2209) as f32 * 1.5,
            ),
            angles: Vec3::new((i % 359) as f32 - 179.0, 19.0, -13.0),
            scale: Vec3::new(0.4, 1.1, 0.7),
        })
        .collect();
    let mut g = Graph::new_minimal();
    let copies = add(
        &mut g,
        NodeKind::Instances {
            placements: placements.clone(),
            half: Vec3::splat(0.8),
            margin: 0.1,
        },
        &[1],
    );
    g.links.retain(|l| l.to != (2, 0));
    g.links.push(Link {
        from: (copies, 0),
        to: (2, 0),
    });
    g.validate().unwrap();
    let prepared = PreparedGraph::new(&g).unwrap();
    for i in 0..107 {
        let p = Vec3::new(
            (i % 17) as f32 * 3.8 - 32.0,
            (i / 17) as f32 * 5.2 - 20.0,
            (i % 7) as f32 * 0.4 - 0.9,
        );
        let expected = placements
            .iter()
            .map(|copy| {
                let a = copy.angles * (std::f32::consts::PI / 180.0);
                let q = (Quat::from_euler(EulerRot::XYZ, a.x, a.y, a.z).conjugate()
                    * (p - copy.position))
                    / copy.scale;
                let fence = (q.abs() - Vec3::splat(0.8)) * copy.scale;
                let bound = fence.max(Vec3::ZERO).length() + fence.max_element().min(0.0);
                ((q.length() - 1.0) * copy.scale.min_element()).max(bound)
            })
            .fold(f32::INFINITY, f32::min);
        assert!(
            (prepared.evaluate(p).distance - expected).abs() < 1e-5,
            "{p}: {} != {expected}",
            prepared.evaluate(p).distance
        );
    }
}

#[test]
fn independent_copies_match_transformed_union_and_roundtrip() {
    use glam::{EulerRot, Quat};
    use syfrack_core::Placement;
    let mut g = Graph::new_minimal();
    g.nodes[1].kind = NodeKind::Torus {
        radius: 1.0,
        tube: 0.2,
    };
    let placements = vec![
        Placement {
            position: Vec3::new(-1.3, 0.2, 0.3),
            angles: Vec3::new(28., 45., -17.),
            scale: Vec3::new(1.4, 0.7, 1.1),
        },
        Placement {
            position: Vec3::new(0.4, -0.2, -0.5),
            angles: Vec3::new(-42., 13., 60.),
            scale: Vec3::new(0.8, 1.2, 1.5),
        },
    ];
    let tag = add(&mut g, NodeKind::MaterialTag { index: 1 }, &[1]);
    let copies = add(
        &mut g,
        NodeKind::Instances {
            placements: placements.clone(),
            half: Vec3::splat(1.5),
            margin: 0.1,
        },
        &[tag],
    );
    g.links.retain(|l| l.to != (2, 0));
    g.links.push(Link {
        from: (copies, 0),
        to: (2, 0),
    });
    g.validate().unwrap();
    for i in 0..729 {
        let p = Vec3::new(
            (i % 9) as f32 * 0.7 - 2.8,
            ((i / 9) % 9) as f32 * 0.7 - 2.8,
            (i / 81) as f32 * 0.7 - 2.8,
        );
        let expected = placements
            .iter()
            .map(|copy| {
                let a = copy.angles * (std::f32::consts::PI / 180.0);
                let q = (Quat::from_euler(EulerRot::XYZ, a.x, a.y, a.z).conjugate()
                    * (p - copy.position))
                    / copy.scale;
                let d =
                    glam::Vec2::new(glam::Vec2::new(q.x, q.z).length() - 1.0, q.y).length() - 0.2;
                let v = (q.abs() - Vec3::splat(1.5)) * copy.scale;
                let bound = v.max(Vec3::ZERO).length() + v.max_element().min(0.0);
                (d * copy.scale.min_element()).max(bound)
            })
            .fold(f32::INFINITY, f32::min);
        let result = g.evaluate(p);
        assert!(
            (result.distance - expected).abs() < 2e-5,
            "{p}: {result:?} {expected}"
        );
        assert_eq!(result.material, 1);
    }
    let source = syfrack_core::graph::language::format(&g);
    let back = syfrack_core::graph::language::compile(&source, Some(&g)).unwrap();
    assert_eq!(g, back);
    g.node_mut(copies).unwrap().enabled = false;
    assert!((g.evaluate(Vec3::X).distance + 0.2).abs() < 1e-6);
}

#[test]
fn independent_copy_nesting_is_bounded_and_positive_scales_are_required() {
    let mut g = Graph::new_minimal();
    let mut last = 1;
    for _ in 0..4 {
        last = add(
            &mut g,
            NodeKind::Instances {
                placements: vec![syfrack_core::Placement::default()],
                half: Vec3::splat(2.),
                margin: 0.1,
            },
            &[last],
        );
    }
    g.links.retain(|l| l.to != (2, 0));
    g.links.push(Link {
        from: (last, 0),
        to: (2, 0),
    });
    assert!(g.validate().is_err());
    g.node_mut(last).unwrap().enabled = false;
    g.validate().unwrap();
    if let NodeKind::Instances { placements, .. } = &mut g.node_mut(last).unwrap().kind {
        placements[0].scale.x = 0.;
    }
    assert!(g.validate().is_err());
}

#[test]
fn finite_repetition_has_exact_copy_counts_and_roundtrips_in_the_form_editor() {
    let mut g = Graph::new_minimal();
    let repeat = add(
        &mut g,
        NodeKind::RepeatFinite {
            period: Vec3::splat(4.0),
            count: [2, 1, 3],
        },
        &[0],
    );
    g.links.retain(|l| l.to != (1, 0));
    g.links.push(Link {
        from: (repeat, 0),
        to: (1, 0),
    });
    g.validate().unwrap();
    for x in [-2.0, 2.0] {
        for z in [-4.0, 0.0, 4.0] {
            let p = Vec3::new(x, 0.0, z);
            assert!((g.evaluate(p).distance + 1.0).abs() < 1e-6);
            assert!(g.evaluate(p + Vec3::Y).distance.abs() < 1e-6);
        }
    }
    assert!((g.evaluate(Vec3::new(6.0, 0.0, 4.0)).distance - 3.0).abs() < 1e-6);
    assert!((g.evaluate(Vec3::new(2.0, 0.0, 8.0)).distance - 3.0).abs() < 1e-6);
    let text = syfrack_core::graph::language::format(&g);
    let back = syfrack_core::graph::language::compile(&text, None).unwrap();
    for p in [
        Vec3::ZERO,
        Vec3::new(2.0, 0.0, 4.0),
        Vec3::new(6.0, 0.0, 4.0),
    ] {
        assert_eq!(g.evaluate(p), back.evaluate(p));
    }
    if let NodeKind::RepeatFinite { count, .. } = &mut g.nodes.last_mut().unwrap().kind {
        count[1] = 0;
    }
    assert!(g.validate().is_err());
}

#[test]
fn personal_finish_textures_are_optional_serializable_and_validated() {
    let mut s = Scene::example();
    s.material.finishes.push(Finish::default());
    let old = s.to_ron().replace("detail: None,", "");
    assert!(Scene::from_ron(&old).unwrap().material.finishes[0]
        .detail
        .is_none());
    for pattern in [
        FinishPattern::VeinedStone,
        FinishPattern::Patina,
        FinishPattern::BrushedMetal,
        FinishPattern::Basalt,
        FinishPattern::Ripples,
        FinishPattern::Rosettes,
        FinishPattern::CoastalRock,
        FinishPattern::Sand,
        FinishPattern::Dust,
    ] {
        s.material.finishes[0].detail = Some(FinishDetail {
            pattern,
            bump: -0.025,
            ..FinishDetail::default()
        });
        s.validate().unwrap();
        assert_eq!(s, Scene::from_ron(&s.to_ron()).unwrap());
    }
    s.material.finishes[0].detail.as_mut().unwrap().scale = 0.0;
    assert!(s.validate().is_err());
}

fn add(g: &mut Graph, kind: NodeKind, inputs: &[u32]) -> u32 {
    let id = g.next_id();
    g.nodes.push(Node {
        id,
        name: kind.label().into(),
        kind,
        pos: [0.0; 2],
        enabled: true,
    });
    for (port, source) in inputs.iter().enumerate() {
        g.links.push(Link {
            from: (*source, 0),
            to: (id, port as u32),
        });
    }
    id
}

#[test]
fn curved_primitives_have_exact_surfaces_and_signed_interiors() {
    let mut g = Graph::new_minimal();
    g.nodes[1].kind = NodeKind::Torus {
        radius: 2.0,
        tube: 0.25,
    };
    for p in [Vec3::new(2.25, 0.0, 0.0), Vec3::new(0.0, 0.25, 2.0)] {
        assert!(g.evaluate(p).distance.abs() < 1e-6);
    }
    assert_eq!(g.evaluate(Vec3::ZERO).distance, 1.75);
    assert_eq!(g.evaluate(Vec3::new(2.0, 0.0, 0.0)).distance, -0.25);
    g.nodes[1].kind = NodeKind::Cylinder {
        radius: 2.0,
        half_height: 3.0,
    };
    assert_eq!(g.evaluate(Vec3::ZERO).distance, -2.0);
    assert_eq!(g.evaluate(Vec3::new(2.0, 3.0, 0.0)).distance, 0.0);
    assert!((g.evaluate(Vec3::new(3.0, 4.0, 0.0)).distance - 2.0f32.sqrt()).abs() < 1e-6);
}

#[test]
fn axis_scaling_is_conservative_and_finish_survives_clipping() {
    let mut g = Graph::new_minimal();
    g.links.clear();
    let scaled = add(
        &mut g,
        NodeKind::ScaleAxes {
            factors: Vec3::new(2.0, 0.5, 1.0),
        },
        &[0],
    );
    g.links.push(Link {
        from: (scaled, 0),
        to: (1, 0),
    });
    let material = add(&mut g, NodeKind::MaterialTag { index: 1 }, &[1]);
    let bounded = add(
        &mut g,
        NodeKind::Bounded {
            half: Vec3::splat(2.0),
            margin: 0.1,
        },
        &[0, material],
    );
    g.links.push(Link {
        from: (bounded, 0),
        to: (2, 0),
    });
    g.validate().unwrap();
    assert_eq!(g.evaluate(Vec3::new(0.5, 0.0, 0.0)).distance, 0.0);
    assert_eq!(g.evaluate(Vec3::new(0.0, 2.0, 0.0)).distance, 0.0);
    let a = g.evaluate(Vec3::new(0.0, 3.0, 0.0));
    assert_eq!(a.distance, 1.0); // the clipping box defines the outer extent
    assert_eq!(a.material, 1);
    let source = syfrack_core::graph::language::format(&g);
    let back = syfrack_core::graph::language::compile(&source, None).unwrap();
    for p in [Vec3::ZERO, Vec3::X, Vec3::splat(3.0)] {
        assert_eq!(g.evaluate(p), back.evaluate(p));
    }
}

#[test]
fn finishes_roundtrip_and_missing_references_are_rejected() {
    let mut scene = Scene::example();
    let old = scene.to_ron().replace("finishes: [],", "");
    assert!(Scene::from_ron(&old).unwrap().material.finishes.is_empty());
    scene.material.finishes.push(Finish {
        name: "Bronze".into(),
        metallic: 1.0,
        emission: [1.0, 0.2, 0.0],
        ..Finish::default()
    });
    scene.validate().unwrap();
    assert_eq!(scene, Scene::from_ron(&scene.to_ron()).unwrap());
    scene.material.finishes[0].emission[0] = -1.0;
    assert!(scene.validate().is_err());
    scene.material.finishes[0].emission[0] = 0.0;
    scene.fractal = syfrack_core::Fractal::Graph(Graph::new_minimal());
    let g = scene.fractal.graph_mut();
    g.links.retain(|l| l.to.0 != 2);
    let tag = add(g, NodeKind::MaterialTag { index: 2 }, &[1]);
    g.links.push(Link {
        from: (tag, 0),
        to: (2, 0),
    });
    assert!(scene.validate().is_err());
    let clay = syfrack_core::clay_scene(&scene);
    clay.validate().unwrap();
    assert_eq!(clay.fractal, scene.fractal);
    assert!(clay
        .material
        .finishes
        .iter()
        .all(|f| f.emission == [0.0; 3]));
}

#[test]
fn local_reinjection_is_invariant_to_translation_and_tracks_the_seed_derivative() {
    let mut g = Graph::new_minimal();
    g.nodes[1].kind = NodeKind::NormFence { inset: 1.0 };
    g.links.clear();
    let scale = add(&mut g, NodeKind::Scale { factor: 2.0 }, &[]);
    let seed = add(
        &mut g,
        NodeKind::AddSeed {
            multiplier: Vec3::ONE,
        },
        &[],
    );
    let loop_id = add(
        &mut g,
        NodeKind::Iterate {
            iterations: 3,
            bailout: 1000.0,
            body: vec![scale, seed],
        },
        &[0],
    );
    g.links.extend([
        Link {
            from: (loop_id, 0),
            to: (1, 0),
        },
        Link {
            from: (1, 0),
            to: (2, 0),
        },
    ]);
    g.validate().unwrap();
    // z_(n+1) = 2 z_n + seed, hence z_3 = 15 seed and dr_3 = 15.
    let p = Vec3::new(0.1, 0.2, 0.3);
    assert!((g.evaluate(p).distance - (p.length() - 1.0 / 15.0)).abs() < 1e-6);
    let mut shifted = g.clone();
    let center = Vec3::new(4.0, 3.0, -2.0);
    let t = add(&mut shifted, NodeKind::Translate { offset: -center }, &[0]);
    shifted.links.retain(|l| l.to != (loop_id, 0));
    shifted.links.push(Link {
        from: (t, 0),
        to: (loop_id, 0),
    });
    assert!((shifted.evaluate(p + center).distance - g.evaluate(p).distance).abs() < 1e-6);
    let axis = add(
        &mut shifted,
        NodeKind::ScaleAxes {
            factors: Vec3::splat(2.0),
        },
        &[t],
    );
    shifted.links.retain(|l| l.to != (loop_id, 0));
    shifted.links.push(Link {
        from: (axis, 0),
        to: (loop_id, 0),
    });
    assert!(
        (shifted.evaluate(p + center).distance - g.evaluate(p * 2.0).distance / 2.0).abs() < 1e-6
    );
}

#[test]
fn nested_loops_capture_their_own_local_seed() {
    let mut g = Graph::new_minimal();
    g.nodes[1].kind = NodeKind::NormFence { inset: 1.0 };
    g.links.clear();
    let scale = add(&mut g, NodeKind::Scale { factor: 2.0 }, &[]);
    let seed = add(
        &mut g,
        NodeKind::AddSeed {
            multiplier: Vec3::ONE,
        },
        &[],
    );
    let inner = add(
        &mut g,
        NodeKind::Iterate {
            iterations: 2,
            bailout: 1000.0,
            body: vec![seed],
        },
        &[],
    );
    let outer = add(
        &mut g,
        NodeKind::Iterate {
            iterations: 2,
            bailout: 1000.0,
            body: vec![scale, inner],
        },
        &[0],
    );
    g.links.extend([
        Link {
            from: (outer, 0),
            to: (1, 0),
        },
        Link {
            from: (1, 0),
            to: (2, 0),
        },
    ]);
    g.validate().unwrap();
    // Each inner loop adds its own entry point twice: outer effect is 6x,
    // twice, so the distance is |p| - 1/36.
    assert!((g.evaluate(Vec3::X * 0.04).distance - (0.04 - 1.0 / 36.0)).abs() < 1e-6);
}
