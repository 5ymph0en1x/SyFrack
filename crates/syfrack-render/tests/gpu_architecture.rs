use glam::Vec3;
use std::sync::Arc;
use syfrack_core::{
    CameraMode, Finish, FinishDetail, FinishPattern, Fractal, Graph, Link, Node, NodeKind, Scene,
};

#[test]
#[ignore = "needs GPU"]
fn seeded_eroded_rocks_match_cpu_across_negative_coordinates() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut s = Scene::example();
    s.fractal = Fractal::Graph(Graph::new_minimal());
    let pts: Vec<_> = (0..1700)
        .map(|i| {
            Vec3::new(
                (i % 17) as f32 * 0.63 - 5.0,
                ((i / 17) % 10) as f32 * 0.31 - 1.4,
                (i / 170) as f32 * 0.57 - 2.5,
            )
        })
        .collect();
    for seed in [17, 371, 65535] {
        s.fractal.graph_mut().nodes[1].kind = NodeKind::Rock {
            half: Vec3::new(3.1, 1.2, 2.7),
            erosion: 0.5,
            seed,
        };
        s.validate().unwrap();
        for (p, a) in pts.iter().zip(r.probe_distances(&s, &pts).unwrap()) {
            let b = s.fractal.evaluate(*p);
            assert!((a.distance - b.distance).abs() < 3e-5, "{p}: {a:?} {b:?}");
        }
        let source = syfrack_core::graph::language::format(s.fractal.graph());
        let back =
            syfrack_core::graph::language::compile(&source, Some(s.fractal.graph())).unwrap();
        assert_eq!(s.fractal.graph(), &back);
    }
}

#[test]
#[ignore = "needs GPU"]
fn fog_keeps_underlying_surface_guides_for_export_filtering() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let clear = scene();
    let reference = r
        .render(&clear, &RenderConfig::new(32, 32, 4), &mut |_| {})
        .unwrap();
    let mut fog = clear.clone();
    fog.atmosphere.fog_density = 0.5;
    fog.atmosphere.fog_falloff = 0.0;
    let mist = r
        .render(&fog, &RenderConfig::new(32, 32, 4), &mut |_| {})
        .unwrap();
    let mut count = 0;
    for i in 0..reference.depth.len() {
        if reference.depth[i] > 0.0 {
            assert_eq!(reference.depth[i], mist.depth[i]);
            assert_eq!(reference.normals[i], mist.normals[i]);
            count += 1;
        }
    }
    assert!(count > 10);
}

