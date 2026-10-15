//! Writes the composed forms of the bundled library to `graphs/` (run once,
//! on purpose): `cargo test -p syfrack-core --test library -- --ignored`.
//! The classics (sphere, Mandelbox, Menger, Mandelbulb) were written from the
//! hand-written formulas before their removal; their files are the source now.

use glam::Vec3;
use std::path::PathBuf;
use syfrack_core::{Fractal, Graph, Link, Measure, Node, NodeKind};

fn node(id: u32, kind: NodeKind, col: f32, row: f32) -> Node {
    Node {
        id,
        // Historical graph names are fixture data, independent of the UI's default language.
        name: syfrack_i18n::with_language(syfrack_i18n::Language::French, || {
            kind.label().to_string()
        }),
        kind,
        pos: [col * 240.0, row * 96.0],
        enabled: true,
    }
}

fn link(a: u32, b: u32) -> Link {
    Link {
        from: (a, 0),
        to: (b, 0),
    }
}

fn trap(scale: f32) -> NodeKind {
    NodeKind::Trap {
        slot: 0,
        measure: Measure::Norm,
        scale,
        mode: Default::default(),
    }
}

/// Input, Repeat (box fold, trap, sphere fold, scale, power, add origin), norm fence, Output.
fn box_bulb() -> Graph {
    Graph {
        nodes: vec![
            node(0, NodeKind::Input, 0.0, 0.0),
            node(
                1,
                NodeKind::Iterate {
                    iterations: 10,
                    bailout: 1000.0,
                    body: vec![2, 3, 4, 5, 6, 7],
                },
                1.0,
                0.0,
            ),
            node(2, NodeKind::BoxFold { limit: 1.0 }, 1.0, 1.0),
            node(3, trap(2.0), 1.0, 2.0),
            node(
                4,
                NodeKind::SphereFold {
                    min_radius: 0.4,
                    fixed_radius: 1.0,
                },
                1.0,
                3.0,
            ),
            node(5, NodeKind::Scale { factor: 2.2 }, 1.0, 4.0),
            node(6, NodeKind::Power { power: 3.0 }, 1.0, 5.0),
            node(7, NodeKind::AddOrigin, 1.0, 6.0),
            node(8, NodeKind::NormFence { inset: 0.0 }, 2.0, 0.0),
            node(9, NodeKind::Output, 3.0, 0.0),
        ],
        links: vec![link(0, 1), link(1, 8), link(8, 9)],
        safety: 0.5,
        bounding_radius_hint: 8.0,
    }
}

/// Input, kaleidoscope, twist, Menger repeat, box, Output.
fn menger_kaleido() -> Graph {
    Graph {
        nodes: vec![
            node(0, NodeKind::Input, 0.0, 0.0),
            node(
                1,
                NodeKind::Kaleido {
                    planes: 5,
                    phase: 0.0,
                },
                1.0,
                0.0,
            ),
            node(2, NodeKind::Twist { rate: 0.25 }, 2.0, 0.0),
            node(
                3,
                NodeKind::Iterate {
                    iterations: 5,
                    bailout: 10000.0,
                    body: vec![4, 5],
                },
                3.0,
                0.0,
            ),
            node(
                4,
                NodeKind::MengerFold {
                    scale: 3.0,
                    offset: Vec3::ONE,
                },
                3.0,
                1.0,
            ),
            node(5, trap(4.0), 3.0, 2.0),
            node(6, NodeKind::Box { half: Vec3::ONE }, 4.0, 0.0),
            node(7, NodeKind::Output, 5.0, 0.0),
        ],
        links: vec![link(0, 1), link(1, 2), link(2, 3), link(3, 6), link(6, 7)],
        safety: 0.7,
        bounding_radius_hint: 4.0,
    }
}

