//! Pure geometry of the node canvas: cards, ports, wires, frames, hit
//! testing, lasso and automatic placement. egui only draws (ui/form_panel.rs).

use crate::ui::theme;
use egui::{pos2, vec2, Color32, Pos2, Rect, Vec2};
use syfrack_core::{Graph, Node, NodeId, NodeKind};

pub const CARD_W: f32 = 192.0;
pub const HEADER_H: f32 = 24.0;
pub const ROW_H: f32 = 24.0;
pub const PORT_R: f32 = 4.0;
pub const PORT_HIT: f32 = 10.0;
pub const FRAME_PAD: f32 = 16.0;
pub const INLINE_PARAMS: usize = 3;
pub const WIRE_HIT: f32 = 6.0;
pub const WIRE_POINT: Color32 = theme::ACCENT;
pub const WIRE_DIST: Color32 = Color32::from_rgb(0x6f, 0x8e, 0xa8);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasView {
    pub offset: Vec2,
    pub zoom: f32,
}

impl Default for CanvasView {
    fn default() -> Self {
        Self {
            offset: vec2(40.0, 40.0),
            zoom: 1.0,
        }
    }
}

impl CanvasView {
    pub fn to_screen(&self, rect_min: Pos2, w: Pos2) -> Pos2 {
        rect_min + (w.to_vec2() + self.offset) * self.zoom
    }
    pub fn to_world(&self, rect_min: Pos2, s: Pos2) -> Pos2 {
        ((s - rect_min) / self.zoom - self.offset).to_pos2()
    }
    pub fn zoom_at(&mut self, rect_min: Pos2, cursor: Pos2, factor: f32) {
        let before = self.to_world(rect_min, cursor);
        self.zoom = (self.zoom * factor).clamp(0.25, 3.0);
        let after = self.to_world(rect_min, cursor);
        self.offset += after - before;
    }
}

pub fn param_count(kind: &NodeKind) -> usize {
    use NodeKind::*;
    match kind {
        Input
        | Output
        | AddOrigin
        | BulbFence
        | Union
        | Intersection
        | Subtraction
        | Abs { .. } => 0,
        Translate { .. }
        | Rotate { .. }
        | Scale { .. }
        | ScaleAxes { .. }
        | AddSeed { .. }
        | MaterialTag { .. }
        | BoxFold { .. }
        | BoxFoldAxes { .. }
        | Power { .. }
        | Twist { .. }
        | Bend { .. }
        | Tile { .. }
        | Sphere { .. }
        | Box { .. }
        | NormFence { .. }
        | SmoothUnion { .. }
        | SmoothIntersection { .. } => 1,
        Mirror { .. }
        | RepeatFinite { .. }
        | Torus { .. }
        | Cylinder { .. }
        | Bounded { .. }
        | Instances { .. }
        | RoundBox { .. }
        | SphereFold { .. }
        | SphereInversion { .. }
        | AddConstant { .. }
        | MengerFold { .. }
        | Kaleido { .. }
        | Iterate { .. }
        | Plane { .. }
        | Trap { .. } => 2,
        Rock { .. } => 3,
    }
}
pub fn inline_rows(kind: &NodeKind) -> usize {
    param_count(kind).min(INLINE_PARAMS)
}
pub fn card_size(node: &Node) -> Vec2 {
    vec2(
        CARD_W,
        HEADER_H + inline_rows(&node.kind) as f32 * ROW_H + 8.0,
    )
}
pub fn card_rect(node: &Node) -> Rect {
    Rect::from_min_size(pos2(node.pos[0], node.pos[1]), card_size(node))
}

pub fn frame_rect(g: &Graph, iterate: &Node) -> Rect {
    let mut r = card_rect(iterate);
    let NodeKind::Iterate { body, .. } = &iterate.kind else {
        return r;
    };
    for b in body {
        if let Some(n) = g.node(*b) {
            let cr = if matches!(n.kind, NodeKind::Iterate { .. }) {
                frame_rect(g, n)
            } else {
                card_rect(n)
            };
            r = r.union(cr);
        }
    }
    if body.is_empty() {
        r.max.y += ROW_H * 2.0;
    }
    Rect::from_min_max(
        r.min - vec2(FRAME_PAD, 0.0),
        r.max + vec2(FRAME_PAD, FRAME_PAD),
    )
}