#[test]
#[ignore = "needs GPU"]
fn arbitrary_copies_match_cpu_in_overlaps_and_subtraction() {
    use syfrack_core::Placement;
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut s = scene();
    let g = s.fractal.graph_mut();
    g.nodes.push(Node {
        id: 6,
        name: "Independent copies".into(),
        pos: [0.; 2],
        enabled: true,
        kind: NodeKind::Instances {
            placements: vec![
                Placement {
                    position: Vec3::new(-0.8, 0.3, 0.2),
                    angles: Vec3::new(28., 45., -17.),
                    scale: Vec3::new(1.4, 0.7, 1.1),
                },
                Placement {
                    position: Vec3::new(0.6, -0.2, -0.4),
                    angles: Vec3::new(-42., 13., 60.),
                    scale: Vec3::new(0.8, 1.2, 1.5),
                },
                Placement {
                    position: Vec3::new(3.0, 0.0, -0.3),
                    angles: Vec3::new(0.0, 72.0, 0.0),
                    scale: Vec3::new(1.0, 1.0, 0.08),
                },
            ],
            half: Vec3::splat(2.5),
            margin: 0.1,
        },
    });
    if let NodeKind::Instances { placements, .. } = &mut g.nodes.last_mut().unwrap().kind {
        placements.extend((0..18).map(|i| Placement {
            position: Vec3::new(30. + (i % 6) as f32 * 3., 0., (i / 6) as f32 * 4.),
            angles: Vec3::new(12., (i * 19 % 360) as f32 - 180., -9.),
            scale: Vec3::new(0.8, 1.3, 0.7),
        }));
    }
    g.links.retain(|l| l.to != (2, 0));
    g.links.extend([
        Link {
            from: (5, 0),
            to: (6, 0),
        },
        Link {
            from: (6, 0),
            to: (2, 0),
        },
    ]);
    g.nodes.extend([
        Node {
            id: 9,
            name: "Shared template".into(),
            pos: [0.; 2],
            enabled: true,
            kind: NodeKind::Instances {
                placements: vec![Placement {
                    position: Vec3::new(-4., 0., 0.),
                    ..Placement::default()
                }],
                half: Vec3::splat(2.5),
                margin: 0.1,
            },
        },
        Node {
            id: 10,
            name: "Combined copies".into(),
            pos: [0.; 2],
            enabled: true,
            kind: NodeKind::Union,
        },
    ]);
    g.links.retain(|l| l.to != (2, 0));
    g.links.extend([
        Link {
            from: (5, 0),
            to: (9, 0),
        },
        Link {
            from: (6, 0),
            to: (10, 0),
        },
        Link {
            from: (9, 0),
            to: (10, 1),
        },
        Link {
            from: (10, 0),
            to: (2, 0),
        },
    ]);
    let mut points: Vec<_> = (0..1000)
        .map(|i| {
            Vec3::new(
                (i % 10) as f32 * 0.6 - 2.7,
                ((i / 10) % 10) as f32 * 0.6 - 2.7,
                (i / 100) as f32 * 0.6 - 2.7,
            )
        })
        .collect();
    points.extend((0..400).map(|i| {
        Vec3::new(
            29. + (i % 20) as f32,
            (i % 7) as f32 * 0.5 - 1.5,
            (i / 20) as f32 * 0.5 - 1.,
        )
    }));
    for negative in [false, true] {
        if negative {
            let g = s.fractal.graph_mut();
            g.nodes.extend([
                Node {
                    id: 7,
                    name: "Base".into(),
                    pos: [0.; 2],
                    enabled: true,
                    kind: NodeKind::Sphere { radius: 3. },
                },
                Node {
                    id: 8,
                    name: "Cut".into(),
                    pos: [0.; 2],
                    enabled: true,
                    kind: NodeKind::Subtraction,
                },
            ]);
            g.links.retain(|l| l.to != (2, 0));
            g.links.extend([
                Link {
                    from: (0, 0),
                    to: (7, 0),
                },
                Link {
                    from: (7, 0),
                    to: (8, 0),
                },
                Link {
                    from: (6, 0),
                    to: (8, 1),
                },
                Link {
                    from: (8, 0),
                    to: (2, 0),
                },
            ]);
        }
        s.validate().unwrap();
        for (p, a) in points.iter().zip(r.probe_distances(&s, &points).unwrap()) {
            let b = s.fractal.evaluate(*p);
            assert!((a.distance - b.distance).abs() < 4e-5, "{p}: {a:?} {b:?}");
            assert_eq!(a.material, b.material);
        }
        if !negative {
            let mut cfg = RenderConfig::new(16, 16, 1);
            cfg.max_bounces = 0;
            cfg.max_steps = 256;
            let image = r.render(&s, &cfg, &mut |_| {}).unwrap();
            assert!(image.depth.iter().any(|d| *d > 0.));
            assert!(image.color.data.iter().flatten().all(|v| v.is_finite()));
        }
    }
}

