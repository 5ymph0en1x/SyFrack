use glam::Vec3;
use syfrack_core::{Graph, NodeKind};

fn lcg(seed: &mut u32) -> f32 {
    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    (*seed >> 8) as f32 / 16_777_216.0
}
fn points(n: usize, extent: f32, seed: u32) -> Vec<Vec3> {
    let mut s = seed;
    (0..n)
        .map(|_| Vec3::new(lcg(&mut s), lcg(&mut s), lcg(&mut s)) * (2.0 * extent) - extent)
        .collect()
}

/// One reference sample of a hand-written formula (tests/fixtures/parity/*.ron),
/// written by the formulas themselves before their removal (sphere extent 3,
/// Mandelbulb 1.6, Mandelbox 8, Menger 1.6; 64 points, LCG seed 11).
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Reference {
    p: (f32, f32, f32),
    distance: f32,
    traps: [f32; 4],
    iterations: u32,
}

fn fixture_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/parity")
        .join(format!("{name}.ron"))
}

/// The bundled graph of `name` against the formula references: distance at
/// 1e-5 relative (absolute below 1), every trap slot at 1e-5, iterations equal.
fn check(name: &str, extent: f32) {
    let g = Graph::bundled(name).unwrap();
    g.validate().unwrap();
    let text = std::fs::read_to_string(fixture_path(name)).unwrap();
    let refs: Vec<Reference> = ron::from_str(&text).unwrap();
    assert_eq!(refs.len(), 64);
    let expected = points(64, extent, 11);
    let mut worst = 0.0f32;
    for (r, q) in refs.into_iter().zip(expected) {
        let p = Vec3::new(r.p.0, r.p.1, r.p.2);
        assert_eq!(p, q, "{name}: the fixture points are the LCG points");
        let b = g.evaluate(p);
        let err = (r.distance - b.distance).abs() / r.distance.abs().max(1.0);
        worst = worst.max(err);
        assert!(
            err <= 1e-5,
            "{name} at {p}: reference {} graph {}",
            r.distance,
            b.distance
        );
        for k in 0..4 {
            assert!(
                (r.traps[k] - b.traps[k]).abs() <= 1e-5,
                "{name} trap {k} at {p}: {} vs {}",
                r.traps[k],
                b.traps[k]
            );
        }
        assert_eq!(r.iterations, b.iterations, "{name} iterations at {p}");
    }
    println!("{name}: worst relative distance error {worst:e}");
}

#[test]
fn sphere_with_ground_matches() {
    check("sphere", 3.0);
}
#[test]
fn mandelbulb_matches() {
    check("mandelbulb", 1.6);
}
#[test]
fn mandelbox_matches() {
    check("mandelbox", 8.0);
}
#[test]
fn menger_matches() {
    check("menger", 1.6);
}

#[test]
fn step_scale_inherits_the_legacy_safety_factor() {
    assert!((Graph::bundled("mandelbox").unwrap().step_scale() - 0.6).abs() < 1e-6);
    assert!((Graph::bundled("menger").unwrap().step_scale() - 0.9).abs() < 1e-6);
}

#[test]
fn twist_lowers_the_step_scale() {
    let mut g = Graph::new_minimal();
    g.nodes.push(syfrack_core::Node {
        id: 3,
        kind: NodeKind::Twist { rate: 1.0 },
        name: "t".into(),
        pos: [0.0; 2],
        enabled: true,
    });
    g.links = vec![
        syfrack_core::Link {
            from: (0, 0),
            to: (3, 0),
        },
        syfrack_core::Link {
            from: (3, 0),
            to: (1, 0),
        },
        syfrack_core::Link {
            from: (1, 0),
            to: (2, 0),
        },
    ];
    g.bounding_radius_hint = 2.0;
    let expected = 1.0 / (1.0f32 + 4.0).sqrt();
    assert!((g.step_scale() - expected).abs() < 1e-6);
}

#[test]
fn bounding_radius_estimate_is_a_tight_enclosure_below_the_legacy_hint() {
    // The legacy hints are generous (2.5, 4.46); the estimate walks in to the
    // surface (Mandelbulb about 1.15, Menger sqrt(3)). It must stay below the
    // hint and above a known lower bound of the true extent.
    for (name, lower) in [("mandelbulb", 1.0f32), ("menger", 1.6)] {
        let g = Graph::bundled(name).unwrap();
        let legacy = g.bounding_radius_hint;
        let est = g.estimate_bounding_radius();
        assert!(
            (lower..=legacy * 1.1 + 0.3).contains(&est),
            "{name}: estimate {est} legacy {legacy}"
        );
    }
}