fn port_y(node: &Node, port: usize) -> f32 {
    node.pos[1] + HEADER_H * 0.5 + port as f32 * 12.0
}
pub fn in_port_pos(node: &Node, port: usize) -> Pos2 {
    pos2(node.pos[0], port_y(node, port))
}
pub fn out_port_pos(node: &Node, port: usize) -> Pos2 {
    pos2(node.pos[0] + CARD_W, port_y(node, port))
}

pub fn bezier(from: Pos2, to: Pos2) -> [Pos2; 4] {
    let h = ((to.x - from.x).abs() * 0.5).clamp(24.0, 200.0);
    [from, pos2(from.x + h, from.y), pos2(to.x - h, to.y), to]
}
pub fn bezier_point(b: &[Pos2; 4], t: f32) -> Pos2 {
    let u = 1.0 - t;
    let v = b[0].to_vec2() * (u * u * u)
        + b[1].to_vec2() * (3.0 * u * u * t)
        + b[2].to_vec2() * (3.0 * u * t * t)
        + b[3].to_vec2() * (t * t * t);
    v.to_pos2()
}
/// Distance from `p` to the curve: the nearest of 24 samples, refined by a
/// ternary search on the two segments around it (exact on the curve itself).
pub fn wire_distance(b: &[Pos2; 4], p: Pos2) -> f32 {
    const SAMPLES: usize = 24;
    let d = |t: f32| (bezier_point(b, t) - p).length();
    let best = (0..=SAMPLES)
        .map(|i| i as f32 / SAMPLES as f32)
        .min_by(|x, y| d(*x).total_cmp(&d(*y)))
        .unwrap_or(0.0);
    let step = 1.0 / SAMPLES as f32;
    let (mut lo, mut hi) = ((best - step).max(0.0), (best + step).min(1.0));
    for _ in 0..32 {
        let m1 = lo + (hi - lo) / 3.0;
        let m2 = hi - (hi - lo) / 3.0;
        if d(m1) < d(m2) {
            hi = m2;
        } else {
            lo = m1;
        }
    }
    d(0.5 * (lo + hi)).min(d(best))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Card(NodeId),
    InPort(NodeId, usize),
    OutPort(NodeId, usize),
    Frame(NodeId),
    Wire(usize),
    Empty,
}

/// A body card carries no wire, so it shows no port.
pub fn has_ports(g: &Graph, id: NodeId) -> bool {
    !g.nodes.iter().any(|n| g.body_ids_of(n.id).contains(&id))
}

pub fn hit_test(g: &Graph, w: Pos2) -> Hit {
    for n in g.nodes.iter().rev().filter(|n| has_ports(g, n.id)) {
        for (i, _) in n.kind.inputs().iter().enumerate() {
            if (in_port_pos(n, i) - w).length() <= PORT_HIT {
                return Hit::InPort(n.id, i);
            }
        }
        for (i, _) in n.kind.outputs().iter().enumerate() {
            if (out_port_pos(n, i) - w).length() <= PORT_HIT {
                return Hit::OutPort(n.id, i);
            }
        }
    }
    for n in g.nodes.iter().rev() {
        if card_rect(n).contains(w) {
            return Hit::Card(n.id);
        }
    }
    // The header band of a frame (the strip of its Répéter card) keeps
    // priority over wires; the rest of the frame comes after them.
    let deepest = |band_only: bool| {
        g.nodes
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Iterate { .. }))
            .map(|n| {
                let mut r = frame_rect(g, n);
                if band_only {
                    r.max.y = r.min.y + HEADER_H;
                }
                (n.id, r)
            })
            .filter(|(_, r)| r.contains(w))
            .min_by(|a, b| a.1.area().total_cmp(&b.1.area()))
            .map(|(id, _)| id)
    };
    if let Some(id) = deepest(true) {
        return Hit::Frame(id);
    }
    for (i, l) in g.links.iter().enumerate() {
        let (Some(a), Some(b)) = (g.node(l.from.0), g.node(l.to.0)) else {
            continue;
        };
        let bz = bezier(
            out_port_pos(a, l.from.1 as usize),
            in_port_pos(b, l.to.1 as usize),
        );
        if wire_distance(&bz, w) <= WIRE_HIT {
            return Hit::Wire(i);
        }
    }
    if let Some(id) = deepest(false) {
        return Hit::Frame(id);
    }
    Hit::Empty
}