#[test]
#[ignore = "needs GPU"]
fn distant_architecture_is_visible_beyond_the_legacy_two_hundred_unit_range() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut s = scene();
    let mut g = Graph::new_minimal();
    g.nodes[1].kind = NodeKind::Sphere { radius: 30. };
    g.nodes.push(Node {
        id: 3,
        name: "Distant island".into(),
        pos: [0.; 2],
        enabled: true,
        kind: NodeKind::Instances {
            placements: vec![syfrack_core::Placement {
                position: Vec3::new(0., 0., -300.),
                ..Default::default()
            }],
            half: Vec3::splat(40.),
            margin: 0.2,
        },
    });
    g.links.retain(|l| l.to != (2, 0));
    g.links.extend([
        Link {
            from: (1, 0),
            to: (3, 0),
        },
        Link {
            from: (3, 0),
            to: (2, 0),
        },
    ]);
    g.bounding_radius_hint = 400.;
    s.fractal = Fractal::Graph(g);
    s.camera.mode = CameraMode::Free {
        position: Vec3::ZERO,
        target: Vec3::new(0., 0., -300.),
        roll: 0.,
    };
    s.camera.focal_mm = 85.;
    let mut cfg = RenderConfig::new(16, 16, 1);
    cfg.max_bounces = 0;
    cfg.max_steps = 256;
    let out = r.render(&s, &cfg, &mut |_| {}).unwrap();
    assert!(
        out.depth[8 * 16 + 8] > 260. && out.depth[8 * 16 + 8] < 300.,
        "{}",
        out.depth[8 * 16 + 8]
    );
}

#[test]
#[ignore = "needs GPU"]
fn the_last_of_ninety_six_lights_illuminates_the_scene() {
    use syfrack_core::{Color, Light};
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut s = scene();
    s.material.finishes[0].emission = [0.; 3];
    let dark = Light::Environment {
        zenith: Color::Linear([0.; 3]),
        horizon: Color::Linear([0.; 3]),
        ground: Color::Linear([0.; 3]),
        intensity: 0.,
        rotation: 0.,
    };
    s.lights = vec![dark; syfrack_core::MAX_LIGHTS];
    let mut cfg = RenderConfig::new(24, 24, 8);
    cfg.max_bounces = 0;
    let a = r.render(&s, &cfg, &mut |_| {}).unwrap();
    *s.lights.last_mut().unwrap() = Light::Area {
        position: Vec3::new(2.5, 3., 1.5),
        normal: Vec3::new(-0.5, -1.0, -0.3),
        size: [2., 2.],
        color: Color::Kelvin(2800.),
        intensity: 20.,
    };
    s.validate().unwrap();
    let b = r.render(&s, &cfg, &mut |_| {}).unwrap();
    let energy =
        |image: &syfrack_render::RenderOutput| image.color.data.iter().flatten().sum::<f32>();
    assert!(energy(&b) > energy(&a) + 1.0);
    assert!(b.color.data.iter().flatten().all(|x| x.is_finite()));
}

#[test]
#[ignore = "needs GPU"]
fn finite_repetition_matches_cpu_at_all_edges_and_outside_the_array() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut s = scene();
    s.fractal
        .graph_mut()
        .nodes
        .iter_mut()
        .find(|n| n.id == 3)
        .unwrap()
        .kind = NodeKind::RepeatFinite {
        period: Vec3::splat(6.0),
        count: [2, 1, 3],
    };
    s.fractal
        .graph_mut()
        .links
        .iter_mut()
        .find(|l| l.to == (4, 0))
        .unwrap()
        .from = (3, 0);
    let points: Vec<_> = (0..1000)
        .map(|i| {
            Vec3::new(
                (i % 10) as f32 * 1.8 - 8.0,
                ((i / 10) % 10) as f32 * 0.3 - 1.0,
                (i / 100) as f32 * 2.4 - 10.0,
            )
        })
        .collect();
    for (p, a) in points.iter().zip(r.probe_distances(&s, &points).unwrap()) {
        let b = s.fractal.evaluate(*p);
        assert!((a.distance - b.distance).abs() < 2e-5, "{p}: {a:?} {b:?}");
        assert_eq!(a.material, b.material);
    }
}

