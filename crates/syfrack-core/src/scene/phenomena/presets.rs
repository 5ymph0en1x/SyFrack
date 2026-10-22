//! Small, editable experiments. Names describe the model, not scientific novelty.
use super::*;
use crate::{CameraMode, Color, Dimension, Fractal, Graph, Look, Scene, Tonemap};
use glam::Vec3;

pub const PRESETS: &[(&str, &str)] = &[
    ("interference", "Jardin des interférences"),
    ("morphogenesis", "Récif de morphogenèse"),
    ("vortex", "Vortex de condensat"),
    ("lightning", "Arbre de foudre"),
    ("schottky", "Perles de Schottky"),
    ("quaternion", "Coupe quaternionique"),
    ("resonance", "Résonances du domaine"),
    ("iridescence", "Iris de nacre"),
    ("kerr", "Observatoire de Kerr"),
    ("dispersion", "Prisme chromatique"),
];
fn node(id: u32, op: FieldOp, inputs: &[u32], name: &str) -> FieldNode {
    FieldNode {
        id,
        name: name.into(),
        op,
        inputs: inputs.into(),
        pos: [180.0 * (id % 5) as f32, 120.0 * (id / 5) as f32],
    }
}
fn palette() -> crate::Ramp {
    crate::Ramp {
        stops: vec![
            [0.003, 0.005, 0.02],
            [0.015, 0.08, 0.2],
            [0.04, 0.4, 0.45],
            [0.8, 0.9, 0.7],
            [1.0, 0.5, 0.08],
        ],
        ..Default::default()
    }
}
pub fn configuration(name: &str) -> Result<Phenomena, crate::SceneError> {
    let mut p = Phenomena::default();
    p.domain.half = Vec3::splat(3.0);
    p.bindings.palette = palette();
    p.fields
        .nodes
        .push(node(0, FieldOp::Position, &[], "Position"));
    match name {
        "interference" => {
            p.fields.nodes.extend([
                node(
                    1,
                    FieldOp::GaussianWave {
                        center: Vec3::new(-0.6, 0.0, 0.0),
                        width: 1.3,
                        wavevector: Vec3::new(5.0, 2.0, 1.0),
                        frequency: 1.0,
                    },
                    &[0],
                    "Première onde",
                ),
                node(
                    2,
                    FieldOp::GaussianWave {
                        center: Vec3::new(0.6, 0.0, 0.0),
                        width: 1.3,
                        wavevector: Vec3::new(-4.0, 3.0, -1.0),
                        frequency: 1.4,
                    },
                    &[0],
                    "Deuxième onde",
                ),
                node(3, FieldOp::Add, &[1, 2], "Superposition cohérente"),
                node(4, FieldOp::Intensity, &[3], "Intensité"),
                node(5, FieldOp::Phase, &[3], "Phase"),
            ]);
            p.bindings.color = Some(5);
            p.bindings.density = Some(4);
            p.bindings.density_gain = 0.5;
            p.bindings.emission_gain = 0.35;
        }
        "morphogenesis" => {
            p.domain.resolution = [96, 96, 1];
            p.simulations.push(Simulation {
                model: SimulationModel::GrayScott {
                    diffusion: [0.00064, 0.00032],
                    feed: 0.035,
                    kill: 0.062,
                },
                steps: 1024,
                dt: 0.5,
                seeds: [
                    (-0.85, -0.7),
                    (0.65, -0.8),
                    (-0.8, 0.65),
                    (0.8, 0.9),
                    (0.0, 0.0),
                ]
                .into_iter()
                .map(|(x, y)| SeedBlob {
                    center: Vec3::new(x, y, 0.0),
                    radius: 0.16,
                    ..Default::default()
                })
                .collect(),
                ..Default::default()
            });
            p.fields.nodes.push(node(
                1,
                FieldOp::Snapshot {
                    simulation: 0,
                    channel: 1,
                },
                &[0],
                "Concentration du morphogène",
            ));
            p.bindings.color = Some(1);
            p.bindings.palette.frequency = 2.5;
            p.bindings.displacement = Some(1);
            p.bindings.displacement_amount = 0.1;
        }
        "vortex" => {
            p.domain.resolution = [40, 40, 40];
            p.simulations.push(Simulation {
                name: "Condensat, modèle de Gross–Pitaevskii".into(),
                model: SimulationModel::GrossPitaevskii {
                    dispersion: 0.03,
                    interaction: 0.5,
                    damping: 0.0,
                },
                steps: 32,
                dt: 0.01,
                seeds: vec![SeedBlob {
                    radius: 1.1,
                    winding: 1,
                    ..Default::default()
                }],
                ..Default::default()
            });
            p.fields.nodes.extend([
                node(
                    1,
                    FieldOp::Snapshot {
                        simulation: 0,
                        channel: 2,
                    },
                    &[0],
                    "Densité du condensat",
                ),
                node(
                    2,
                    FieldOp::Snapshot {
                        simulation: 0,
                        channel: 3,
                    },
                    &[0],
                    "Phase du condensat",
                ),
            ]);
            p.bindings.color = Some(2);
            p.bindings.density = Some(1);
            p.bindings.emission_gain = 0.5;
        }
        "lightning" => {
            p.domain.resolution = [48, 80, 1];
            p.domain.half = Vec3::new(2.5, 4.0, 1.0);
            p.simulations.push(Simulation {
                name: "Décharge diélectrique".into(),
                model: SimulationModel::DielectricGrowth {
                    eta: 2.5,
                    relaxation_steps: 48,
                    branches_per_step: 1,
                },
                steps: 128,
                dt: 1.0,
                seeds: vec![SeedBlob {
                    center: Vec3::ZERO,
                    radius: 0.16,
                    ..Default::default()
                }],
                ..Default::default()
            });
            p.fields.nodes.extend([
                node(
                    1,
                    FieldOp::Snapshot {
                        simulation: 0,
                        channel: 0,
                    },
                    &[0],
                    "Arbre de décharge",
                ),
                node(
                    2,
                    FieldOp::Snapshot {
                        simulation: 0,
                        channel: 1,
                    },
                    &[0],
                    "Potentiel électrique",
                ),
            ]);
            p.bindings.color = Some(1);
            p.bindings.emission = Some(1);
            p.bindings.emission_gain = 0.5;
            p.bindings.palette.stops = vec![
                [0.002, 0.004, 0.015],
                [0.01, 0.08, 0.2],
                [0.03, 0.4, 0.9],
                [0.6, 0.85, 1.0],
                [1.0, 1.0, 1.0],
            ];
            p.guides.push(ArtisticGuide {
                center: Vec3::new(0.0, 3.0, 0.0),
                radius: 2.0,
                strength: 2.0,
                kind: GuideKind::Attractor,
            });
        }
        "schottky" => {
            p.domain.resolution = [192, 192, 1];
            p.domain.half = Vec3::splat(2.5);
            p.simulations.push(Simulation {
                name: "Groupe de Schottky classique".into(),
                model: SimulationModel::Schottky {
                    pairs: vec![
                        CirclePair {
                            source: [-1.2, 0.0],
                            target: [1.2, 0.0],
                            radius: 0.5,
                            twist: 25.0,
                        },
                        CirclePair {
                            source: [0.0, 1.3],
                            target: [0.0, -1.3],
                            radius: 0.55,
                            twist: -15.0,
                        },
                    ],
                    points: 400_000,
                    burn_in: 64,
                },
                steps: 0,
                ..Default::default()
            });
            p.fields.nodes.push(node(
                1,
                FieldOp::Snapshot {
                    simulation: 0,
                    channel: 0,
                },
                &[0],
                "Densité de l'ensemble limite",
            ));
            p.bindings.color = Some(1);
            p.bindings.palette.frequency = 2.0;
        }
        "quaternion" => {
            p.domain.resolution = [64, 64, 64];
            p.domain.half = Vec3::splat(1.7);
            p.fields.nodes.extend([
                node(
                    1,
                    FieldOp::QuaternionJulia {
                        constant: [0.2, -0.3, 0.1, -0.4],
                        slice: 0.0,
                        iterations: 24,
                        bailout: 8.0,
                    },
                    &[0],
                    "Julia quaternionique, coupe 3D",
                ),
                node(2, FieldOp::Length, &[0], "Rayon spatial"),
            ]);
            p.bindings.geometry = Some(1);
            p.bindings.geometry_mode = FieldGeometry::Replace;
            p.bindings.color = Some(2);
            p.bindings.film_thickness = Some(2);
            p.film = Some(ThinFilm::default());
        }
        "resonance" => {
            p.domain.resolution = [41, 41, 1];
            p.domain.half = Vec3::splat(3.0);
            p.fields.nodes.extend([
                node(
                    1,
                    FieldOp::SphereDistance {
                        center: Vec3::ZERO,
                        radius: 2.5,
                    },
                    &[0],
                    "Domaine circulaire",
                ),
                node(2, FieldOp::Scalar { value: -1.0 }, &[], "Intérieur positif"),
                node(3, FieldOp::Scale, &[1, 2], "Masque de la membrane"),
                node(
                    4,
                    FieldOp::Snapshot {
                        simulation: 0,
                        channel: 0,
                    },
                    &[0],
                    "Mode propre de la membrane",
                ),
            ]);
            p.simulations.push(Simulation {
                name: "Mode propre de Dirichlet".into(),
                model: SimulationModel::Resonance {
                    mode: 2,
                    iterations: 2048,
                },
                steps: 0,
                mask: Some(3),
                ..Default::default()
            });
            p.bindings.color = Some(4);
            p.bindings.palette.cyclic = true;
            p.bindings.palette.frequency = 2.0;
        }
        "iridescence" => {
            p.fields.nodes.push(node(
                1,
                FieldOp::GeometryTrap { slot: 0 },
                &[0],
                "Épaisseur guidée par l'orbite",
            ));
            p.bindings.film_thickness = Some(1);
            p.film = Some(ThinFilm::default());
        }
        "dispersion" => {
            p.fields.nodes.clear();
            p.dispersion = Some(ChromaticDispersion::default());
        }
        "kerr" => {
            p.fields.nodes.clear();
            p.lens = Some(RelativisticLens {
                center: Vec3::new(0.0, 0.0, 3.0),
                mass: 0.25,
                spin: 0.7,
                ..Default::default()
            });
        }
        _ => {
            return Err(crate::SceneError::Graph(format!(
                "unknown natural-field preset {name}"
            )))
        }
    }
    p.validate()?;
    Ok(p)
}
pub fn scene(name: &str) -> Result<Scene, crate::SceneError> {
    let flat = matches!(
        name,
        "morphogenesis" | "lightning" | "schottky" | "resonance"
    );
    let mut s = if flat {
        Scene::example_2d()
    } else {
        Scene::example_for("mandelbulb")?
    };
    s.phenomena = Some(configuration(name)?);
    s.seed = 19102026;
    if flat {
        s.dimension = Dimension::TwoD;
        s.canvas.zoom = 1.0;
        s.material.texture = None;
        s.look.tonemap = Tonemap::Direct;
    } else {
        s.camera.mode = CameraMode::Free {
            position: Vec3::new(4.0, 2.0, 7.0),
            target: Vec3::ZERO,
            roll: 0.0,
        };
        s.camera.focal_mm = 42.0;
        s.camera.f_number = 0.0;
        s.material.texture = None;
        s.look = Look::neutral();
        s.look.exposure_ev = 1.0;
    }
    if name == "interference" || name == "vortex" {
        s.fractal = Fractal::Graph(Graph::new_minimal());
        if let crate::NodeKind::Sphere { radius } = &mut s.fractal.graph_mut().nodes[1].kind {
            *radius = 0.01;
        }
        s.lights = vec![crate::Light::Environment {
            zenith: Color::Linear([0.001, 0.002, 0.006]),
            horizon: Color::Linear([0.001, 0.002, 0.006]),
            ground: Color::Linear([0.001, 0.002, 0.006]),
            intensity: 1.0,
            rotation: 0.0,
        }];
    }
    if name == "lightning" {
        s.canvas.zoom = 0.5;
    }
    if matches!(name, "interference" | "vortex") {
        s.look.exposure_ev = 0.0;
    }
    if name == "iridescence" {
        s.material.metallic = 1.0;
        s.material.roughness = 0.18;
        if let Some(film) = &mut s.phenomena.as_mut().unwrap().film {
            film.substrate_ior = 1.0;
            film.substrate_extinction = 0.0;
        }
    }
    if name == "kerr" {
        s.camera.mode = CameraMode::Free {
            position: Vec3::new(0.1, 0.2, 7.0),
            target: Vec3::ZERO,
            roll: 0.0,
        };
    }
    if name == "dispersion" {
        s.fractal = Fractal::Graph(Graph::new_minimal());
        s.material.optics = Some(crate::scene::optics::Optics::default());
        s.material.roughness = 0.03;
        s.look.exposure_ev = 0.0;
        s.lights = vec![crate::Light::Environment {
            zenith: Color::Linear([0.02, 0.04, 0.12]),
            horizon: Color::Linear([1.0, 0.7, 0.18]),
            ground: Color::Linear([0.01, 0.02, 0.04]),
            intensity: 1.0,
            rotation: 0.0,
        }];
    }
    s.validate()?;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_natural_preset_validates_and_roundtrips() {
        for (name, _) in PRESETS {
            let s = scene(name).unwrap();
            assert_eq!(s, Scene::from_ron(&s.to_ron()).unwrap(), "{name}");
        }
    }
}
