//! Pure editing operations on a form graph. Each returns the graph in a
//! valid-or-unchanged state; the caller (AppState::edit_graph) validates
//! the whole graph again and owns undo.

use std::collections::HashMap;

use egui::pos2;
use glam::Vec3;
use syfrack_core::{Graph, Link, Measure, Node, NodeId, NodeKind, Params, PortType, SceneError};

use crate::canvas;

/// Every node the add menu offers, with the interesting defaults of the
/// bounds table (graph/mod.rs). Input and Output are not offered: a graph
/// has exactly one of each.
pub static ALL_KINDS: std::sync::LazyLock<Vec<NodeKind>> = std::sync::LazyLock::new(|| {
    vec![
        NodeKind::Translate {
            offset: Vec3::new(0.5, 0.0, 0.0),
        },
        NodeKind::Rotate {
            angles: Vec3::new(0.0, 30.0, 0.0),
        },
        NodeKind::Scale { factor: 2.0 },
        NodeKind::ScaleAxes { factors: Vec3::ONE },
        NodeKind::Mirror {
            normal: Vec3::X,
            offset: 0.0,
        },
        NodeKind::BoxFold { limit: 1.0 },
        NodeKind::BoxFoldAxes {
            limit: Vec3::ONE,
            x: true,
            y: true,
            z: false,
        },
        NodeKind::SphereInversion {
            radius: 1.0,
            min_radius: 0.1,
        },
        NodeKind::SphereFold {
            min_radius: 0.5,
            fixed_radius: 1.0,
        },
        NodeKind::Abs {
            x: true,
            y: true,
            z: true,
        },
        NodeKind::MengerFold {
            scale: 3.0,
            offset: Vec3::ONE,
        },
        NodeKind::Kaleido {
            planes: 6,
            phase: 0.0,
        },
        NodeKind::Power { power: 8.0 },
        NodeKind::AddOrigin,
        NodeKind::AddSeed {
            multiplier: Vec3::ONE,
        },
        NodeKind::AddConstant {
            multiplier: Vec3::ONE,
            julia: Vec3::ZERO,
        },
        NodeKind::Twist { rate: 0.2 },
        NodeKind::Bend { rate: 0.2 },
        NodeKind::Tile {
            period: Vec3::splat(4.0),
        },
        NodeKind::RepeatFinite {
            period: Vec3::splat(4.0),
            count: [3, 1, 3],
        },
        NodeKind::Iterate {
            iterations: 8,
            bailout: 4.0,
            body: Vec::new(),
        },
        NodeKind::Trap {
            slot: 0,
            measure: Measure::Norm,
            scale: 2.0,
            mode: Default::default(),
        },
        NodeKind::Sphere { radius: 1.0 },
        NodeKind::Torus {
            radius: 1.0,
            tube: 0.15,
        },
        NodeKind::Cylinder {
            radius: 1.0,
            half_height: 1.0,
        },
        NodeKind::MaterialTag { index: 0 },
        NodeKind::Bounded {
            half: Vec3::ONE,
            margin: 0.2,
        },
        NodeKind::Box { half: Vec3::ONE },
        NodeKind::RoundBox {
            half: Vec3::ONE,
            radius: 0.1,
        },
        NodeKind::Rock {
            half: Vec3::ONE,
            erosion: 0.25,
            seed: 17,
        },
        NodeKind::Plane {
            normal: Vec3::Y,
            offset: 0.0,
        },
        NodeKind::NormFence { inset: 0.0 },
        NodeKind::BulbFence,
        NodeKind::Union,
        NodeKind::Intersection,
        NodeKind::Subtraction,
        NodeKind::SmoothUnion { k: 0.2 },
        NodeKind::SmoothIntersection { k: 0.2 },
        NodeKind::Instances {
            placements: vec![syfrack_core::Placement::default()],
            half: Vec3::ONE,
            margin: 0.2,
        },
    ]
});

/// The add menu's categories, in menu order.
pub const CATEGORIES: [&str; 4] = [
    "Transformations",
    "Boucle et pièges",
    "Primitives",
    "Combinaisons",
];