pub fn lasso_select(g: &Graph, rect: Rect) -> Vec<NodeId> {
    g.nodes
        .iter()
        .filter(|n| rect.contains_rect(card_rect(n)))
        .map(|n| n.id)
        .collect()
}

pub fn auto_layout(g: &Graph, ids: &[NodeId], anchor: Pos2) -> Vec<(NodeId, Pos2)> {
    // Column = longest path from a node without inputs among `ids`; rows stack per column.
    let mut depth: std::collections::HashMap<NodeId, usize> = std::collections::HashMap::new();
    fn depth_of(
        g: &Graph,
        id: NodeId,
        ids: &[NodeId],
        memo: &mut std::collections::HashMap<NodeId, usize>,
        guard: usize,
    ) -> usize {
        if let Some(d) = memo.get(&id) {
            return *d;
        }
        if guard > 256 {
            return 0;
        }
        let d = g
            .links
            .iter()
            .filter(|l| l.to.0 == id && ids.contains(&l.from.0))
            .map(|l| 1 + depth_of(g, l.from.0, ids, memo, guard + 1))
            .max()
            .unwrap_or(0);
        memo.insert(id, d);
        d
    }
    let mut owners = std::collections::HashSet::new();
    for id in ids {
        for b in g.body_ids_of(*id) {
            owners.insert(*b);
        }
    }
    let mut per_col: std::collections::BTreeMap<usize, Vec<NodeId>> =
        std::collections::BTreeMap::new();
    for id in ids {
        if owners.contains(id) {
            continue;
        }
        per_col
            .entry(depth_of(g, *id, ids, &mut depth, 0))
            .or_default()
            .push(*id);
    }
    let mut out = Vec::new();
    for (col, nodes) in per_col {
        let mut y = anchor.y;
        for id in nodes {
            let Some(n) = g.node(id) else { continue };
            out.push((id, pos2(anchor.x + col as f32 * 240.0, y)));
            let h = card_size(n).y;
            // Body cards stack under their Iterate, nested bodies included.
            let x = anchor.x + col as f32 * 240.0 + FRAME_PAD;
            let mut by = stack_body(g, id, x, y + h + 8.0, &mut out, 0);
            if matches!(n.kind, NodeKind::Iterate { .. }) && g.body_ids_of(id).is_empty() {
                by += ROW_H * 2.0; // an empty frame keeps a drop zone
            }
            y += (by - y).max(h) + 16.0 + FRAME_PAD;
        }
    }
    out
}

/// Stacks the body of the Répéter `frame` under its card, in body order,
/// nested bodies included, so cards never overlap and their order on screen
/// is the order they run in.
pub fn restack_body(g: &mut Graph, frame: NodeId) {
    let Some(n) = g.node(frame) else { return };
    let x = n.pos[0] + FRAME_PAD;
    let top = n.pos[1] + card_size(n).y + 8.0;
    let mut out = Vec::new();
    stack_body(g, frame, x, top, &mut out, 0);
    for (id, p) in out {
        if let Some(b) = g.node_mut(id) {
            b.pos = [p.x, p.y];
        }
    }
}

/// Places the body of `id` from `top` down at `x`, a nested Répéter's body
/// under it, and returns the y below the last card (and nested frame).
fn stack_body(
    g: &Graph,
    id: NodeId,
    x: f32,
    top: f32,
    out: &mut Vec<(NodeId, Pos2)>,
    depth: u32,
) -> f32 {
    let mut by = top;
    if depth > 8 {
        return by;
    }
    for b in g.body_ids_of(id) {
        let Some(bn) = g.node(*b) else { continue };
        out.push((*b, pos2(x, by)));
        by += card_size(bn).y + 8.0;
        if matches!(bn.kind, NodeKind::Iterate { .. }) {
            by = stack_body(g, *b, x, by, out, depth + 1);
            if g.body_ids_of(*b).is_empty() {
                by += ROW_H * 2.0;
            }
            by += FRAME_PAD;
        }
    }
    by
}

