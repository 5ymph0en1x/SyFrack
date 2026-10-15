//! CPU/GPU consistency harness: the WGSL generated from a form graph against
//! the CPU interpreter (`Graph::evaluate`) on 4096 points.

use glam::Vec3;
use std::sync::Arc;
use syfrack_core::{Fractal, Graph, Link, Measure, Node, NodeKind, Scene};
use syfrack_render::{GpuContext, Renderer};

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

fn node(id: u32, kind: NodeKind) -> Node {
    Node {
        id,
        name: kind.label().into(),
        kind,
        pos: [0.0; 2],
        enabled: true,
    }
}

fn l(a: u32, b: u32) -> Link {
    Link {
        from: (a, 0),
        to: (b, 0),
    }
}

/// Samples 4096 points in [-extent, extent]^3 and counts the points away
/// from the set (|d| >= 0.02) where the distance or a trap disagrees.
fn check(name: &str, g: Graph, extent: f32) {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = Scene::example();
    scene.fractal = Fractal::Graph(g.clone());
    let pts = points(4096, extent, 11);
    let gpu = r.probe_distances(&scene, &pts).unwrap();
    assert_eq!(gpu.len(), pts.len());
    let (mut considered, mut bad, mut d_bad, mut t_bad) = (0usize, 0usize, 0usize, 0usize);
    let mut worst_d = 0.0f32;
    let mut worst_t = 0.0f32;
    for (p, s) in pts.iter().zip(&gpu) {
        let c = g.evaluate(*p);
        assert!(
            s.distance.is_finite(),
            "{name}: gpu distance not finite at {p}"
        );
        if c.distance.abs() < 0.02 {
            continue;
        }
        considered += 1;
        let d_err = (s.distance - c.distance).abs();
        let d_off = d_err > 1e-3 + 0.02 * c.distance.abs();
        let t_err = (0..4)
            .map(|i| (s.traps[i] - c.traps[i]).abs())
            .fold(0.0f32, f32::max);
        let t_off = t_err > 0.02;
        worst_d = worst_d.max(d_err / (1e-3 + 0.02 * c.distance.abs()));
        worst_t = worst_t.max(t_err);
        d_bad += d_off as usize;
        t_bad += t_off as usize;
        if d_off || t_off {
            bad += 1;
            eprintln!(
                "{name} mismatch at {p}: distance cpu {} gpu {}, traps cpu {:?} gpu {:?}",
                c.distance, s.distance, c.traps, s.traps
            );
        }
        assert_eq!(s.iterations, c.iterations, "{name}: iterations at {p}");
    }
    eprintln!(
        "HARNESS {name}: considered {considered}/{}, off {bad} (distance {d_bad}, traps {t_bad}), worst distance error {worst_d:.2e} of tolerance, worst trap error {worst_t:.2e}",
        pts.len()
    );
    assert!(
        considered > 500,
        "{name}: only {considered} points away from the set"
    );
    assert!(
        bad * 1000 <= considered * 5,
        "{name}: {bad}/{considered} points off"
    );
}

#[test]
#[ignore = "needs GPU"]
fn sphere_graph_matches_cpu() {
    check("sphere", Graph::bundled("sphere").unwrap(), 3.0);
}