/// A small hollow Mandelbox (A) minus a shrunken Menger sponge (B), smoothly
/// joined to an offset sphere (C).
fn evidement() -> Graph {
    Graph {
        nodes: vec![
            node(0, NodeKind::Input, 0.0, 0.0),
            // A: Mandelbox with a negative scale (a smaller hollow box).
            node(
                1,
                NodeKind::Iterate {
                    iterations: 8,
                    bailout: 1000.0,
                    body: vec![2, 3, 4, 5, 6],
                },
                1.0,
                0.0,
            ),
            node(2, NodeKind::BoxFold { limit: 1.0 }, 1.0, 1.0),
            node(3, trap(2.0), 1.0, 2.0),
            node(
                4,
                NodeKind::SphereFold {
                    min_radius: 0.5,
                    fixed_radius: 1.0,
                },
                1.0,
                3.0,
            ),
            node(5, NodeKind::Scale { factor: -1.5 }, 1.0, 4.0),
            node(6, NodeKind::AddOrigin, 1.0, 5.0),
            node(7, NodeKind::NormFence { inset: 0.0 }, 2.0, 0.0),
            // B: shifted, shrunken Menger sponge.
            node(
                8,
                NodeKind::Translate {
                    offset: Vec3::new(0.0, 0.5, 0.0),
                },
                1.0,
                7.0,
            ),
            node(9, NodeKind::Scale { factor: 0.4 }, 2.0, 7.0),
            node(
                10,
                NodeKind::Iterate {
                    iterations: 3,
                    bailout: 1.0e6,
                    body: vec![11],
                },
                3.0,
                7.0,
            ),
            node(
                11,
                NodeKind::MengerFold {
                    scale: 3.0,
                    offset: Vec3::ONE,
                },
                3.0,
                8.0,
            ),
            node(12, NodeKind::Box { half: Vec3::ONE }, 4.0, 7.0),
            // C: offset sphere.
            node(
                13,
                NodeKind::Translate {
                    offset: Vec3::new(-3.6, 0.0, 0.0),
                },
                1.0,
                10.0,
            ),
            node(14, NodeKind::Sphere { radius: 1.2 }, 2.0, 10.0),
            node(15, NodeKind::Subtraction, 5.0, 0.0),
            node(16, NodeKind::SmoothUnion { k: 0.3 }, 6.0, 0.0),
            node(17, NodeKind::Output, 7.0, 0.0),
        ],
        links: vec![
            link(0, 1),
            link(1, 7),
            link(0, 8),
            link(8, 9),
            link(9, 10),
            link(10, 12),
            link(0, 13),
            link(13, 14),
            Link {
                from: (7, 0),
                to: (15, 0),
            },
            Link {
                from: (12, 0),
                to: (15, 1),
            },
            Link {
                from: (15, 0),
                to: (16, 0),
            },
            Link {
                from: (14, 0),
                to: (16, 1),
            },
            link(16, 17),
        ],
        safety: 0.5,
        bounding_radius_hint: 5.0,
    }
}

/// The exhibition form « Cathédrale », as composed in scenes/forme-exposition.ron.
fn cathedrale() -> Graph {
    let text = include_str!("../../../scenes/forme-exposition.ron");
    let Fractal::Graph(g) = syfrack_core::Scene::from_ron(text).unwrap().fractal;
    g
}

fn graphs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../graphs")
}

fn to_ron(g: &Graph) -> String {
    ron::ser::to_string_pretty(g, ron::ser::PrettyConfig::new()).unwrap()
}

#[test]
fn the_composed_forms_match_their_files() {
    for (name, g) in [
        ("cathedrale", cathedrale()),
        ("boite-bulbe", box_bulb()),
        ("menger-kaleido", menger_kaleido()),
        ("evidement", evidement()),
    ] {
        assert_eq!(Graph::bundled(name).unwrap(), g, "{name}");
    }
}

#[test]
#[ignore = "writes graphs/"]
fn write_library_graphs() {
    let dir = graphs_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let all: [(&str, Graph); 4] = [
        ("cathedrale", cathedrale()),
        ("boite-bulbe", box_bulb()),
        ("menger-kaleido", menger_kaleido()),
        ("evidement", evidement()),
    ];
    for (name, g) in all {
        g.validate().unwrap_or_else(|e| panic!("{name}: {e}"));
        std::fs::write(dir.join(format!("{name}.ron")), to_ron(&g)).unwrap();
    }
}