#[test]
#[ignore = "needs GPU"]
fn personal_textures_change_shading_preserve_geometry_and_are_deterministic() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut s = scene();
    s.material.finishes[0].emission = [0.0; 3];
    let mut cfg = RenderConfig::new(32, 32, 4);
    cfg.max_bounces = 0;
    let plain = r.render(&s, &cfg, &mut |_| {}).unwrap();
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
        FinishPattern::Concrete,
        FinishPattern::WeatheredConcrete,
        FinishPattern::WetConcrete,
    ] {
        s.material.finishes[0].detail = Some(FinishDetail {
            pattern,
            strength: 1.0,
            bump: -0.02,
            scale: 3.0,
            tint: [0.02, 0.08, 0.5],
            ..FinishDetail::default()
        });
        let detailed = r.render(&s, &cfg, &mut |_| {}).unwrap();
        assert_eq!(plain.depth, detailed.depth);
        assert_eq!(plain.normals, detailed.normals);
        assert_ne!(plain.color.data, detailed.color.data, "{pattern:?}");
        assert!(detailed.color.data.iter().flatten().all(|v| v.is_finite()));
    }
    let a = r.render(&s, &cfg, &mut |_| {}).unwrap();
    let b = r.render(&s, &cfg, &mut |_| {}).unwrap();
    assert_eq!(a.color.data, b.color.data);
}
use syfrack_render::{GpuContext, RenderConfig, Renderer};

fn scene() -> Scene {
    let mut s = Scene::example();
    let mut g = Graph::new_minimal();
    g.nodes[1].kind = NodeKind::Torus {
        radius: 1.0,
        tube: 0.2,
    };
    for (id, kind) in [
        (
            3,
            NodeKind::ScaleAxes {
                factors: Vec3::new(1.0, 0.7, 1.0),
            },
        ),
        (
            4,
            NodeKind::Bounded {
                half: Vec3::splat(2.5),
                margin: 0.1,
            },
        ),
        (5, NodeKind::MaterialTag { index: 1 }),
    ] {
        g.nodes.push(Node {
            id,
            name: kind.label().into(),
            kind,
            pos: [0.0; 2],
            enabled: true,
        });
    }
    g.links = [
        (0, 3, 0),
        (3, 1, 0),
        (0, 4, 0),
        (1, 4, 1),
        (4, 5, 0),
        (5, 2, 0),
    ]
    .into_iter()
    .map(|(a, b, p)| Link {
        from: (a, 0),
        to: (b, p),
    })
    .collect();
    s.fractal = Fractal::Graph(g);
    s.material.finishes = vec![Finish {
        emission: [0.5, 0.2, 0.1],
        ..Finish::default()
    }];
    s.camera.mode = CameraMode::Free {
        position: Vec3::new(0.0, 4.0, 0.0),
        target: Vec3::ZERO,
        roll: 0.0,
    };
    s.camera.focal_mm = 45.0;
    s.atmosphere.cavity_darkness = 0.0;
    s
}

#[test]
#[ignore = "needs GPU"]
fn bounded_curves_match_cpu_and_do_not_render_the_bound_as_geometry() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let s = scene();
    s.validate().unwrap();
    let pts: Vec<_> = (0..729)
        .map(|i| {
            Vec3::new(
                (i % 9) as f32 - 4.0,
                ((i / 9) % 9) as f32 - 4.0,
                (i / 81) as f32 - 4.0,
            ) * 0.4
        })
        .collect();
    let gpu = r.probe_distances(&s, &pts).unwrap();
    for (p, a) in pts.iter().zip(gpu) {
        let b = s.fractal.evaluate(*p);
        assert!((a.distance - b.distance).abs() < 1e-5, "{p}: {a:?} {b:?}");
        assert_eq!(a.material, b.material);
    }
    let mut config = RenderConfig::new(32, 32, 4);
    config.max_bounces = 0;
    config.max_steps = 384;
    config.pixel_tolerance = 1.0; // deliberately coarse: box must never become a hit
    let out = r.render(&s, &config, &mut |_| {}).unwrap();
    let center = out.depth[16 * 32 + 16];
    assert!(
        center <= 0.0 || center >= 100.0,
        "the torus hole must show the sky: {center}"
    );
    let hits: Vec<_> = out
        .depth
        .iter()
        .copied()
        .filter(|t| *t > 0.0 && *t < 100.0)
        .collect();
    assert!(hits.len() > 80);
    assert!(
        hits.iter().all(|t| *t > 3.4),
        "phantom bounding-box front face"
    );
}