#[test]
#[ignore = "needs GPU"]
fn spherical_integer_powers_match_cpu_and_keep_polar_filaments() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut g = Graph::new_minimal();
    g.nodes.push(node(3, NodeKind::Power { power: 8.0 }));
    let mut links = vec![l(0, 3)];
    for (i, measure) in [
        Measure::AxisZ,
        Measure::AxisX,
        Measure::PlaneY,
        Measure::AngleXY,
    ]
    .into_iter()
    .enumerate()
    {
        let id = 4 + i as u32;
        g.nodes.push(node(
            id,
            NodeKind::Trap {
                slot: i as u32,
                measure,
                scale: 1.0,
                mode: Default::default(),
            },
        ));
        links.push(l(id - 1, id));
    }
    links.extend([l(7, 1), l(1, 2)]);
    g.links = links;
    let mut pts = points(2048, 1.4, 191);
    pts.extend([
        Vec3::Z,
        -Vec3::Z,
        Vec3::new(1e-6, 0.0, 1.0),
        Vec3::new(-1e-6, 2e-6, -1.0),
    ]);
    for power in [2.0, 3.0, 6.0, 8.0, 16.0, 6.25] {
        if let NodeKind::Power { power: p } = &mut g.node_mut(3).unwrap().kind {
            *p = power;
        }
        let mut scene = Scene::example();
        scene.fractal = Fractal::Graph(g.clone());
        let gpu = r.probe_distances(&scene, &pts).unwrap();
        for (&p, s) in pts.iter().zip(&gpu) {
            let c = g.evaluate(p);
            assert!(s.distance.is_finite());
            assert!(
                (s.distance - c.distance).abs() <= 1e-4 + 2e-5 * c.distance.abs(),
                "power={power}, p={p}, cpu={c:?}, gpu={s:?}"
            );
            // Raw angular-power rounding is amplified by r^power. A trap's
            // x/(1+x) mapping is 1-Lipschitz, while azimuth is ill-conditioned
            // where the powered point approaches the polar axis.
            let tol = 2e-5 + 1.2e-5 * p.length().powf(power);
            let rho = c.traps[0] / (1.0 - c.traps[0]).max(1e-20);
            for i in 0..4 {
                if i == 3 && rho < 64.0 * tol {
                    continue;
                }
                // AngleXY wraps at +/-pi; those endpoints represent one angle.
                let e = (s.traps[i] - c.traps[i]).abs();
                let bound = if i == 3 { 2e-5 + 2.0 * tol / rho } else { tol };
                assert!(
                    e < bound || (i == 3 && (1.0 - e).abs() < bound),
                    "power={power}, p={p}, trap={i}, cpu={}, gpu={}, bound={bound}",
                    c.traps[i],
                    s.traps[i]
                );
            }
        }
        if power == 8.0 {
            assert!(
                gpu[pts.len() - 2].traps[0] > 6e-6,
                "polar filament collapsed"
            );
        }
    }
}

#[test]
#[ignore = "needs GPU"]
fn hybrid_operators_match_cpu_including_inversion_core() {
    let mut g = Graph::new_minimal();
    g.nodes.extend([
        node(
            3,
            NodeKind::BoxFoldAxes {
                limit: Vec3::new(0.7, 1.2, 0.4),
                x: true,
                y: false,
                z: true,
            },
        ),
        node(
            4,
            NodeKind::SphereInversion {
                radius: 1.3,
                min_radius: 0.4,
            },
        ),
        node(
            5,
            NodeKind::AddConstant {
                multiplier: Vec3::new(-0.4, 0.0, 1.2),
                julia: Vec3::new(0.3, -0.2, 0.1),
            },
        ),
        node(
            6,
            NodeKind::Trap {
                slot: 0,
                measure: Measure::Norm,
                scale: 4.0,
                mode: Default::default(),
            },
        ),
    ]);
    g.links = vec![l(0, 3), l(3, 4), l(4, 5), l(5, 6), l(6, 1), l(1, 2)];
    check("hybrid-operators", g, 2.0);
}

#[test]
#[ignore = "needs GPU"]
fn hybrid_bundled_forms_match_cpu() {
    for name in ["rosace-plis", "dentelle-inversee", "menger-torsade"] {
        check(name, Graph::bundled(name).unwrap(), 6.0);
    }
}

#[test]
#[ignore = "needs GPU"]
fn mandelbulb_graph_matches_cpu() {
    check("mandelbulb", Graph::bundled("mandelbulb").unwrap(), 1.6);
}

#[test]
#[ignore = "needs GPU"]
fn mandelbox_graph_matches_cpu() {
    check("mandelbox", Graph::bundled("mandelbox").unwrap(), 8.0);
}

#[test]
#[ignore = "needs GPU"]
fn menger_graph_matches_cpu() {
    check("menger", Graph::bundled("menger").unwrap(), 1.6);
}

#[test]
#[ignore = "needs GPU"]
fn rotated_box_matches_cpu() {
    let mut g = Graph::new_minimal();
    g.nodes[1].kind = NodeKind::Box {
        half: Vec3::new(1.0, 0.5, 0.25),
    };
    g.nodes.push(node(
        3,
        NodeKind::Rotate {
            angles: Vec3::new(30.0, 45.0, 10.0),
        },
    ));
    g.links = vec![l(0, 3), l(3, 1), l(1, 2)];
    check("rotated-box", g, 3.0);
}