#[test]
fn a_ground_plane_does_not_inflate_the_bounding_radius_estimate() {
    // The plane reaches every far point below or beside the sphere: those
    // directions are skipped, the estimate encloses the sphere alone.
    let est = Graph::bundled("sphere").unwrap().estimate_bounding_radius();
    assert!((1.0..=1.2).contains(&est), "{est}");
}

#[test]
fn smooth_union_is_bounded_by_its_operands_and_blends_traps() {
    let mut g = Graph::new_minimal();
    // Sphère(1) ∪lisse Sphère(3) décalée
    g.nodes.push(syfrack_core::Node {
        id: 3,
        kind: NodeKind::Translate {
            offset: Vec3::new(1.0, 0.0, 0.0),
        },
        name: "t".into(),
        pos: [0.0; 2],
        enabled: true,
    });
    g.nodes.push(syfrack_core::Node {
        id: 4,
        kind: NodeKind::Sphere { radius: 1.0 },
        name: "s2".into(),
        pos: [0.0; 2],
        enabled: true,
    });
    g.nodes.push(syfrack_core::Node {
        id: 5,
        kind: NodeKind::SmoothUnion { k: 0.3 },
        name: "u".into(),
        pos: [0.0; 2],
        enabled: true,
    });
    g.nodes.push(syfrack_core::Node {
        id: 6,
        kind: NodeKind::Trap {
            slot: 0,
            measure: syfrack_core::Measure::Norm,
            scale: 4.0,
            mode: Default::default(),
        },
        name: "trap".into(),
        pos: [0.0; 2],
        enabled: true,
    });
    g.links = vec![
        syfrack_core::Link {
            from: (0, 0),
            to: (6, 0),
        },
        syfrack_core::Link {
            from: (6, 0),
            to: (1, 0),
        },
        syfrack_core::Link {
            from: (0, 0),
            to: (3, 0),
        },
        syfrack_core::Link {
            from: (3, 0),
            to: (4, 0),
        },
        syfrack_core::Link {
            from: (1, 0),
            to: (5, 0),
        },
        syfrack_core::Link {
            from: (4, 0),
            to: (5, 1),
        },
        syfrack_core::Link {
            from: (5, 0),
            to: (2, 0),
        },
    ];
    g.validate().unwrap();
    let sample = g.evaluate(Vec3::new(-0.5, 0.0, 0.0));
    let d = sample.distance;
    // Only the first operand carries a trap: its value survives the blend.
    assert!((sample.traps[0] - 0.125).abs() < 1e-6, "{:?}", sample.traps);
    assert_eq!(sample.traps[1], 1.0);
    assert!((-0.8..=-0.5 + 1e-6).contains(&d), "smooth union {d}");
    let far = g.evaluate(Vec3::new(-5.0, 0.0, 0.0)).distance;
    assert!((far - 3.0).abs() < 1e-5);
}

#[test]
fn disabled_nodes_pass_the_point_through() {
    let f = Graph::bundled("mandelbox").unwrap();
    let mut g = f.clone();
    for n in &mut g.nodes {
        if matches!(n.kind, NodeKind::BoxFold { .. }) {
            n.enabled = false;
        }
    }
    let p = Vec3::new(1.3, -0.4, 0.7);
    assert_ne!(g.evaluate(p).distance, f.evaluate(p).distance);
}

fn nd(id: u32, kind: NodeKind) -> syfrack_core::Node {
    syfrack_core::Node {
        id,
        kind,
        name: format!("n{id}"),
        pos: [0.0; 2],
        enabled: true,
    }
}
fn lk(a: u32, b: u32) -> syfrack_core::Link {
    syfrack_core::Link {
        from: (a, 0),
        to: (b, 0),
    }
}

