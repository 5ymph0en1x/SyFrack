//! Compare the fixed interpreter with the production GPU distance evaluator.
//! No iterations, traps, branches or instance transforms may be simplified.
use glam::Vec3;
use std::sync::Arc;
use syfrack_core::{Fractal, Graph, Link, Node, NodeKind, Scene};
use syfrack_render::{control::ControlSession, GpuContext, Renderer};

fn node(id: u32, kind: NodeKind) -> Node {
    Node {
        id,
        name: format!("node {id}"),
        kind,
        pos: [0.0; 2],
        enabled: true,
    }
}
fn link(a: u32, b: u32, port: u32) -> Link {
    Link {
        from: (a, 0),
        to: (b, port),
    }
}

fn compare(r: &Renderer, name: &str, g: Graph, points: &[Vec3]) {
    compare_traps(r, name, g, points, 2e-5, 2e-6);
}

fn compare_traps(
    r: &Renderer,
    name: &str,
    g: Graph,
    points: &[Vec3],
    trap_max: f32,
    trap_rms: f64,
) {
    let mut scene = Scene::example();
    scene.fractal = Fractal::Graph(g);
    scene.material.finishes = vec![syfrack_core::Finish::default(); 8];
    let control = r.control_distances(&scene, points).unwrap();
    let native = r.probe_distances(&scene, points).unwrap();
    assert_eq!(control.len(), points.len());
    assert_eq!(native.len(), points.len());
    let mut worst = 0.0f32;
    let mut max_absolute = 0.0f32;
    let mut trap_worst = 0.0f32;
    let mut trap_squared = 0.0f64;
    for ((a, b), p) in control.iter().zip(&native).zip(points) {
        if a.distance.is_finite() && b.distance.is_finite() {
            let distance = (a.distance - b.distance).abs();
            let tolerance = 2e-5 + 2e-4 * b.distance.abs();
            worst = worst.max(distance / tolerance);
            max_absolute = max_absolute.max(distance);
            assert!(
                distance <= tolerance,
                "{name} at {p}: control={a:?}, native={b:?}, normalized error={}",
                distance / tolerance
            );
        } else {
            assert!(
                !a.distance.is_nan() && !b.distance.is_nan(),
                "unexpected NaN field at {p}: {name}"
            );
            assert_eq!(
                a.distance.to_bits(),
                b.distance.to_bits(),
                "singularity differs: {name} at {p}"
            );
        }
        assert_eq!(a.material, b.material, "{name} at {p}");
        assert_eq!(a.iterations, b.iterations, "{name} at {p}");
        for k in 0..4 {
            let delta = (a.traps[k] - b.traps[k]).abs();
            trap_worst = trap_worst.max(delta);
            trap_squared += f64::from(delta).powi(2);
            assert!(a.traps[k].is_finite() && (0.0..=1.0).contains(&a.traps[k]));
            assert!(b.traps[k].is_finite() && (0.0..=1.0).contains(&b.traps[k]));
        }
    }
    let rms = (trap_squared / (points.len() * 4) as f64).sqrt();
    eprintln!(
        "{name}: {} points, max absolute distance error {max_absolute:.9}, normalized {worst:.6}, max trap difference {trap_worst:.8}, trap RMS {rms:.9}",
        points.len()
    );
    assert!(
        trap_worst <= trap_max && rms <= trap_rms,
        "{name}: trap max {trap_worst} / {trap_max}, RMS {rms} / {trap_rms}"
    );
}

#[test]
#[ignore = "needs GPU"]
fn all_bundled_graphs_match_specialized_gpu_distances_and_payloads() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let count = std::env::var("SYFRACK_CONTROL_PARITY_POINTS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(4096);
    let points = syfrack_render::parity::corpus(count, 2.5, 1847).unwrap();
    for &(name, _) in syfrack_core::graph::bundled::BUNDLED {
        // Long chaotic orbits amplify the native driver's arithmetic contraction
        // in colour traps. Geometry/iterations/materials retain the strict checks
        // above; isolated trap operators use the much tighter compare() bounds.
        compare_traps(&r, name, Graph::bundled(name).unwrap(), &points, 1e-2, 1e-4);
    }
}