#[test]
#[ignore = "needs GPU"]
fn twisted_kaleidoscopic_menger_with_smooth_union_matches_cpu() {
    let mut g = Graph::bundled("menger").unwrap(); // 0 in, 1 iterate{2,3}, 4 box, 5 out
    g.nodes.push(node(6, NodeKind::Twist { rate: 0.3 }));
    g.nodes.push(node(
        7,
        NodeKind::Kaleido {
            planes: 5,
            phase: 10.0,
        },
    ));
    g.nodes.push(node(8, NodeKind::Sphere { radius: 1.2 }));
    g.nodes.push(node(9, NodeKind::SmoothUnion { k: 0.2 }));
    g.links = vec![
        l(0, 6),
        l(6, 7),
        l(7, 1),
        l(1, 4),
        l(0, 8),
        l(4, 9),
        Link {
            from: (8, 0),
            to: (9, 1),
        },
        l(9, 5),
    ];
    g.bounding_radius_hint = 4.0;
    check("twist-kaleido-menger", g, 2.0);
}

#[test]
#[ignore = "needs GPU"]
fn tiled_mirrored_bulb_minus_box_matches_cpu() {
    let mut g = Graph::bundled("mandelbulb").unwrap(); // 0 in, 1 iterate{2,3,4}, 5 bulb, 6 out
    g.nodes.push(node(
        7,
        NodeKind::Tile {
            period: Vec3::splat(6.0),
        },
    ));
    g.nodes.push(node(
        8,
        NodeKind::Mirror {
            normal: Vec3::new(1.0, 0.0, 1.0),
            offset: 0.2,
        },
    ));
    g.nodes.push(node(
        9,
        NodeKind::Box {
            half: Vec3::new(0.4, 2.0, 0.4),
        },
    ));
    g.nodes.push(node(10, NodeKind::Subtraction));
    g.links = vec![
        l(0, 7),
        l(7, 8),
        l(8, 1),
        l(1, 5),
        l(7, 9),
        l(5, 10),
        Link {
            from: (9, 0),
            to: (10, 1),
        },
        l(10, 6),
    ];
    check("tile-mirror-bulb-minus-box", g, 3.0);
}

#[test]
#[ignore = "needs GPU"]
fn rounded_box_matches_cpu_including_zero_and_clamped_radius() {
    for radius in [0.0, 0.12, 2.0] {
        let mut g = Graph::new_minimal();
        g.nodes[1].kind = NodeKind::RoundBox {
            half: Vec3::new(0.7, 1.2, 0.4),
            radius,
        };
        check("rounded-box", g, 2.0);
    }
}

#[test]
#[ignore = "needs GPU"]
fn changing_a_scalar_reuses_the_pipeline_and_changing_topology_does_not() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let a = Graph::bundled("mandelbox").unwrap();
    let pa = r.pipeline_for(&a).unwrap();
    let mut b = a.clone();
    for n in &mut b.nodes {
        if let NodeKind::Scale { factor } = &mut n.kind {
            *factor = 2.5;
        }
    }
    assert!(Arc::ptr_eq(&pa, &r.pipeline_for(&b).unwrap()));
    let mut c = a.clone();
    c.nodes[2].enabled = false;
    assert!(!Arc::ptr_eq(&pa, &r.pipeline_for(&c).unwrap()));
}

#[test]
#[ignore = "needs GPU"]
fn probing_no_points_returns_nothing() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let scene = Scene::example_for("mandelbulb").unwrap();
    assert!(r.probe_distances(&scene, &[]).unwrap().is_empty());
}

