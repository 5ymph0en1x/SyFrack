//! The form graph: nodes, links and parameters. Validation lives in
//! `validate.rs`, the CPU interpreter in `eval.rs`, the forms shipped in
//! `graphs/` in `bundled.rs`.

pub mod bundled;
pub mod eval;
pub mod language;
pub mod validate;

use crate::params::{join, visit_u32, visit_vec3, Bounds, Params};
use glam::Vec3;
use serde::{Deserialize, Serialize};

pub type NodeId = u32;
pub const MAX_NODES: usize = 1536;
pub const MAX_SCALARS: usize = 8192;
pub const MAX_PLACEMENTS: usize = 128;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Independent world transform of a copy of a locally defined distance field.
/// Angles are XYZ Euler degrees; strictly positive scale preserves orientation.
pub struct Placement {
    pub position: Vec3,
    pub angles: Vec3,
    pub scale: Vec3,
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            angles: Vec3::ZERO,
            scale: Vec3::ONE,
        }
    }
}
pub const MAX_DEPTH: u32 = 3;
pub const MAX_ITER_PRODUCT: u32 = 256;

pub const SAFETY: Bounds = Bounds::new(0.1, 1.0, &[(0.5, 0.9)]);
pub const OFFSET: Bounds = Bounds::new(-10.0, 10.0, &[(-2.0, 2.0)]);
pub const ANGLE_DEG: Bounds = Bounds::new(-180.0, 180.0, &[(-45.0, 45.0)]);
pub const SCALE: Bounds = Bounds::new(-4.0, 4.0, &[(-3.0, -1.5), (1.5, 3.0)]);
pub const AXIS_SCALE: Bounds = Bounds::new(0.001, 100.0, &[(0.25, 4.0)]);
pub const MATERIAL_INDEX: Bounds = Bounds::new(0.0, 8.0, &[(0.0, 8.0)]);
pub const BOUND_MARGIN: Bounds = Bounds::new(0.001, 10.0, &[(0.05, 0.5)]);
pub const MIRROR_OFFSET: Bounds = Bounds::new(-10.0, 10.0, &[(-2.0, 2.0)]);
pub const FOLD_LIMIT: Bounds = Bounds::new(0.5, 2.0, &[(0.8, 1.2)]);
pub const AXIS_FOLD_LIMIT: Bounds = Bounds::new(0.01, 4.0, &[(0.5, 1.5)]);
pub const INVERSION_CORE: Bounds = Bounds::new(0.0001, 2.0, &[(0.05, 0.2)]);
pub const INVERSION_RADIUS: Bounds = Bounds::new(0.01, 10.0, &[(0.8, 1.5)]);
pub const MIN_RADIUS: Bounds = Bounds::new(0.05, 1.0, &[(0.25, 0.75)]);
pub const FIXED_RADIUS: Bounds = Bounds::new(0.5, 2.0, &[(1.0, 1.0)]);
pub const MENGER_SCALE: Bounds = Bounds::new(2.0, 4.0, &[(2.5, 3.5)]);
pub const MENGER_OFFSET: Bounds = Bounds::new(0.5, 2.0, &[(0.8, 1.2)]);
pub const PLANES: Bounds = Bounds::new(2.0, 12.0, &[(3.0, 8.0)]);
pub const PHASE: Bounds = Bounds::new(-180.0, 180.0, &[(-45.0, 45.0)]);
pub const POWER: Bounds = Bounds::new(2.0, 16.0, &[(6.0, 10.0)]);
pub const RATE: Bounds = Bounds::new(-2.0, 2.0, &[(-0.5, 0.5)]);
pub const PERIOD: Bounds = Bounds::new(0.1, 100.0, &[(1.0, 10.0)]);
pub const ITERATIONS: Bounds = Bounds::new(1.0, 64.0, &[(6.0, 16.0)]);
pub const BAILOUT: Bounds = Bounds::new(2.0, 1.0e6, &[(4.0, 8.0)]);
pub const SLOT: Bounds = Bounds::new(0.0, 3.0, &[(0.0, 3.0)]);
pub const TRAP_SCALE: Bounds = Bounds::new(0.01, 100.0, &[(1.0, 4.0)]);
pub const RADIUS: Bounds = Bounds::new(0.01, 100.0, &[(0.5, 2.0)]);
pub const HALF: Bounds = Bounds::new(0.01, 500.0, &[(0.5, 2.0)]);
pub const PLANE_OFFSET: Bounds = Bounds::new(-100.0, 100.0, &[(-2.0, 2.0)]);
pub const INSET: Bounds = Bounds::new(0.0, 2.0, &[(0.0, 1.5)]);
pub const SMOOTH_K: Bounds = Bounds::new(0.001, 10.0, &[(0.05, 0.5)]);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PortType {
    Point,
    Distance,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Measure {
    Norm,
    AxisX,
    AxisY,
    AxisZ,
    PlaneY,
    AngleXY,
    AngleXZ,
    AngleYZ,
    LogNorm,
}

/// How a trap records values along the orbit. Empty slots initialise on first write.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrapMode {
    #[default]
    Minimum,
    Maximum,
    Last,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum NodeKind {
    Input,
    Output,
    Translate {
        offset: Vec3,
    },
    Rotate {
        angles: Vec3,
    },
    Scale {
        factor: f32,
    },
    /// Positive, independent coordinate scales; distances use the largest scale.
    ScaleAxes {
        factors: Vec3,
    },
    Mirror {
        normal: Vec3,
        offset: f32,
    },
    BoxFold {
        limit: f32,
    },
    /// Independent fold widths; unchecked axes pass through unchanged.
    BoxFoldAxes {
        limit: Vec3,
        x: bool,
        y: bool,
        z: bool,
    },
    /// Spherical inversion with a continuous linear core around the singularity.
    SphereInversion {
        radius: f32,
        min_radius: f32,
    },
    SphereFold {
        min_radius: f32,
        fixed_radius: f32,
    },
    Abs {
        x: bool,
        y: bool,
        z: bool,
    },
    MengerFold {
        scale: f32,
        offset: Vec3,
    },
    Kaleido {
        planes: u32,
        phase: f32,
    },
    Power {
        power: f32,
    },
    AddOrigin,
    /// Reinject the point entering the current Iterate, including its derivative.
    /// Outside an Iterate this uses the point entering this node.
    AddSeed {
        multiplier: Vec3,
    },
    /// z += multiplier * world_input + julia. Zero multiplier gives Julia mode.
    AddConstant {
        multiplier: Vec3,
        julia: Vec3,
    },
    Twist {
        rate: f32,
    },
    Bend {
        rate: f32,
    },
    Tile {
        period: Vec3,
    },
    /// Finite, centred lattice. Keep the repeated form inside half a period.
    RepeatFinite {
        period: Vec3,
        count: [u32; 3],
    },
    Iterate {
        iterations: u32,
        bailout: f32,
        body: Vec<NodeId>,
    },
    Trap {
        slot: u32,
        measure: Measure,
        scale: f32,
        #[serde(default)]
        mode: TrapMode,
    },
    Sphere {
        radius: f32,
    },
    /// Circular torus around the Y axis.
    Torus {
        radius: f32,
        tube: f32,
    },
    /// Capped cylinder along the Y axis.
    Cylinder {
        radius: f32,
        half_height: f32,
    },
    Box {
        half: Vec3,
    },
    /// Rounded corners inside the original box extents.
    RoundBox {
        half: Vec3,
        radius: f32,
    },
    /// Eroded ellipsoid. Bounded seeded relief with a conservative distance.
    Rock {
        half: Vec3,
        erosion: f32,
        seed: u32,
    },
    Plane {
        normal: Vec3,
        offset: f32,
    },
    NormFence {
        inset: f32,
    },
    BulbFence,
    /// Assign a named finish to a distance branch. Zero keeps the global material.
    MaterialTag {
        index: u32,
    },
    /// Clip a distance branch to a box in the coordinates of port 0.
    /// The GPU can skip that branch outside the box plus its evaluation margin.
    Bounded {
        half: Vec3,
        margin: f32,
    },
    /// Copies a local distance branch with independent transforms and exact CSG.
    Instances {
        placements: Vec<Placement>,
        half: Vec3,
        margin: f32,
    },
    Union,
    Intersection,
    Subtraction,
    SmoothUnion {
        k: f32,
    },
    SmoothIntersection {
        k: f32,
    },
}

const P: &[PortType] = &[PortType::Point];
const D: &[PortType] = &[PortType::Distance];
const DD: &[PortType] = &[PortType::Distance, PortType::Distance];
const PD: &[PortType] = &[PortType::Point, PortType::Distance];
const NONE: &[PortType] = &[];

impl NodeKind {
    pub fn label(&self) -> &'static str {
        use NodeKind::*;
        match self {
            Input => syfrack_i18n::tr!("Entrée"),
            Output => syfrack_i18n::tr!("Sortie"),
            Translate { .. } => "Translation",
            Rotate { .. } => "Rotation",
            Scale { .. } => syfrack_i18n::tr!("Échelle"),
            ScaleAxes { .. } => syfrack_i18n::tr!("Échelle par axe"),
            Mirror { .. } => syfrack_i18n::tr!("Miroir"),
            BoxFold { .. } => syfrack_i18n::tr!("Pli boîte"),
            BoxFoldAxes { .. } => syfrack_i18n::tr!("Pli par axe"),
            SphereInversion { .. } => syfrack_i18n::tr!("Inversion sphérique"),
            SphereFold { .. } => syfrack_i18n::tr!("Pli sphère"),
            Abs { .. } => syfrack_i18n::tr!("Valeur absolue"),
            MengerFold { .. } => syfrack_i18n::tr!("Pli Menger"),
            Kaleido { .. } => syfrack_i18n::tr!("Kaléidoscope"),
            Power { .. } => syfrack_i18n::tr!("Puissance polaire"),
            AddOrigin => syfrack_i18n::tr!("Ajouter l'origine"),
            AddSeed { .. } => syfrack_i18n::tr!("Réinjecter le point local"),
            AddConstant { .. } => syfrack_i18n::tr!("Constante fractale"),
            Twist { .. } => syfrack_i18n::tr!("Torsion"),
            Bend { .. } => syfrack_i18n::tr!("Flexion"),
            Tile { .. } => syfrack_i18n::tr!("Répétition"),
            RepeatFinite { .. } => syfrack_i18n::tr!("Répétition limitée"),
            Iterate { .. } => syfrack_i18n::tr!("Répéter"),
            Trap { .. } => syfrack_i18n::tr!("Piège"),
            Sphere { .. } => syfrack_i18n::tr!("Sphère"),
            Torus { .. } => syfrack_i18n::tr!("Tore"),
            Cylinder { .. } => syfrack_i18n::tr!("Cylindre"),
            Box { .. } => syfrack_i18n::tr!("Boîte"),
            RoundBox { .. } => syfrack_i18n::tr!("Boîte arrondie"),
            Rock { .. } => syfrack_i18n::tr!("Roche érodée"),
            Plane { .. } => syfrack_i18n::tr!("Plan"),
            NormFence { .. } => syfrack_i18n::tr!("Clôture norme"),
            BulbFence => syfrack_i18n::tr!("Clôture bulbe"),
            MaterialTag { .. } => syfrack_i18n::tr!("Matériau de branche"),
            Bounded { .. } => syfrack_i18n::tr!("Volume de détail"),
            Instances { .. } => syfrack_i18n::tr!("Copies indépendantes"),
            Union => "Union",
            Intersection => "Intersection",
            Subtraction => syfrack_i18n::tr!("Soustraction"),
            SmoothUnion { .. } => syfrack_i18n::tr!("Union lisse"),
            SmoothIntersection { .. } => syfrack_i18n::tr!("Intersection lisse"),
        }
    }

    pub fn is_point_to_point(&self) -> bool {
        use NodeKind::*;
        matches!(
            self,
            Translate { .. }
                | Rotate { .. }
                | Scale { .. }
                | ScaleAxes { .. }
                | Mirror { .. }
                | BoxFold { .. }
                | BoxFoldAxes { .. }
                | SphereInversion { .. }
                | SphereFold { .. }
                | Abs { .. }
                | MengerFold { .. }
                | Kaleido { .. }
                | Power { .. }
                | AddOrigin
                | AddSeed { .. }
                | AddConstant { .. }
                | Twist { .. }
                | Bend { .. }
                | Tile { .. }
                | RepeatFinite { .. }
                | Iterate { .. }
                | Trap { .. }
        )
    }

    pub fn is_fence(&self) -> bool {
        use NodeKind::*;
        matches!(
            self,
            Sphere { .. }
                | Torus { .. }
                | Cylinder { .. }
                | Box { .. }
                | RoundBox { .. }
                | Rock { .. }
                | Plane { .. }
                | NormFence { .. }
                | BulbFence
        )
    }

    pub fn is_combine(&self) -> bool {
        use NodeKind::*;
        matches!(
            self,
            Union | Intersection | Subtraction | SmoothUnion { .. } | SmoothIntersection { .. }
        )
    }

    pub fn inputs(&self) -> &'static [PortType] {
        match self {
            NodeKind::Input => NONE,
            NodeKind::Output => D,
            NodeKind::MaterialTag { .. } | NodeKind::Instances { .. } => D,
            NodeKind::Bounded { .. } => PD,
            k if k.is_combine() => DD,
            _ => P,
        }
    }

    pub fn outputs(&self) -> &'static [PortType] {
        match self {
            NodeKind::Output => NONE,
            NodeKind::MaterialTag { .. }
            | NodeKind::Bounded { .. }
            | NodeKind::Instances { .. } => D,
            k if k.is_fence() || k.is_combine() => D,
            _ => P,
        }
    }

    /// Everything that changes the generated shader: the variant, the
    /// integer/boolean fields and the body. Scalars are omitted except for the
    /// Power 6/8 shader family, which uses a shared direct polynomial kernel.
    pub fn topology_key(&self) -> String {
        use NodeKind::*;
        match self {
            Power { power } if *power == 6.0 || *power == 8.0 => "PowerEven68".into(),
            Abs { x, y, z } => format!("Abs({x},{y},{z})"),
            BoxFoldAxes { x, y, z, .. } => format!("BoxFoldAxes({x},{y},{z})"),
            Kaleido { planes, .. } => format!("Kaleido({planes})"),
            Iterate {
                iterations, body, ..
            } => format!("Iterate({iterations},{body:?})"),
            Trap {
                slot,
                measure,
                mode,
                ..
            } => format!("Trap({slot},{measure:?},{mode:?})"),
            Instances { placements, .. } => format!("Instances({})", placements.len()),
            other => {
                let dbg = format!("{other:?}");
                dbg.split([' ', '{', '(']).next().unwrap_or("").to_string()
            }
        }
    }
}