/// Index in `CATEGORIES` of a kind.
pub fn category(kind: &NodeKind) -> usize {
    match kind {
        NodeKind::Iterate { .. } | NodeKind::Trap { .. } => 1,
        k if k.is_fence() => 2,
        k if k.is_combine()
            || matches!(
                k,
                NodeKind::MaterialTag { .. }
                    | NodeKind::Bounded { .. }
                    | NodeKind::Instances { .. }
            ) =>
        {
            3
        }
        _ => 0,
    }
}

/// The `ALL_KINDS` entry of `kind`'s variant: its default values.
pub fn default_kind(kind: &NodeKind) -> Option<&'static NodeKind> {
    ALL_KINDS
        .iter()
        .find(|k| std::mem::discriminant(*k) == std::mem::discriminant(kind))
}

/// The default value of one parameter of a node of `kind`'s variant;
/// `field` is the path after `nodes[<id>].` (`factor`, `offset.x`…).
pub fn default_param(kind: &NodeKind, field: &str) -> Option<f32> {
    let mut g = Graph::new_minimal();
    g.nodes.clear();
    g.links.clear();
    g.nodes.push(Node {
        id: 0,
        name: String::new(),
        kind: default_kind(kind)?.clone(),
        pos: [0.0; 2],
        enabled: true,
    });
    let want = format!("nodes[0].{field}");
    let mut found = None;
    g.visit_params("", &mut |path, v, _| {
        if path == want {
            found = Some(*v);
        }
    });
    found
}

/// The first input port of `kind` of type `ty`.
pub fn input_port_of(kind: &NodeKind, ty: PortType) -> Option<u32> {
    kind.inputs()
        .iter()
        .position(|t| *t == ty)
        .map(|i| i as u32)
}

/// The first output port of `kind` of type `ty`.
pub fn output_port_of(kind: &NodeKind, ty: PortType) -> Option<u32> {
    kind.outputs()
        .iter()
        .position(|t| *t == ty)
        .map(|i| i as u32)
}

fn graph_err(msg: impl Into<String>) -> SceneError {
    SceneError::Graph(msg.into())
}

fn is_io(kind: &NodeKind) -> bool {
    matches!(kind, NodeKind::Input | NodeKind::Output)
}

pub fn add_node(g: &mut Graph, kind: NodeKind, pos: [f32; 2]) -> NodeId {
    let id = g.next_id();
    g.nodes.push(Node {
        id,
        name: kind.label().to_string(),
        kind,
        pos,
        enabled: true,
    });
    id
}

pub fn remove_nodes(g: &mut Graph, ids: &[NodeId]) {
    let doomed: Vec<NodeId> = ids
        .iter()
        .copied()
        .filter(|id| g.node(*id).is_some_and(|n| !is_io(&n.kind)))
        .collect();
    g.nodes.retain(|n| !doomed.contains(&n.id));
    g.links
        .retain(|l| !doomed.contains(&l.from.0) && !doomed.contains(&l.to.0));
    for n in &mut g.nodes {
        if let NodeKind::Iterate { body, .. } = &mut n.kind {
            body.retain(|b| !doomed.contains(b));
        }
    }
}

/// Removes nodes like `remove_nodes`, but a node with one input and one
/// output of the same type is bypassed first: its source is wired to every
/// consumer, so deleting a link of a chain keeps the chain.
pub fn dissolve_nodes(g: &mut Graph, ids: &[NodeId]) {
    for id in ids {
        let Some(n) = g.node(*id) else { continue };
        let bypass = match (n.kind.inputs(), n.kind.outputs()) {
            ([a], [b]) => a == b && !is_io(&n.kind),
            _ => false,
        };
        if let (true, Some(src)) = (bypass, g.input_of(*id, 0)) {
            let consumers: Vec<(NodeId, u32)> = g
                .links
                .iter()
                .filter(|l| l.from.0 == *id)
                .map(|l| l.to)
                .collect();
            g.links.retain(|l| l.from.0 != *id && l.to.0 != *id);
            for to in consumers {
                g.links.push(Link { from: src, to });
            }
        }
        remove_nodes(g, &[*id]);
    }
}

/// The Répéter whose body holds `id`, if any.
pub fn frame_of(g: &Graph, id: NodeId) -> Option<NodeId> {
    g.nodes
        .iter()
        .find(|n| g.body_ids_of(n.id).contains(&id))
        .map(|n| n.id)
}