pub fn drop_target_frame(g: &Graph, card: Rect) -> Option<NodeId> {
    drop_target_frame_excluding(g, card, &[])
}

/// The deepest frame holding the centre of `card`, `exclude` left out (a
/// dragged Répéter and the frames inside it cannot take it).
pub fn drop_target_frame_excluding(g: &Graph, card: Rect, exclude: &[NodeId]) -> Option<NodeId> {
    let c = card.center();
    g.nodes
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Iterate { .. }) && !exclude.contains(&n.id))
        .map(|n| (n.id, frame_rect(g, n)))
        .filter(|(_, r)| r.contains(c))
        .min_by(|a, b| a.1.area().total_cmp(&b.1.area()))
        .map(|(id, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2, Rect};
    use syfrack_core::{Graph, Link, Node, NodeKind};

    fn node(id: u32, kind: NodeKind, x: f32, y: f32) -> Node {
        Node {
            id,
            name: kind.label().into(),
            kind,
            pos: [x, y],
            enabled: true,
        }
    }

    #[test]
    fn view_round_trips_and_zooms_around_the_cursor() {
        let mut v = CanvasView {
            offset: vec2(10.0, 20.0),
            zoom: 1.0,
        };
        let min = pos2(100.0, 50.0);
        let w = pos2(300.0, -40.0);
        let s = v.to_screen(min, w);
        assert_eq!(v.to_world(min, s), w);
        let cursor = pos2(400.0, 200.0);
        let before = v.to_world(min, cursor);
        v.zoom_at(min, cursor, 2.0);
        assert!((v.zoom - 2.0).abs() < 1e-6);
        assert!((v.to_world(min, cursor) - before).length() < 1e-3);
        v.zoom_at(min, cursor, 100.0);
        assert_eq!(v.zoom, 3.0);
    }

    #[test]
    fn card_size_follows_the_inline_parameter_count() {
        let a = node(1, NodeKind::AddOrigin, 0.0, 0.0);
        let b = node(
            2,
            NodeKind::SphereFold {
                min_radius: 0.5,
                fixed_radius: 1.0,
            },
            0.0,
            0.0,
        );
        let c = node(
            3,
            NodeKind::Mirror {
                normal: glam::Vec3::Y,
                offset: 0.0,
            },
            0.0,
            0.0,
        );
        assert_eq!(card_size(&a).y, HEADER_H + 8.0);
        assert_eq!(card_size(&b).y, HEADER_H + 2.0 * ROW_H + 8.0);
        assert_eq!(inline_rows(&c.kind), 2); // normal (vec3 row) + offset
        assert_eq!(card_size(&a).x, CARD_W);
    }

    #[test]
    fn ports_sit_on_the_card_edges() {
        let n = node(1, NodeKind::Union, 100.0, 40.0);
        let r = card_rect(&n);
        assert_eq!(in_port_pos(&n, 0).x, r.min.x);
        assert_eq!(out_port_pos(&n, 0).x, r.max.x);
        assert!(in_port_pos(&n, 1).y > in_port_pos(&n, 0).y);
    }

    #[test]
    fn body_cards_have_no_ports_to_hit() {
        let g = Graph::bundled("mandelbox").unwrap();
        let b = g.node(2).unwrap(); // Pli boîte, in the body
        assert!(has_ports(&g, 1) && !has_ports(&g, 2));
        // Its in-port sits on the frame edge: the press lands on the frame
        // or the card, never on a port.
        assert!(!matches!(
            hit_test(&g, in_port_pos(b, 0)),
            Hit::InPort(..) | Hit::OutPort(..)
        ));
        assert!(!matches!(
            hit_test(&g, out_port_pos(b, 0)),
            Hit::InPort(..) | Hit::OutPort(..)
        ));
    }

    #[test]
    fn restacking_a_body_follows_its_order_without_overlaps() {
        let mut g = Graph::bundled("mandelbox").unwrap();
        // Scramble the body cards.
        for (k, id) in [2u32, 3, 4, 5, 6].iter().enumerate() {
            g.node_mut(*id).unwrap().pos = [500.0 + k as f32, 900.0 - 50.0 * k as f32];
        }
        restack_body(&mut g, 1);
        let body = g.body_ids_of(1).to_vec();
        let rects: Vec<Rect> = body
            .iter()
            .map(|b| card_rect(g.node(*b).unwrap()))
            .collect();
        for w in rects.windows(2) {
            assert!(w[0].max.y <= w[1].min.y, "in body order, no overlap");
        }
        let frame = frame_rect(&g, g.node(1).unwrap());
        assert!(rects.iter().all(|r| frame.contains_rect(*r)));
        assert_eq!(rects[0].min.x, g.node(1).unwrap().pos[0] + FRAME_PAD);
    }

    #[test]
    fn hit_test_prefers_ports_then_cards_then_wires_then_frames() {
        let mut g = Graph::bundled("mandelbox").unwrap();
        // legacy layout: input at (0,0), iterate at (240,0) with body cards below it
        let it = g.node(1).unwrap().clone();
        let frame = frame_rect(&g, &it);
        assert!(frame.contains(pos2(
            card_rect(g.node(2).unwrap()).center().x,
            card_rect(g.node(2).unwrap()).center().y
        )));
        assert_eq!(
            hit_test(&g, out_port_pos(g.node(0).unwrap(), 0)),
            Hit::OutPort(0, 0)
        );
        assert_eq!(
            hit_test(&g, card_rect(g.node(0).unwrap()).center()),
            Hit::Card(0)
        );
        assert_eq!(
            hit_test(&g, card_rect(g.node(2).unwrap()).center()),
            Hit::Card(2)
        );
        let gap = pos2(frame.min.x + 2.0, frame.max.y - 2.0);
        assert_eq!(hit_test(&g, gap), Hit::Frame(1));
        let a = out_port_pos(g.node(7).unwrap(), 0);
        let b = in_port_pos(g.node(8).unwrap(), 0);
        let mid = bezier_point(&bezier(a, b), 0.5);
        assert!(matches!(hit_test(&g, mid), Hit::Wire(_)));
        assert_eq!(hit_test(&g, pos2(-500.0, -500.0)), Hit::Empty);
        g.nodes.clear();
        assert_eq!(hit_test(&g, pos2(0.0, 0.0)), Hit::Empty);
    }

    #[test]
    fn bezier_has_horizontal_tangents_and_bounded_handles() {
        let b = bezier(pos2(0.0, 0.0), pos2(1000.0, 100.0));
        assert_eq!(b[1].y, 0.0);
        assert_eq!(b[2].y, 100.0);
        assert_eq!(b[1].x, 200.0);
        let short = bezier(pos2(0.0, 0.0), pos2(10.0, 0.0));
        assert_eq!(short[1].x, 24.0);
        assert!((wire_distance(&b, bezier_point(&b, 0.3))).abs() < 1e-3);
    }

    #[test]
    fn lasso_selects_fully_contained_cards() {
        let g = Graph::bundled("menger").unwrap();
        let r0 = card_rect(g.node(0).unwrap());
        let sel = lasso_select(
            &g,
            Rect::from_min_max(r0.min - vec2(1.0, 1.0), r0.max + vec2(1.0, 1.0)),
        );
        assert_eq!(sel, vec![0]);
        let half = lasso_select(&g, Rect::from_min_max(r0.min, r0.center()));
        assert!(half.is_empty());
    }

    #[test]
    fn auto_layout_places_dependencies_left_of_dependants_without_overlap() {
        let g = Graph::bundled("mandelbulb").unwrap();
        let ids: Vec<_> = g.nodes.iter().map(|n| n.id).collect();
        let placed = auto_layout(&g, &ids, pos2(1000.0, 0.0));
        let pos = |id: u32| placed.iter().find(|(i, _)| *i == id).unwrap().1;
        assert!(pos(0).x < pos(1).x && pos(1).x < pos(5).x && pos(5).x < pos(6).x);
        assert!(pos(0).x >= 1000.0);
        for (i, (ia, pa)) in placed.iter().enumerate() {
            for (ib, pb) in placed.iter().skip(i + 1) {
                let ra = Rect::from_min_size(*pa, card_size(g.node(*ia).unwrap()));
                let rb = Rect::from_min_size(*pb, card_size(g.node(*ib).unwrap()));
                assert!(!ra.intersects(rb), "{ia} and {ib} overlap");
            }
        }
    }

    #[test]
    fn drop_target_is_the_deepest_frame_containing_the_card_centre() {
        let g = Graph::bundled("mandelbox").unwrap();
        let inside = card_rect(g.node(3).unwrap());
        assert_eq!(drop_target_frame(&g, inside), Some(1));
        let outside = Rect::from_min_size(pos2(-400.0, -400.0), vec2(CARD_W, 40.0));
        assert_eq!(drop_target_frame(&g, outside), None);
    }

    #[test]
    fn a_wire_across_a_frame_is_hit_but_the_frame_header_keeps_priority() {
        let mut g = Graph::bundled("mandelbox").unwrap();
        // Clôture norme far left, Sortie far right: their wire crosses the
        // Répéter frame at y = 412, in the left padding beside « Échelle ».
        g.node_mut(7).unwrap().pos = [-300.0, 400.0];
        g.node_mut(8).unwrap().pos = [700.0, 400.0];
        let link = g
            .links
            .iter()
            .position(|l| l.from.0 == 7 && l.to.0 == 8)
            .unwrap();
        let it = g.node(1).unwrap().clone();
        assert!(frame_rect(&g, &it).contains(pos2(230.0, 412.0)));
        assert_eq!(hit_test(&g, pos2(230.0, 412.0)), Hit::Wire(link));
        // The Entrée → Répéter wire runs through the frame header band.
        assert_eq!(hit_test(&g, pos2(226.0, 12.0)), Hit::Frame(1));
    }

    /// Input → Répéter(Répéter(Pli boîte, Échelle), Ajouter l'origine) →
    /// Clôture bulbe → Sortie, every card at the origin.
    fn nested_graph() -> Graph {
        let it = |iterations, body| NodeKind::Iterate {
            iterations,
            bailout: 4.0,
            body,
        };
        let link = |a, b| Link {
            from: (a, 0),
            to: (b, 0),
        };
        Graph {
            nodes: vec![
                node(0, NodeKind::Input, 0.0, 0.0),
                node(1, it(4, vec![2, 3]), 0.0, 0.0),
                node(2, it(3, vec![4, 5]), 0.0, 0.0),
                node(4, NodeKind::BoxFold { limit: 1.0 }, 0.0, 0.0),
                node(5, NodeKind::Scale { factor: 2.0 }, 0.0, 0.0),
                node(3, NodeKind::AddOrigin, 0.0, 0.0),
                node(6, NodeKind::BulbFence, 0.0, 0.0),
                node(7, NodeKind::Output, 0.0, 0.0),
            ],
            links: vec![link(0, 1), link(1, 6), link(6, 7)],
            safety: 0.7,
            bounding_radius_hint: 2.0,
        }
    }

    #[test]
    fn auto_layout_stacks_nested_bodies_inside_their_frames() {
        let g = nested_graph();
        let ids: Vec<_> = g.nodes.iter().map(|n| n.id).collect();
        let placed = auto_layout(&g, &ids, pos2(0.0, 0.0));
        assert_eq!(placed.len(), ids.len(), "every card is placed once");
        let mut laid = g.clone();
        for (id, p) in &placed {
            laid.node_mut(*id).unwrap().pos = [p.x, p.y];
        }
        for (i, a) in laid.nodes.iter().enumerate() {
            for b in laid.nodes.iter().skip(i + 1) {
                assert!(
                    !card_rect(a).intersects(card_rect(b)),
                    "{} and {} overlap",
                    a.id,
                    b.id
                );
            }
        }
        let inner = frame_rect(&laid, laid.node(2).unwrap());
        let outer = frame_rect(&laid, laid.node(1).unwrap());
        assert!(outer.contains_rect(inner));
        assert!(
            !inner.intersects(card_rect(laid.node(3).unwrap())),
            "the card after a nested Répéter sits below its frame"
        );
    }
}