#[test]
fn power_shader_families_keep_valid_parameter_updates() {
    let key = |power| NodeKind::Power { power }.topology_key();
    assert_eq!(key(6.0), key(8.0));
    assert_eq!(key(6.25), key(7.25));
    assert_ne!(key(8.0), key(8.25));
    assert_ne!(key(6.0), key(16.0));
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: NodeId,
    pub kind: NodeKind,
    pub name: String,
    pub pos: [f32; 2],
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Link {
    pub from: (NodeId, u32),
    pub to: (NodeId, u32),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub links: Vec<Link>,
    /// Global step multiplier, combined with the Lipschitz bound (eval.rs).
    pub safety: f32,
    /// Enclosing radius used when the estimate (eval.rs) is not available.
    pub bounding_radius_hint: f32,
}

impl Graph {
    pub fn new_minimal() -> Self {
        let node = |id, kind: NodeKind, x| Node {
            id,
            name: kind.label().to_string(),
            kind,
            pos: [x, 0.0],
            enabled: true,
        };
        Self {
            nodes: vec![
                node(0, NodeKind::Input, 0.0),
                node(1, NodeKind::Sphere { radius: 1.0 }, 240.0),
                node(2, NodeKind::Output, 480.0),
            ],
            links: vec![
                Link {
                    from: (0, 0),
                    to: (1, 0),
                },
                Link {
                    from: (1, 0),
                    to: (2, 0),
                },
            ],
            safety: 1.0,
            bounding_radius_hint: 2.0,
        }
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }
    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }
    pub fn next_id(&self) -> NodeId {
        self.nodes.iter().map(|n| n.id + 1).max().unwrap_or(0)
    }
    pub fn input_of(&self, to: NodeId, port: u32) -> Option<(NodeId, u32)> {
        self.links
            .iter()
            .find(|l| l.to == (to, port))
            .map(|l| l.from)
    }

    pub fn scalar_count(&self) -> usize {
        let mut copy = self.clone();
        let mut n = 0;
        copy.visit_params("", &mut |path, _, _| {
            if path.starts_with("nodes[") {
                n += 1;
            }
        });
        n
    }
}