/// True when `to` can be reached from `from` along links.
fn reaches(g: &Graph, from: NodeId, to: NodeId) -> bool {
    let mut stack = vec![from];
    let mut seen = Vec::new();
    while let Some(id) = stack.pop() {
        if id == to {
            return true;
        }
        if seen.contains(&id) {
            continue;
        }
        seen.push(id);
        stack.extend(g.links.iter().filter(|l| l.from.0 == id).map(|l| l.to.0));
    }
    false
}

pub fn connect(g: &mut Graph, from: (NodeId, u32), to: (NodeId, u32)) -> Result<(), SceneError> {
    let a = g
        .node(from.0)
        .ok_or_else(|| graph_err(syfrack_i18n::tr!("nœud source inconnu")))?;
    let b = g
        .node(to.0)
        .ok_or_else(|| graph_err(syfrack_i18n::tr!("nœud cible inconnu")))?;
    let out_ty = a
        .kind
        .outputs()
        .get(from.1 as usize)
        .ok_or_else(|| graph_err(syfrack_i18n::tr!("port de sortie absent")))?;
    let in_ty = b
        .kind
        .inputs()
        .get(to.1 as usize)
        .ok_or_else(|| graph_err(syfrack_i18n::tr!("port d'entrée absent")))?;
    if out_ty != in_ty {
        return Err(graph_err(syfrack_i18n::tr_format!(
            "fil {out_ty:?} vers un port {in_ty:?}",
            in_ty = in_ty,
            out_ty = out_ty
        )));
    }
    if frame_of(g, from.0).is_some() || frame_of(g, to.0).is_some() {
        return Err(graph_err(syfrack_i18n::tr!(
            "un nœud d'un cadre Répéter ne porte pas de fil"
        )));
    }
    // Any cycle, among active nodes or not: the new link closes one when
    // `to` already reaches `from`.
    if from.0 == to.0 || reaches(g, to.0, from.0) {
        return Err(graph_err(syfrack_i18n::tr!("ce fil fermerait une boucle")));
    }
    let mut copy = g.clone();
    copy.links.retain(|l| l.to != to);
    copy.links.push(Link { from, to });
    copy.topo_order()?;
    g.links = copy.links;
    Ok(())
}

pub fn disconnect(g: &mut Graph, to: (NodeId, u32)) {
    g.links.retain(|l| l.to != to);
}

pub fn duplicate(g: &mut Graph, ids: &[NodeId], offset: [f32; 2]) -> Vec<NodeId> {
    fn collect(g: &Graph, id: NodeId, all: &mut Vec<NodeId>) {
        if all.contains(&id) {
            return;
        }
        all.push(id);
        for b in g.body_ids_of(id).to_vec() {
            collect(g, b, all);
        }
    }
    let mut all: Vec<NodeId> = Vec::new();
    for id in ids {
        collect(g, *id, &mut all);
    }
    all.retain(|id| g.node(*id).is_some_and(|n| !is_io(&n.kind)));
    let map: HashMap<NodeId, NodeId> = all.iter().copied().zip(g.next_id()..).collect();
    let mut new_nodes = Vec::new();
    for id in &all {
        let mut n = g.node(*id).expect("collected from the graph").clone();
        n.id = map[id];
        n.pos = [n.pos[0] + offset[0], n.pos[1] + offset[1]];
        if let NodeKind::Iterate { body, .. } = &mut n.kind {
            for b in body.iter_mut() {
                if let Some(m) = map.get(b) {
                    *b = *m;
                }
            }
        }
        new_nodes.push(n);
    }
    let new_links: Vec<Link> = g
        .links
        .iter()
        .filter(|l| map.contains_key(&l.from.0) && map.contains_key(&l.to.0))
        .map(|l| Link {
            from: (map[&l.from.0], l.from.1),
            to: (map[&l.to.0], l.to.1),
        })
        .collect();
    g.nodes.extend(new_nodes);
    g.links.extend(new_links);
    all.iter().map(|id| map[id]).collect()
}

pub fn set_enabled(g: &mut Graph, ids: &[NodeId], enabled: bool) {
    for n in &mut g.nodes {
        if ids.contains(&n.id) && !is_io(&n.kind) {
            n.enabled = enabled;
        }
    }
}