#[test]
#[ignore = "needs GPU"]
fn bounded_nested_instances_preserve_scaled_rotated_fences_and_material_ties() {
    use syfrack_core::{graph::Placement, Measure, TrapMode};
    use NodeKind::*;
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut g = Graph::new_minimal();
    let copies = (0..384)
        .map(|i| Placement {
            position: Vec3::new((i % 24) as f32 * 1.5, (i / 24) as f32 * 1.5, 0.0),
            angles: Vec3::new(12.0, 31.0, ((i * 3) % 360) as f32 - 180.0),
            scale: Vec3::new(0.7, 1.1, 0.9),
        })
        .collect();
    g.nodes = vec![
        node(0, Input),
        node(
            1,
            Trap {
                slot: 1,
                measure: Measure::Norm,
                scale: 2.0,
                mode: TrapMode::Maximum,
            },
        ),
        node(
            2,
            Box {
                half: Vec3::splat(0.4),
            },
        ),
        node(3, MaterialTag { index: 3 }),
        node(
            4,
            Instances {
                placements: copies,
                half: Vec3::ONE,
                margin: 0.05,
            },
        ),
        node(
            5,
            Instances {
                placements: vec![
                    Placement::default(),
                    Placement {
                        position: Vec3::new(0.0, 0.0, 4.0),
                        ..Placement::default()
                    },
                ],
                half: Vec3::splat(50.0),
                margin: 0.05,
            },
        ),
        node(
            6,
            Bounded {
                half: Vec3::splat(100.0),
                margin: 0.1,
            },
        ),
        node(7, Output),
    ];
    g.links = vec![
        link(0, 1, 0),
        link(1, 2, 0),
        link(2, 3, 0),
        link(3, 4, 0),
        link(4, 5, 0),
        link(0, 6, 0),
        link(5, 6, 1),
        link(6, 7, 0),
    ];
    let mut points = syfrack_render::parity::corpus(256, 8.0, 9201).unwrap();
    points.extend([
        Vec3::new(34.0, 22.0, 0.1),
        Vec3::new(8.0, 10.0, 4.0),
        Vec3::new(99.99, 0.0, 0.0),
    ]);
    compare(&r, "nested rotated copies", g.clone(), &points);
    g.nodes.iter_mut().find(|n| n.id == 4).unwrap().enabled = false;
    compare(&r, "disabled instance", g, &points);
}

#[test]
#[ignore = "needs GPU"]
fn control_edits_never_compile_native_topologies_and_pending_work_is_replaceable() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = Scene::example();
    scene.fractal = Fractal::Graph(Graph::new_minimal());
    let mut control = ControlSession::new(r.clone(), &scene, (64, 36)).unwrap();
    control.tick().unwrap(); // deliberately replace a pending image
    for name in ["mandelbox", "menger", "mandelbulb", "sphere"] {
        scene.fractal = Fractal::Graph(Graph::bundled(name).unwrap());
        control.set_scene(&scene).unwrap();
        control.tick().unwrap();
        assert_eq!(
            r.cached_pipelines(),
            0,
            "a control edit must not compile the graph"
        );
    }
    scene.fractal = Fractal::Graph(Graph::new_minimal());
    control.set_scene(&scene).unwrap();
    control.wait_complete().unwrap();
    assert!(control.complete());
    let hit = control.pick(0.5, 0.5).unwrap().expect("sphere visible");
    assert!(hit.0 > 0.0 && hit.1[3] >= 1.0);
    let norm = Vec3::new(hit.1[0], hit.1[1], hit.1[2]).length();
    assert!((norm - 1.0).abs() < 1e-4);
    assert_eq!(r.cached_pipelines(), 0);
}

#[test]
#[ignore = "needs GPU"]
fn grazing_rays_continue_past_512_steps_without_missing_geometry() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = Scene::example();
    let mut g = Graph::new_minimal();
    g.nodes.iter_mut().find(|n| n.id == 1).unwrap().kind = NodeKind::Plane {
        normal: Vec3::Y,
        offset: 0.0,
    };
    g.safety = 0.1;
    scene.fractal = Fractal::Graph(g);
    scene.camera.mode = syfrack_core::CameraMode::Free {
        position: Vec3::new(0.0, 0.1, 0.0),
        target: Vec3::new(100.0, 0.0, 0.0),
        roll: 0.0,
    };
    scene.camera.focal_mm = 1200.0;
    let mut control = ControlSession::new(r, &scene, (257, 1)).unwrap();
    control.wait_complete().unwrap();
    let (depth, aux) = control.pick(0.5, 0.5).unwrap().expect("grazing plane hit");
    assert!(depth > 40.0 && depth < 100.0, "{depth}");
    assert!(aux[1] > 0.999, "normal {aux:?}");
    assert!(aux[3] > 512.0, "ray was truncated: {aux:?}");
    eprintln!(
        "grazing ray: {} steps, {} quanta, depth {depth}",
        aux[3],
        control.dispatches()
    );
}