impl Params for Graph {
    fn visit_params(&mut self, prefix: &str, f: &mut dyn FnMut(&str, &mut f32, Bounds)) {
        f(&join(prefix, "safety"), &mut self.safety, SAFETY);
        for node in &mut self.nodes {
            let p = format!("{}[{}]", join(prefix, "nodes"), node.id);
            use NodeKind::*;
            match &mut node.kind {
                Translate { offset } => visit_vec3(&join(&p, "offset"), offset, OFFSET, f),
                Rotate { angles } => visit_vec3(&join(&p, "angles"), angles, ANGLE_DEG, f),
                Scale { factor } => f(&join(&p, "factor"), factor, SCALE),
                ScaleAxes { factors } => visit_vec3(&join(&p, "factors"), factors, AXIS_SCALE, f),
                Mirror { normal, offset } => {
                    visit_vec3(&join(&p, "normal"), normal, crate::params::DIRECTION, f);
                    f(&join(&p, "offset"), offset, MIRROR_OFFSET);
                }
                BoxFold { limit } => f(&join(&p, "limit"), limit, FOLD_LIMIT),
                BoxFoldAxes { limit, .. } => {
                    visit_vec3(&join(&p, "limit"), limit, AXIS_FOLD_LIMIT, f);
                }
                SphereInversion { radius, min_radius } => {
                    f(&join(&p, "radius"), radius, INVERSION_RADIUS);
                    f(&join(&p, "min_radius"), min_radius, INVERSION_CORE);
                }
                AddConstant { multiplier, julia } => {
                    visit_vec3(&join(&p, "multiplier"), multiplier, SCALE, f);
                    visit_vec3(&join(&p, "julia"), julia, OFFSET, f);
                }
                AddSeed { multiplier } => visit_vec3(&join(&p, "multiplier"), multiplier, SCALE, f),
                SphereFold {
                    min_radius,
                    fixed_radius,
                } => {
                    f(&join(&p, "min_radius"), min_radius, MIN_RADIUS);
                    f(&join(&p, "fixed_radius"), fixed_radius, FIXED_RADIUS);
                }
                MengerFold { scale, offset } => {
                    f(&join(&p, "scale"), scale, MENGER_SCALE);
                    visit_vec3(&join(&p, "offset"), offset, MENGER_OFFSET, f);
                }
                Kaleido { planes, phase } => {
                    visit_u32(&join(&p, "planes"), planes, PLANES, f);
                    f(&join(&p, "phase"), phase, PHASE);
                }
                Power { power } => f(&join(&p, "power"), power, POWER),
                Twist { rate } | Bend { rate } => f(&join(&p, "rate"), rate, RATE),
                Tile { period } => visit_vec3(&join(&p, "period"), period, PERIOD, f),
                Instances {
                    placements,
                    half,
                    margin,
                } => {
                    visit_vec3(&join(&p, "half"), half, HALF, f);
                    f(&join(&p, "margin"), margin, BOUND_MARGIN);
                    for (i, placement) in placements.iter_mut().enumerate() {
                        let prefix = format!("{p}.placements[{i}]");
                        visit_vec3(
                            &join(&prefix, "position"),
                            &mut placement.position,
                            Bounds::new(-1000.0, 1000.0, &[(-100.0, 100.0)]),
                            f,
                        );
                        visit_vec3(
                            &join(&prefix, "angles"),
                            &mut placement.angles,
                            ANGLE_DEG,
                            f,
                        );
                        visit_vec3(
                            &join(&prefix, "scale"),
                            &mut placement.scale,
                            Bounds::new(0.05, 20.0, &[(0.5, 2.0)]),
                            f,
                        );
                    }
                }
                RepeatFinite { period, count } => {
                    visit_vec3(&join(&p, "period"), period, PERIOD, f);
                    for (i, n) in count.iter_mut().enumerate() {
                        visit_u32(
                            &format!("{p}.count.{}", ["x", "y", "z"][i]),
                            n,
                            Bounds::new(1.0, 32.0, &[(1.0, 12.0)]),
                            f,
                        );
                    }
                }
                Iterate {
                    iterations,
                    bailout,
                    ..
                } => {
                    visit_u32(&join(&p, "iterations"), iterations, ITERATIONS, f);
                    f(&join(&p, "bailout"), bailout, BAILOUT);
                }
                Trap { slot, scale, .. } => {
                    visit_u32(&join(&p, "slot"), slot, SLOT, f);
                    f(&join(&p, "scale"), scale, TRAP_SCALE);
                }
                Sphere { radius } => f(&join(&p, "radius"), radius, RADIUS),
                Torus { radius, tube } => {
                    f(&join(&p, "radius"), radius, RADIUS);
                    f(&join(&p, "tube"), tube, RADIUS);
                }
                Cylinder {
                    radius,
                    half_height,
                } => {
                    f(&join(&p, "radius"), radius, RADIUS);
                    f(&join(&p, "half_height"), half_height, HALF);
                }
                MaterialTag { index } => visit_u32(&join(&p, "index"), index, MATERIAL_INDEX, f),
                Bounded { half, margin } => {
                    visit_vec3(&join(&p, "half"), half, HALF, f);
                    f(&join(&p, "margin"), margin, BOUND_MARGIN);
                }
                Box { half } => visit_vec3(&join(&p, "half"), half, HALF, f),
                RoundBox { half, radius } => {
                    visit_vec3(&join(&p, "half"), half, HALF, f);
                    f(&join(&p, "radius"), radius, INSET);
                }
                Rock {
                    half,
                    erosion,
                    seed,
                } => {
                    visit_vec3(&join(&p, "half"), half, HALF, f);
                    f(
                        &join(&p, "erosion"),
                        erosion,
                        Bounds::new(0.0, 0.6, &[(0.0, 0.4)]),
                    );
                    visit_u32(
                        &join(&p, "seed"),
                        seed,
                        Bounds::new(0.0, 65535.0, &[(0.0, 65535.0)]),
                        f,
                    );
                }
                Plane { normal, offset } => {
                    visit_vec3(&join(&p, "normal"), normal, crate::params::DIRECTION, f);
                    f(&join(&p, "offset"), offset, PLANE_OFFSET);
                }
                NormFence { inset } => f(&join(&p, "inset"), inset, INSET),
                SmoothUnion { k } | SmoothIntersection { k } => f(&join(&p, "k"), k, SMOOTH_K),
                Input
                | Output
                | Abs { .. }
                | AddOrigin
                | BulbFence
                | Union
                | Intersection
                | Subtraction => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Params;

    #[test]
    fn rounded_box_keeps_its_extents_and_rounds_the_corner() {
        let mut g = Graph::new_minimal();
        g.nodes[1].kind = NodeKind::RoundBox {
            half: Vec3::ONE,
            radius: 0.2,
        };
        assert!(g.evaluate(Vec3::X).distance.abs() < 1e-6);
        assert!((g.evaluate(Vec3::ONE).distance - 0.2 * (3.0f32.sqrt() - 1.0)).abs() < 1e-6);
        assert_eq!(g.evaluate(Vec3::ZERO).distance, -1.0);
        assert_eq!(g.node(1).unwrap().kind.outputs(), &[PortType::Distance]);
        g.validate().unwrap();
    }

    #[test]
    fn minimal_graph_round_trips_through_ron() {
        let g = Graph::new_minimal();
        let text = ron::ser::to_string_pretty(&g, ron::ser::PrettyConfig::new()).unwrap();
        let back: Graph = ron::from_str(&text).unwrap();
        assert_eq!(back, g);
        assert_eq!(g.nodes.len(), 3);
        assert_eq!(g.links.len(), 2);
    }

    #[test]
    fn unknown_field_in_a_node_is_rejected() {
        let text = r#"(nodes: [(id: 0, kind: Input, name: "e", pos: (0.0, 0.0), enabled: true, bogus: 1)], links: [], safety: 0.7, bounding_radius_hint: 2.0)"#;
        assert!(ron::from_str::<Graph>(text).is_err());
    }

    #[test]
    fn ports_follow_the_node_family() {
        assert_eq!(NodeKind::Input.inputs(), &[]);
        assert_eq!(NodeKind::Input.outputs(), &[PortType::Point]);
        assert_eq!(
            NodeKind::BoxFold { limit: 1.0 }.inputs(),
            &[PortType::Point]
        );
        assert_eq!(
            NodeKind::NormFence { inset: 0.0 }.outputs(),
            &[PortType::Distance]
        );
        assert_eq!(
            NodeKind::Union.inputs(),
            &[PortType::Distance, PortType::Distance]
        );
        assert_eq!(NodeKind::Output.inputs(), &[PortType::Distance]);
        assert_eq!(NodeKind::Output.outputs(), &[]);
        assert!(NodeKind::Iterate {
            iterations: 4,
            bailout: 4.0,
            body: vec![]
        }
        .is_point_to_point());
        assert!(!NodeKind::Sphere { radius: 1.0 }.is_point_to_point());
    }

    #[test]
    fn params_are_addressed_by_node_id_and_name() {
        let mut g = Graph::new_minimal();
        let mut seen = Vec::new();
        g.visit_params("fractal", &mut |path, v, b| {
            seen.push((path.to_string(), *v, b.min, b.max));
        });
        assert!(seen
            .iter()
            .any(|(p, v, lo, hi)| p == "fractal.nodes[1].radius"
                && *v == 1.0
                && *lo == 0.01
                && *hi == 100.0));
        assert!(seen.iter().any(|(p, ..)| p == "fractal.safety"));
    }

    #[test]
    fn topology_key_ignores_scalar_values_but_not_structure() {
        let a = NodeKind::Scale { factor: 2.0 }.topology_key();
        let b = NodeKind::Scale { factor: -1.5 }.topology_key();
        assert_eq!(a, b);
        let i1 = NodeKind::Iterate {
            iterations: 8,
            bailout: 4.0,
            body: vec![1],
        }
        .topology_key();
        let i2 = NodeKind::Iterate {
            iterations: 9,
            bailout: 4.0,
            body: vec![1],
        }
        .topology_key();
        assert_ne!(i1, i2);
        let i3 = NodeKind::Iterate {
            iterations: 8,
            bailout: 9.0,
            body: vec![1],
        }
        .topology_key();
        assert_eq!(i1, i3);
    }

    #[test]
    fn scalar_count_counts_vec3_as_three() {
        let mut g = Graph::new_minimal();
        g.nodes.push(Node {
            id: 3,
            kind: NodeKind::Translate { offset: Vec3::ONE },
            name: "t".into(),
            pos: [0.0; 2],
            enabled: true,
        });
        assert_eq!(g.scalar_count(), 1 + 3); // radius + offset
    }
}