/// Position in the body of `frame` of a card whose top is at `y`: the
/// number of body cards above it.
pub fn body_index_for(g: &Graph, frame: NodeId, y: f32) -> usize {
    g.body_ids_of(frame)
        .iter()
        .filter(|b| g.node(**b).is_some_and(|n| n.pos[1] < y))
        .count()
}

pub fn move_into_frame(
    g: &mut Graph,
    node: NodeId,
    frame: NodeId,
    index: usize,
) -> Result<(), SceneError> {
    let n = g
        .node(node)
        .ok_or_else(|| graph_err(syfrack_i18n::tr!("nœud inconnu")))?;
    if !n.kind.is_point_to_point() {
        return Err(graph_err(syfrack_i18n::tr_format!(
            "« {} » n'est pas Point → Point",
            n.kind.label()
        )));
    }
    // A frame can hold neither itself nor a frame that holds it.
    let mut owner = Some(frame);
    let mut guard = 0;
    while let (Some(o), true) = (owner, guard < 64) {
        if o == node {
            return Err(graph_err(syfrack_i18n::tr!(
                "un cadre ne peut pas se contenir"
            )));
        }
        owner = frame_of(g, o);
        guard += 1;
    }
    if !matches!(
        g.node(frame).map(|f| &f.kind),
        Some(NodeKind::Iterate { .. })
    ) {
        return Err(graph_err(syfrack_i18n::tr!(
            "la cible n'est pas un Répéter"
        )));
    }
    let mut copy = g.clone();
    move_out_of_frame(&mut copy, node);
    copy.links.retain(|l| l.from.0 != node && l.to.0 != node);
    if let Some(NodeKind::Iterate { body, .. }) = copy.node_mut(frame).map(|f| &mut f.kind) {
        body.insert(index.min(body.len()), node);
    }
    copy.validate_structure()?;
    *g = copy;
    Ok(())
}

pub fn move_out_of_frame(g: &mut Graph, node: NodeId) {
    for n in &mut g.nodes {
        if let NodeKind::Iterate { body, .. } = &mut n.kind {
            body.retain(|b| *b != node);
        }
    }
}