#[test]
#[ignore = "needs GPU"]
fn undefined_field_reports_its_node_and_a_valid_edit_recovers() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = Scene::example();
    let mut g = Graph::new_minimal();
    g.nodes.iter_mut().find(|n| n.id == 1).unwrap().kind = NodeKind::Power { power: 8.0 };
    g.nodes.push(node(2, NodeKind::NormFence { inset: 0.5 }));
    g.nodes
        .iter_mut()
        .find(|n| matches!(n.kind, NodeKind::Output))
        .unwrap()
        .id = 3;
    g.links = vec![link(0, 1, 0), link(1, 2, 0), link(2, 3, 0)];
    scene.fractal = Fractal::Graph(g);
    scene.camera.mode = syfrack_core::CameraMode::Free {
        position: Vec3::ZERO,
        target: Vec3::NEG_Z,
        roll: 0.0,
    };
    let mut control = ControlSession::new(r, &scene, (1, 1)).unwrap();
    let error = control.wait_complete().unwrap_err().to_string();
    assert!(error.contains("node 2"), "{error}");
    assert_eq!(control.tick().unwrap_err().to_string(), error);
    scene.fractal = Fractal::Graph(Graph::new_minimal());
    control.set_scene(&scene).unwrap();
    control.wait_complete().unwrap();
    assert!(control.pick(0.5, 0.5).unwrap().is_some());
}

#[test]
#[ignore = "needs GPU"]
fn algebraic_cubic_preserves_the_complex_plane_and_reuses_generic_power_pipelines() {
    use syfrack_core::{Measure, TrapMode};
    use NodeKind::*;
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut g = Graph::new_minimal();
    g.nodes = vec![
        node(0, Input),
        node(1, Power { power: 3.0 }),
        node(
            2,
            Trap {
                slot: 0,
                measure: Measure::PlaneY,
                scale: 1.0,
                mode: TrapMode::Last,
            },
        ),
        node(3, NormFence { inset: 0.5 }),
        node(4, Output),
    ];
    g.links = vec![link(0, 1, 0), link(1, 2, 0), link(2, 3, 0), link(3, 4, 0)];
    let mut points: Vec<_> = syfrack_render::parity::corpus(1024, 2.0, 18031)
        .unwrap()
        .iter()
        .map(|p| Vec3::new(p.x, 0.0, p.z))
        .collect();
    points.extend([
        Vec3::ZERO,
        Vec3::Z,
        Vec3::NEG_Z,
        Vec3::X,
        Vec3::NEG_X,
        Vec3::new(1e-20, 0.0, -1e-20),
    ]);
    points.extend(syfrack_render::parity::corpus(512, 2.0, 9018).unwrap());
    let mut s = Scene::example();
    s.fractal = Fractal::Graph(g.clone());
    let native = r.probe_distances(&s, &points).unwrap();
    let fixed = r.control_distances(&s, &points).unwrap();
    for ((a, b), p) in native.iter().zip(&fixed).zip(&points) {
        let expected = g.evaluate(*p);
        assert!(
            (a.distance - expected.distance).abs() <= 2e-5 + 2e-4 * expected.distance.abs(),
            "{p}: {a:?} / {expected:?}"
        );
        if p.y == 0.0 && p.length() >= 1e-12 {
            assert_eq!(a.traps[0], 0.0, "native plane drift at {p}");
            assert_eq!(b.traps[0], 0.0, "control plane drift at {p}");
        }
    }
    let cached = r.cached_pipelines();
    g.nodes.iter_mut().find(|n| n.id == 1).unwrap().kind = Power { power: 3.5 };
    compare(&r, "generic power after cubic", g, &points);
    assert_eq!(
        r.cached_pipelines(),
        cached,
        "numeric powers must retain the runtime branch"
    );
}

#[test]
#[ignore = "needs GPU"]
fn compiled_pipeline_adoption_checks_topology_and_keeps_the_supplied_pipeline() {
    use syfrack_render::{RenderConfig, RenderSession};
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = Scene::example();
    scene.fractal = Fractal::Graph(Graph::new_minimal());
    let pipeline = r.pipeline_for_scene(&scene).unwrap();
    let cfg = RenderConfig::new(8, 8, 1);
    let session = RenderSession::from_pipeline(r.clone(), &scene, &cfg, pipeline.clone()).unwrap();
    assert_eq!(session.scene().fractal, scene.fractal);
    assert_eq!(r.cached_pipelines(), 1);
    scene.fractal = Fractal::Graph(Graph::bundled("menger").unwrap());
    let error = RenderSession::from_pipeline(r.clone(), &scene, &cfg, pipeline)
        .err()
        .expect("stale pipeline refused");
    assert!(error.to_string().contains("does not match scene topology"));
    assert_eq!(
        r.cached_pipelines(),
        1,
        "a rejected adoption must not compile"
    );
}