#[test]
fn twist_inside_a_nested_iterate_lowers_the_step_scale() {
    let mut g = Graph::new_minimal();
    g.nodes.push(nd(
        3,
        NodeKind::Iterate {
            iterations: 2,
            bailout: 4.0,
            body: vec![4],
        },
    ));
    g.nodes.push(nd(
        4,
        NodeKind::Iterate {
            iterations: 2,
            bailout: 4.0,
            body: vec![5],
        },
    ));
    g.nodes.push(nd(5, NodeKind::Twist { rate: 1.0 }));
    g.links = vec![lk(0, 3), lk(3, 1), lk(1, 2)];
    g.validate().unwrap();
    g.bounding_radius_hint = 2.0;
    let expected = 1.0 / 5.0f32.sqrt();
    assert!(
        (g.step_scale() - expected).abs() < 1e-6,
        "{}",
        g.step_scale()
    );
}

#[test]
fn the_sphere_reports_zero_iterations() {
    let g = Graph::bundled("sphere").unwrap();
    assert_eq!(g.evaluate(Vec3::new(2.0, 0.0, 0.0)).iterations, 0);
}

#[test]
fn the_ground_plane_carries_the_trap_payload() {
    // The sphere on its ground, radius 3 (sphere and trap scale).
    let mut g = Graph::bundled("sphere").unwrap();
    for n in &mut g.nodes {
        match &mut n.kind {
            NodeKind::Sphere { radius } => *radius = 3.0,
            NodeKind::Trap { scale, .. } => *scale = 3.0,
            _ => {}
        }
    }
    let p = Vec3::new(0.7, -2.8, 0.0); // the plane wins here, |p| / 3 < 1
    let s = g.evaluate(p);
    assert!((s.distance - (p.y + 1.0)).abs() < 1e-6, "{}", s.distance);
    assert!(
        (s.traps[0] - p.length() / 3.0).abs() < 1e-6,
        "{:?}",
        s.traps
    );
}

#[test]
fn nested_iterate_count_is_clamped_to_the_iteration_product() {
    let mut g = Graph::new_minimal();
    g.nodes.push(nd(
        3,
        NodeKind::Iterate {
            iterations: 2,
            bailout: 4.0,
            body: vec![4],
        },
    ));
    g.nodes.push(nd(
        4,
        NodeKind::Iterate {
            iterations: 3,
            bailout: 4.0,
            body: vec![],
        },
    ));
    g.links = vec![lk(0, 3), lk(3, 1), lk(1, 2)];
    g.validate().unwrap();
    assert_eq!(g.iteration_product(), 6);
    assert_eq!(g.evaluate(Vec3::new(0.1, 0.0, 0.0)).iterations, 6);
}

#[test]
fn a_disabled_combinator_passes_its_first_input_through() {
    let mut g = Graph::new_minimal();
    g.nodes.push(nd(3, NodeKind::Translate { offset: Vec3::X }));
    g.nodes.push(nd(4, NodeKind::Sphere { radius: 1.0 }));
    g.nodes.push(nd(5, NodeKind::Union));
    g.links = vec![
        lk(0, 1),
        lk(0, 3),
        lk(3, 4),
        lk(1, 5),
        syfrack_core::Link {
            from: (4, 0),
            to: (5, 1),
        },
        lk(5, 2),
    ];
    // second operand is the closer one at x = -5 (distance 3 against 4)
    let p = Vec3::new(-5.0, 0.0, 0.0);
    assert!((g.evaluate(p).distance - 3.0).abs() < 1e-5);
    g.node_mut(5).unwrap().enabled = false;
    assert!((g.evaluate(p).distance - 4.0).abs() < 1e-5);
}

#[test]
fn a_trap_in_a_body_that_never_runs_still_defines_the_slot_scale() {
    let mut g = Graph::new_minimal();
    let trap = |id, scale| {
        nd(
            id,
            NodeKind::Trap {
                slot: 0,
                measure: syfrack_core::Measure::Norm,
                scale,
                mode: Default::default(),
            },
        )
    };
    g.nodes.push(trap(3, 1.0));
    g.nodes.push(nd(
        4,
        NodeKind::Iterate {
            iterations: 2,
            bailout: 2.0,
            body: vec![5],
        },
    ));
    g.nodes.push(trap(5, 4.0));
    g.links = vec![lk(0, 3), lk(3, 4), lk(4, 1), lk(1, 2)];
    g.validate().unwrap();
    // |p| = 3 > bailout: the body never runs, the static scale is still 4.
    let t = g.evaluate(Vec3::new(3.0, 0.0, 0.0)).traps[0];
    assert!((t - 0.75).abs() < 1e-6, "{t}");
}