pub fn insert_subgraph(g: &mut Graph, sub: &Graph, anchor: [f32; 2]) -> Vec<NodeId> {
    let keep: Vec<NodeId> = sub
        .nodes
        .iter()
        .filter(|n| !is_io(&n.kind))
        .map(|n| n.id)
        .collect();
    let map: HashMap<NodeId, NodeId> = keep.iter().copied().zip(g.next_id()..).collect();
    let placed = canvas::auto_layout(sub, &keep, pos2(anchor[0], anchor[1]));
    for id in &keep {
        let mut n = sub.node(*id).expect("kept from the sub-graph").clone();
        n.id = map[id];
        if let Some((_, p)) = placed.iter().find(|(i, _)| i == id) {
            n.pos = [p.x, p.y];
        }
        if let NodeKind::Iterate { body, .. } = &mut n.kind {
            body.retain(|b| map.contains_key(b));
            for b in body.iter_mut() {
                *b = map[b];
            }
        }
        g.nodes.push(n);
    }
    for l in &sub.links {
        if let (Some(a), Some(b)) = (map.get(&l.from.0), map.get(&l.to.0)) {
            g.links.push(Link {
                from: (*a, l.from.1),
                to: (*b, l.to.1),
            });
        }
    }
    keep.iter().map(|id| map[id]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use syfrack_core::{Graph, NodeKind};

    #[test]
    fn connect_replaces_the_existing_link_and_checks_types() {
        let mut g = Graph::new_minimal(); // 0 → 1(Sphere) → 2
        let t = add_node(
            &mut g,
            NodeKind::Translate {
                offset: glam::Vec3::X,
            },
            [100.0, 0.0],
        );
        connect(&mut g, (0, 0), (t, 0)).unwrap();
        connect(&mut g, (t, 0), (1, 0)).unwrap(); // replaces 0 → 1
        assert_eq!(g.input_of(1, 0), Some((t, 0)));
        assert!(connect(&mut g, (0, 0), (2, 0)).is_err()); // Point into Distance
        g.validate().unwrap();
    }

    #[test]
    fn connect_refuses_a_cycle_and_leaves_the_graph_unchanged() {
        let mut g = Graph::new_minimal();
        let a = add_node(&mut g, NodeKind::BoxFold { limit: 1.0 }, [0.0; 2]);
        let b = add_node(&mut g, NodeKind::Scale { factor: 2.0 }, [0.0; 2]);
        connect(&mut g, (0, 0), (a, 0)).unwrap();
        connect(&mut g, (a, 0), (b, 0)).unwrap();
        connect(&mut g, (b, 0), (1, 0)).unwrap();
        let before = g.clone();
        assert!(connect(&mut g, (b, 0), (a, 0)).is_err());
        assert_eq!(g, before);
    }

    #[test]
    fn remove_drops_links_and_body_memberships_but_never_input_or_output() {
        let mut g = Graph::bundled("mandelbox").unwrap();
        remove_nodes(&mut g, &[2, 0, 8]); // box fold (body), input, output
        assert!(g.node(2).is_none());
        assert!(g.node(0).is_some() && g.node(8).is_some());
        let NodeKind::Iterate { body, .. } = &g.node(1).unwrap().kind else {
            panic!()
        };
        assert!(!body.contains(&2));
        g.validate().unwrap();
    }

    #[test]
    fn duplicate_remaps_bodies_and_internal_links() {
        let mut g = Graph::bundled("mandelbox").unwrap();
        let copies = duplicate(&mut g, &[1, 7], [300.0, 0.0]); // iterate + fence
        assert_eq!(copies.len(), 2 + 5); // body copied too
        let it = copies
            .iter()
            .find(|id| matches!(g.node(**id).unwrap().kind, NodeKind::Iterate { .. }))
            .unwrap();
        let NodeKind::Iterate { body, .. } = &g.node(*it).unwrap().kind else {
            panic!()
        };
        assert!(body.iter().all(|b| copies.contains(b)));
        assert_eq!(
            g.links
                .iter()
                .filter(|l| copies.contains(&l.from.0) && copies.contains(&l.to.0))
                .count(),
            1
        ); // iterate → fence
        g.validate().unwrap(); // copies dangle but the graph stays valid
    }

    #[test]
    fn moving_into_and_out_of_a_frame_keeps_the_graph_valid() {
        let mut g = Graph::bundled("mandelbox").unwrap();
        let t = add_node(&mut g, NodeKind::Twist { rate: 0.1 }, [0.0; 2]);
        move_into_frame(&mut g, t, 1, 0).unwrap();
        assert_eq!(g.body_ids_of(1)[0], t);
        assert!(move_into_frame(&mut g, 7, 1, 0).is_err()); // a fence cannot enter a body
        move_out_of_frame(&mut g, t);
        assert!(!g.body_ids_of(1).contains(&t));
        g.validate().unwrap();
    }

    #[test]
    fn insert_subgraph_brings_everything_but_input_and_output() {
        let mut g = Graph::new_minimal();
        let sub = Graph::bundled("menger").unwrap(); // 6 nodes incl. in/out
        let ids = insert_subgraph(&mut g, &sub, [600.0, 0.0]);
        assert_eq!(ids.len(), 4);
        assert_eq!(g.nodes.len(), 3 + 4);
        assert!(g.nodes.iter().filter(|n| n.kind == NodeKind::Input).count() == 1);
        g.validate().unwrap();
        assert!(g
            .links
            .iter()
            .any(|l| ids.contains(&l.from.0) && ids.contains(&l.to.0))); // iterate → box kept
    }

    #[test]
    fn every_offered_kind_has_valid_defaults_and_a_category() {
        let mut g = Graph::new_minimal();
        for k in ALL_KINDS.iter() {
            add_node(&mut g, k.clone(), [0.0; 2]);
            assert!(category(k) < CATEGORIES.len());
            assert_eq!(default_kind(k), Some(k));
        }
        g.validate().unwrap();
        assert!(!ALL_KINDS
            .iter()
            .any(|k| matches!(k, NodeKind::Input | NodeKind::Output)));
    }

    #[test]
    fn body_nodes_take_no_wire_and_a_frame_cannot_hold_itself() {
        let mut g = Graph::bundled("mandelbox").unwrap();
        assert!(connect(&mut g, (0, 0), (2, 0)).is_err());
        assert!(connect(&mut g, (2, 0), (7, 0)).is_err());
        let it = add_node(&mut g, ALL_KINDS[14].clone(), [0.0; 2]);
        move_into_frame(&mut g, it, 1, 1).unwrap();
        assert_eq!(g.body_ids_of(1)[1], it);
        assert!(
            move_into_frame(&mut g, 1, it, 0).is_err(),
            "outer into inner"
        );
        assert!(move_into_frame(&mut g, it, it, 0).is_err());
        g.validate().unwrap();
    }

    #[test]
    fn connect_refuses_a_cycle_among_dangling_nodes() {
        let mut g = Graph::new_minimal();
        let a = add_node(&mut g, NodeKind::Scale { factor: 2.0 }, [0.0; 2]);
        let b = add_node(&mut g, NodeKind::Twist { rate: 0.2 }, [0.0; 2]);
        connect(&mut g, (a, 0), (b, 0)).unwrap();
        assert!(connect(&mut g, (b, 0), (a, 0)).is_err());
        assert!(connect(&mut g, (a, 0), (a, 0)).is_err());
    }

    #[test]
    fn dissolving_a_mid_chain_node_rewires_its_source_to_its_consumers() {
        let mut g = Graph::bundled("mandelbox").unwrap(); // 0 → 1 (Répéter) → 7 → 8
        dissolve_nodes(&mut g, &[1]);
        assert!(g.node(1).is_none());
        assert_eq!(g.input_of(7, 0), Some((0, 0)));
        g.validate().unwrap(); // still complete
        let mut g = Graph::new_minimal();
        let t = add_node(&mut g, NodeKind::Twist { rate: 0.2 }, [0.0; 2]);
        let s = add_node(&mut g, NodeKind::Scale { factor: 2.0 }, [0.0; 2]);
        connect(&mut g, (0, 0), (t, 0)).unwrap();
        connect(&mut g, (t, 0), (s, 0)).unwrap();
        connect(&mut g, (s, 0), (1, 0)).unwrap();
        let fan = add_node(&mut g, NodeKind::Box { half: Vec3::ONE }, [0.0; 2]);
        connect(&mut g, (s, 0), (fan, 0)).unwrap();
        dissolve_nodes(&mut g, &[t, s]); // a chain, with two consumers
        assert_eq!(g.input_of(1, 0), Some((0, 0)));
        assert_eq!(g.input_of(fan, 0), Some((0, 0)));
        g.validate().unwrap();
    }

    #[test]
    fn dissolving_a_node_that_changes_type_leaves_its_consumer_unwired() {
        let mut g = Graph::bundled("mandelbox").unwrap();
        dissolve_nodes(&mut g, &[7, 8, 0]); // fence: Point → Distance
        assert!(g.node(7).is_none() && g.node(8).is_some() && g.node(0).is_some());
        assert_eq!(g.input_of(8, 0), None);
        g.validate_structure().unwrap();
        assert!(g.validate().is_err(), "the Sortie waits for a wire");
    }

    #[test]
    fn a_wired_node_moved_into_a_frame_leaves_its_consumer_unwired() {
        let mut g = Graph::bundled("mandelbox").unwrap();
        let t = add_node(&mut g, NodeKind::Twist { rate: 0.2 }, [0.0; 2]);
        connect(&mut g, (0, 0), (t, 0)).unwrap();
        connect(&mut g, (t, 0), (1, 0)).unwrap();
        move_into_frame(&mut g, t, 1, 0).unwrap();
        assert_eq!(g.input_of(1, 0), None);
        g.validate_structure().unwrap();
    }

    #[test]
    fn default_params_come_from_the_offered_kinds() {
        let k = NodeKind::Scale { factor: -1.0 };
        assert_eq!(default_param(&k, "factor"), Some(2.0));
        let t = NodeKind::Tile { period: Vec3::ONE };
        assert_eq!(default_param(&t, "period.y"), Some(4.0));
        assert_eq!(default_param(&k, "nope"), None);
        assert_eq!(default_param(&NodeKind::Input, "x"), None);
    }

    #[test]
    fn body_index_follows_the_card_height() {
        let g = Graph::bundled("mandelbox").unwrap(); // body rows at y 96..480
        assert_eq!(body_index_for(&g, 1, 0.0), 0);
        assert_eq!(body_index_for(&g, 1, 200.0), 2);
        assert_eq!(body_index_for(&g, 1, 1000.0), 5);
    }
}