#[test]
#[ignore = "needs GPU"]
fn a_bound_in_a_subtracted_operand_preserves_exact_geometry() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut s = scene();
    let g = s.fractal.graph_mut();
    g.nodes.push(Node {
        id: 6,
        kind: NodeKind::Cylinder {
            radius: 1.5,
            half_height: 0.4,
        },
        name: "Solid".into(),
        pos: [0.0; 2],
        enabled: true,
    });
    g.nodes.push(Node {
        id: 7,
        kind: NodeKind::Subtraction,
        name: "Cut".into(),
        pos: [0.0; 2],
        enabled: true,
    });
    g.links.retain(|l| l.to.0 != 2);
    g.links.extend(
        [(0, 6, 0), (6, 7, 0), (5, 7, 1), (7, 2, 0)]
            .into_iter()
            .map(|(a, b, p)| Link {
                from: (a, 0),
                to: (b, p),
            }),
    );
    let pts: Vec<_> = (0..100)
        .map(|i| Vec3::new(i as f32 * 0.04, 0.2, 0.0))
        .collect();
    for (p, a) in pts.iter().zip(r.probe_distances(&s, &pts).unwrap()) {
        assert!((a.distance - s.fractal.distance(*p)).abs() < 1e-5);
    }
}

#[test]
#[ignore = "needs GPU"]
fn local_reinjection_matches_cpu_after_nonuniform_scaling_and_translation() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut s = scene();
    let g = s.fractal.graph_mut();
    g.nodes.iter_mut().find(|n| n.id == 1).unwrap().kind = NodeKind::NormFence { inset: 0.0 };
    for (id, kind) in [
        (6, NodeKind::BoxFold { limit: 1.0 }),
        (
            7,
            NodeKind::SphereFold {
                min_radius: 0.4,
                fixed_radius: 1.0,
            },
        ),
        (8, NodeKind::Scale { factor: 2.6 }),
        (
            9,
            NodeKind::AddSeed {
                multiplier: Vec3::ONE,
            },
        ),
        (
            10,
            NodeKind::Iterate {
                iterations: 12,
                bailout: 1000.0,
                body: vec![6, 7, 8, 9],
            },
        ),
        (
            11,
            NodeKind::Translate {
                offset: Vec3::new(-2.0, 0.0, -1.0),
            },
        ),
    ] {
        g.nodes.push(Node {
            id,
            kind,
            name: format!("Seed {id}"),
            pos: [0.0; 2],
            enabled: true,
        });
    }
    g.links.retain(|l| l.to != (1, 0) && l.to != (3, 0));
    g.links.extend(
        [(0, 11, 0), (11, 3, 0), (3, 10, 0), (10, 1, 0)]
            .into_iter()
            .map(|(a, b, p)| Link {
                from: (a, 0),
                to: (b, p),
            }),
    );
    let pts: Vec<_> = (0..729)
        .map(|i| {
            Vec3::new(
                (i % 9) as f32 - 4.0,
                ((i / 9) % 9) as f32 - 4.0,
                (i / 81) as f32 - 4.0,
            ) * 0.5
        })
        .collect();
    for (p, a) in pts.iter().zip(r.probe_distances(&s, &pts).unwrap()) {
        let b = s.fractal.evaluate(*p);
        assert!(
            (a.distance - b.distance).abs() < 1e-3 + 0.01 * b.distance.abs(),
            "{p}: {a:?} {b:?}"
        );
        assert_eq!(a.material, b.material);
    }
}