/// Every remaining node kind, nested Repeat, trap slots 1-3 with axis
/// measures, smooth blends of a trapped and an untrapped branch (empty slots),
/// a disabled fence under a Union, a disabled combination and a disabled
/// Point → Point node.
#[test]
#[ignore = "needs GPU"]
fn nested_traps_and_disabled_nodes_match_cpu() {
    let trap = |slot, measure, scale| NodeKind::Trap {
        slot,
        measure,
        scale,
        mode: Default::default(),
    };
    let mut nodes = vec![
        node(0, NodeKind::Input),
        node(
            10,
            NodeKind::Translate {
                offset: Vec3::new(0.1, 0.2, -0.1),
            },
        ),
        node(11, NodeKind::Bend { rate: 0.2 }),
        node(30, trap(3, Measure::PlaneY, 1.5)),
        node(
            12,
            NodeKind::Iterate {
                iterations: 4,
                bailout: 8.0,
                body: vec![13, 14, 15],
            },
        ),
        node(
            13,
            NodeKind::Iterate {
                iterations: 3,
                bailout: 8.0,
                body: vec![16, 17],
            },
        ),
        node(
            16,
            NodeKind::Abs {
                x: true,
                y: false,
                z: true,
            },
        ),
        node(17, trap(2, Measure::AxisX, 2.0)),
        node(14, NodeKind::Scale { factor: 1.3 }),
        node(15, trap(1, Measure::AxisZ, 3.0)),
        node(20, NodeKind::Sphere { radius: 1.0 }),
        node(
            21,
            NodeKind::Plane {
                normal: Vec3::new(0.0, 1.0, 0.2),
                offset: -1.5,
            },
        ),
        node(22, NodeKind::SmoothUnion { k: 0.3 }),
        node(23, NodeKind::NormFence { inset: 1.8 }),
        node(24, NodeKind::SmoothIntersection { k: 0.2 }),
        node(
            26,
            NodeKind::Box {
                half: Vec3::splat(0.5),
            },
        ),
        node(25, NodeKind::Union),
        node(29, NodeKind::Twist { rate: 0.5 }),
        node(28, NodeKind::Sphere { radius: 0.3 }),
        node(27, NodeKind::Subtraction),
        node(31, NodeKind::Output),
    ];
    for n in &mut nodes {
        if [26, 27, 29].contains(&n.id) {
            n.enabled = false;
        }
    }
    let port1 = |a, b| Link {
        from: (a, 0),
        to: (b, 1),
    };
    let g = Graph {
        nodes,
        links: vec![
            l(0, 10),
            l(10, 11),
            l(11, 30),
            l(30, 12),
            l(12, 20),
            l(0, 21),
            l(20, 22),
            port1(21, 22),
            l(10, 23),
            l(22, 24),
            port1(23, 24),
            l(11, 26),
            l(24, 25),
            port1(26, 25),
            l(0, 29),
            l(29, 28),
            l(25, 27),
            port1(28, 27),
            l(27, 31),
        ],
        safety: 0.8,
        bounding_radius_hint: 4.0,
    };
    g.validate().unwrap();
    check("nested-traps-disabled", g, 3.0);
}

/// A disabled Box (empty: 1e30 on the GPU, +inf on the CPU) on port 1 of a
/// SmoothUnion: the union is the other side, distance and payload.
/// Axis traps are distances to the axes (`AxisY = length(p.xz)`), unlike
/// `PlaneY = |p.y|`: a Mandelbox trapping AxisY on slot 0 and AxisX on slot 1.
#[test]
#[ignore = "needs GPU"]
fn axis_traps_match_cpu() {
    let mut g = Graph::bundled("mandelbox").unwrap();
    for n in &mut g.nodes {
        if let NodeKind::Trap { measure, .. } = &mut n.kind {
            *measure = Measure::AxisY;
        }
        if let NodeKind::Iterate { body, .. } = &mut n.kind {
            body.insert(4, 9);
        }
    }
    g.nodes.push(node(
        9,
        NodeKind::Trap {
            slot: 1,
            measure: Measure::AxisX,
            scale: 3.0,
            mode: Default::default(),
        },
    ));
    g.validate().unwrap();
    check("axis-traps", g, 8.0);
}

#[test]
#[ignore = "needs GPU"]
fn smooth_union_with_a_disabled_box_matches_cpu() {
    let mut g = Graph::bundled("mandelbulb").unwrap(); // 0 in, 1 iterate{2,3,4}, 5 bulb, 6 out
    let mut bx = node(
        7,
        NodeKind::Box {
            half: Vec3::splat(0.5),
        },
    );
    bx.enabled = false;
    g.nodes.push(bx);
    g.nodes.push(node(8, NodeKind::SmoothUnion { k: 0.3 }));
    g.links = vec![
        l(0, 1),
        l(1, 5),
        l(0, 7),
        l(5, 8),
        Link {
            from: (7, 0),
            to: (8, 1),
        },
        l(8, 6),
    ];
    g.validate().unwrap();
    check("bulb-smooth-union-disabled-box", g, 1.6);
}