#[test]
#[ignore = "needs GPU"]
fn every_point_operation_fence_and_trap_mode_keeps_the_production_field() {
    use syfrack_core::{Measure, TrapMode};
    use NodeKind::*;
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let points = syfrack_render::parity::corpus(128, 1.7, 432).unwrap();
    let mut operations = vec![
        Translate {
            offset: Vec3::new(0.2, -0.3, 0.5),
        },
        Rotate {
            angles: Vec3::new(37.0, -62.0, 123.0),
        },
        Scale { factor: -1.7 },
        ScaleAxes {
            factors: Vec3::new(0.4, 1.2, 2.0),
        },
        Mirror {
            normal: Vec3::new(0.3, 0.5, 0.7),
            offset: 0.2,
        },
        BoxFold { limit: 0.7 },
        SphereFold {
            min_radius: 0.3,
            fixed_radius: 1.1,
        },
        BoxFoldAxes {
            limit: Vec3::new(0.3, 0.7, 1.1),
            x: true,
            y: false,
            z: true,
        },
        SphereInversion {
            radius: 1.2,
            min_radius: 0.1,
        },
        AddConstant {
            multiplier: Vec3::new(0.4, -0.2, 0.7),
            julia: Vec3::new(0.1, 0.2, -0.1),
        },
        Abs {
            x: false,
            y: true,
            z: true,
        },
        MengerFold {
            scale: 2.7,
            offset: Vec3::new(1.0, 0.8, 0.9),
        },
        Kaleido {
            planes: 7,
            phase: 22.0,
        },
        Power { power: 3.0 },
        Power { power: 3.5 },
        Power { power: 6.0 },
        Power { power: 8.0 },
        AddOrigin,
        AddSeed {
            multiplier: Vec3::new(0.2, -0.4, 0.7),
        },
        Twist { rate: 0.7 },
        Bend { rate: -0.3 },
        Tile {
            period: Vec3::new(1.0, 1.5, 0.8),
        },
        RepeatFinite {
            period: Vec3::new(1.0, 1.5, 0.8),
            count: [3, 4, 5],
        },
        Iterate {
            iterations: 64,
            bailout: 4.0,
            body: vec![13],
        },
    ];
    for mode in [TrapMode::Minimum, TrapMode::Maximum, TrapMode::Last] {
        for measure in [
            Measure::Norm,
            Measure::AxisX,
            Measure::AxisY,
            Measure::AxisZ,
            Measure::PlaneY,
            Measure::AngleXY,
            Measure::AngleXZ,
            Measure::AngleYZ,
            Measure::LogNorm,
        ] {
            operations.push(Trap {
                slot: 2,
                measure,
                scale: 2.0,
                mode,
            });
        }
    }
    for kind in operations {
        let mut g = Graph::new_minimal();
        g.nodes = vec![
            node(0, Input),
            node(1, kind.clone()),
            node(2, NormFence { inset: 0.5 }),
            node(3, Output),
        ];
        g.links = vec![link(0, 1, 0), link(1, 2, 0), link(2, 3, 0)];
        if matches!(kind, Iterate { .. }) {
            g.nodes.extend([
                node(
                    13,
                    Iterate {
                        iterations: 4,
                        bailout: 4.0,
                        body: vec![10, 11, 12],
                    },
                ),
                node(10, BoxFold { limit: 0.7 }),
                node(
                    11,
                    AddSeed {
                        multiplier: Vec3::splat(0.3),
                    },
                ),
                node(
                    12,
                    Trap {
                        slot: 0,
                        measure: Measure::Norm,
                        scale: 2.0,
                        mode: TrapMode::Last,
                    },
                ),
            ]);
        }
        compare(&r, &format!("{kind:?}"), g, &points);
    }
    for kind in [
        Sphere { radius: 0.7 },
        Torus {
            radius: 0.8,
            tube: 0.2,
        },
        Cylinder {
            radius: 0.5,
            half_height: 0.7,
        },
        Box {
            half: Vec3::new(0.5, 0.7, 0.9),
        },
        RoundBox {
            half: Vec3::ONE,
            radius: 0.2,
        },
        Rock {
            half: Vec3::new(0.7, 0.8, 0.9),
            erosion: 0.2,
            seed: 73,
        },
        Plane {
            normal: Vec3::new(0.3, 0.5, 0.7),
            offset: 0.4,
        },
        NormFence { inset: 0.4 },
        BulbFence,
    ] {
        let mut g = Graph::new_minimal();
        g.nodes.iter_mut().find(|n| n.id == 1).unwrap().kind = kind.clone();
        compare(&r, &format!("{kind:?}"), g.clone(), &points);
        g.nodes.iter_mut().find(|n| n.id == 1).unwrap().enabled = false;
        compare(&r, "disabled fence", g, &points);
    }
}