/// A mirror normal far below the old 1e-8 cut-off still normalises, as
/// glam's `try_normalize` does on the CPU.
#[test]
#[ignore = "needs GPU"]
fn mirror_with_a_tiny_normal_matches_cpu() {
    let mut g = Graph::new_minimal();
    g.nodes[1].kind = NodeKind::Box {
        half: Vec3::new(1.0, 0.5, 0.25),
    };
    g.nodes.push(node(
        3,
        NodeKind::Mirror {
            normal: Vec3::new(1e-9, 0.0, 0.0),
            offset: 0.3,
        },
    ));
    g.links = vec![l(0, 3), l(3, 1), l(1, 2)];
    g.validate().unwrap();
    check("tiny-mirror-normal", g, 3.0);
}

#[test]
#[ignore = "needs GPU"]
fn pipeline_for_validates_the_graph_even_on_a_cache_hit() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let a = Graph::bundled("mandelbox").unwrap();
    r.pipeline_for(&a).unwrap();
    let mut bad = a.clone();
    for n in &mut bad.nodes {
        if let NodeKind::Scale { factor } = &mut n.kind {
            *factor = 0.0; // same topology, invalid value
        }
    }
    assert!(r.pipeline_for(&bad).is_err());
}

#[test]
#[ignore = "needs GPU"]
fn a_shader_error_names_the_node() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut g = Graph::new_minimal();
    g.nodes[1].kind = NodeKind::Box {
        half: Vec3::splat(1.0),
    };
    g.nodes[1].name = "Ma boîte".into();
    let mut c = syfrack_render::graph_wgsl::compile(&g).unwrap();
    c.wgsl = c.wgsl.replace("box_distance(", "box_distanc(");
    let err = syfrack_render::CompiledPipeline::build(&r.context().device, r.layouts(), c)
        .err()
        .expect("broken WGSL is refused")
        .to_string();
    assert!(err.contains("near node 1 Ma boîte"), "{err}");
}

#[test]
#[ignore = "needs GPU"]
fn a_failed_build_is_not_cached_and_the_cached_pipeline_stays() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let good = Graph::bundled("mandelbox").unwrap();
    let adopted = r.pipeline_for(&good).unwrap();
    let before = r.cached_pipelines();
    let mut g = Graph::new_minimal();
    g.nodes[1].kind = NodeKind::Box {
        half: Vec3::splat(1.0),
    };
    let mut c = syfrack_render::graph_wgsl::compile(&g).unwrap();
    let broken_fp = c.fingerprint;
    c.wgsl = c.wgsl.replace("box_distance(", "box_distanc(");
    assert!(r.build_and_cache(c).is_err(), "broken WGSL is refused");
    assert_eq!(r.cached_pipelines(), before, "the failure is not cached");
    // The adopted topology is still served from the cache (same Arc).
    let again = r.pipeline_for(&good).unwrap();
    assert!(Arc::ptr_eq(&adopted, &again));
    assert_ne!(broken_fp, adopted.fingerprint);
}

#[test]
#[ignore = "needs GPU"]
fn angular_and_logarithmic_orbits_match_cpu() {
    use syfrack_core::TrapMode;
    for mode in [TrapMode::Minimum, TrapMode::Maximum, TrapMode::Last] {
        let mut g = Graph::bundled("mandelbox").unwrap();
        let loop_id = g
            .nodes
            .iter()
            .find(|n| matches!(n.kind, NodeKind::Iterate { .. }))
            .unwrap()
            .id;
        let first = g.next_id();
        for (i, measure) in [
            Measure::AngleXY,
            Measure::AngleXZ,
            Measure::AngleYZ,
            Measure::LogNorm,
        ]
        .into_iter()
        .enumerate()
        {
            g.nodes.push(node(
                first + i as u32,
                NodeKind::Trap {
                    slot: i as u32,
                    measure,
                    scale: if i == 3 { 4.0 } else { 1.0 },
                    mode,
                },
            ));
        }
        if let NodeKind::Iterate { body, .. } = &mut g.node_mut(loop_id).unwrap().kind {
            body.retain(|id| *id != 3);
            body.extend(first..first + 4);
        }
        g.validate().unwrap();
        check(&format!("angular-log-{mode:?}"), g, 8.0);
    }
}
