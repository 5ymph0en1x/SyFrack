//! Editing rules use the real graph validator, including polymorphic sockets.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Socket {
    pub node: u32,
    pub port: u32,
    pub output: bool,
}
impl Socket {
    pub fn new(node: u32, port: u32, output: bool) -> Self {
        Self { node, port, output }
    }
    pub fn end(self) -> (u32, u32) {
        (self.node, self.port)
    }
}

pub(super) fn input_types(
    g: &CreationGraph,
    n: &CreationNode,
    types: &BTreeMap<(u32, u32), SocketType>,
) -> Vec<SocketType> {
    use SocketType::*;
    let field = |t| match t {
        FieldType::Scalar => Scalar,
        FieldType::Vector => Vector,
        FieldType::Complex => Complex,
    };
    match &n.kind {
        CreationKind::Form(k) => k
            .inputs()
            .iter()
            .map(|t| match t {
                PortType::Point => Point,
                PortType::Distance => Distance,
            })
            .collect(),
        CreationKind::Pattern { .. } | CreationKind::Texture(_) | CreationKind::Flame(_) => {
            vec![Vector]
        }
        CreationKind::Simulation(_) => vec![Vector, Scalar, Scalar, Scalar],
        CreationKind::SampleState { .. } => vec![State, Vector],
        CreationKind::Palette(_) => vec![Scalar],
        CreationKind::Mix => vec![Vector, Vector, Scalar],
        CreationKind::Output => vec![
            Distance, Vector, Scalar, Scalar, Scalar, Scalar, Scalar, Texture,
        ],
        CreationKind::Field(op) => {
            let first = match g.input(n.id, 0).and_then(|s| types.get(&s)) {
                Some(Vector) => FieldType::Vector,
                Some(Complex) => FieldType::Complex,
                _ => FieldType::Scalar,
            };
            op.signature(first).0.into_iter().map(field).collect()
        }
    }
}
pub(super) fn socket_type(
    g: &CreationGraph,
    s: Socket,
    types: &BTreeMap<(u32, u32), SocketType>,
) -> Option<SocketType> {
    if s.output {
        types.get(&s.end()).copied()
    } else {
        input_types(g, g.node(s.node)?, types)
            .get(s.port as usize)
            .copied()
    }
}

/// Distinct IN/OUT types for a catalogue row, using the same signatures as
/// the canvas. Polymorphic operators show their alternatives until a source
/// or destination provides the type that the new connection will infer.
pub(super) fn library_signature(
    kind: &CreationKind,
    context: Option<SocketType>,
) -> (Vec<SocketType>, Vec<SocketType>) {
    let polymorphic = matches!(
        kind,
        CreationKind::Field(FieldOp::Add | FieldOp::Subtract | FieldOp::Multiply | FieldOp::Scale)
    );
    let variants = if polymorphic {
        match context {
            Some(t @ (SocketType::Scalar | SocketType::Vector | SocketType::Complex)) => vec![t],
            _ => vec![SocketType::Scalar, SocketType::Vector, SocketType::Complex],
        }
    } else {
        vec![SocketType::Scalar]
    };
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    for t in variants {
        let mut probe = CreationGraph::default();
        let id = probe.add(kind.clone(), [0.0; 2]);
        if polymorphic && t != SocketType::Scalar {
            let op = if t == SocketType::Vector {
                FieldOp::Vector {
                    value: [0.0; 3].into(),
                }
            } else {
                FieldOp::Complex { value: [0.0; 2] }
            };
            let source = probe.add(CreationKind::Field(op), [0.0; 2]);
            if probe.connect((source, 0), (id, 0)).is_err() {
                continue;
            }
        }
        let types = probe.types().unwrap_or_default();
        for t in input_types(&probe, probe.node(id).unwrap(), &types) {
            if !inputs.contains(&t) {
                inputs.push(t);
            }
        }
        for p in 0..kind.output_labels().len() as u32 {
            if let Some(&t) = types.get(&(id, p)) {
                if !outputs.contains(&t) {
                    outputs.push(t);
                }
            }
        }
    }
    (inputs, outputs)
}
pub(super) fn connect(g: &mut CreationGraph, a: Socket, b: Socket) -> Result<(), SceneError> {
    let (from, to) = if a.output { (a, b) } else { (b, a) };
    if !from.output || to.output || from.node == to.node {
        return Err(SceneError::Graph(label(
            "Choisissez une entrée et une sortie de deux nœuds différents.",
        )));
    }
    g.connect(from.end(), to.end())
}
pub(super) fn compatible(g: &CreationGraph, a: Socket, b: Socket) -> bool {
    a.output != b.output && a.node != b.node && connect(&mut g.clone(), a, b).is_ok()
}

/// Pointer motion and repaint must not revalidate every possible edge in a
/// large scene. Invalidate on socket contracts, inferred types or connectivity.
pub(super) fn candidates(
    g: &CreationGraph,
    anchor: Socket,
    types: &BTreeMap<(u32, u32), SocketType>,
    editor: &mut Editor,
) -> Vec<Socket> {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    for n in &g.nodes {
        n.id.hash(&mut hash);
        std::mem::discriminant(&n.kind).hash(&mut hash);
        for t in input_types(g, n, types) {
            std::mem::discriminant(&t).hash(&mut hash);
        }
        n.kind.output_labels().len().hash(&mut hash);
        match &n.kind {
            CreationKind::Field(op) => std::mem::discriminant(op).hash(&mut hash),
            CreationKind::SampleState { channel } => channel.hash(&mut hash),
            _ => {}
        }
    }
    for l in &g.links {
        l.from.hash(&mut hash);
        l.to.hash(&mut hash);
    }
    for (s, t) in types {
        s.hash(&mut hash);
        std::mem::discriminant(t).hash(&mut hash);
    }
    let signature = hash.finish();
    if editor.connections.0 != signature {
        editor.connections = (signature, BTreeMap::new());
        editor.addable.clear();
    }
    if let Some(cached) = editor.connections.1.get(&anchor) {
        return cached.clone();
    }
    let mut result = Vec::new();
    for n in &g.nodes {
        let count = if anchor.output {
            n.kind.input_labels().len()
        } else {
            n.kind.output_labels().len()
        };
        for p in 0..count as u32 {
            let s = Socket::new(n.id, p, !anchor.output);
            let (from, to) = if anchor.output {
                (anchor, s)
            } else {
                (s, anchor)
            };
            let polymorphic = to.port == 0
                && g.node(to.node).is_some_and(|n| {
                    matches!(
                        n.kind,
                        CreationKind::Field(
                            FieldOp::Add | FieldOp::Subtract | FieldOp::Multiply | FieldOp::Scale
                        )
                    )
                });
            if !polymorphic && socket_type(g, from, types) != socket_type(g, to, types) {
                continue;
            }
            if compatible(g, anchor, s) {
                result.push(s);
            }
        }
    }
    editor.connections.1.insert(anchor, result.clone());
    result
}
pub(super) fn attachment(g: &CreationGraph, kind: &CreationKind, anchor: Socket) -> Option<u32> {
    let mut trial = g.clone();
    let id = trial.add(kind.clone(), [0.0; 2]);
    let count = if anchor.output {
        kind.input_labels().len()
    } else {
        kind.output_labels().len()
    };
    (0..count as u32).find(|p| compatible(&trial, anchor, Socket::new(id, *p, !anchor.output)))
}

/// Attach a new operation to an explicit socket, or to the selected source.
/// Unary operations are inserted in the chain only when every consumer remains valid.
pub(super) fn attach(g: &mut CreationGraph, id: u32, anchor: Socket, explicit: bool) -> bool {
    let Some(kind) = g.node(id).map(|n| n.kind.clone()) else {
        return false;
    };
    let count = if anchor.output {
        kind.input_labels().len()
    } else {
        kind.output_labels().len()
    };
    let Some(p) =
        (0..count as u32).find(|p| compatible(g, anchor, Socket::new(id, *p, !anchor.output)))
    else {
        return false;
    };
    let consumers: Vec<_> = g
        .links
        .iter()
        .filter(|l| l.from == anchor.end())
        .map(|l| l.to)
        .collect();
    if connect(g, anchor, Socket::new(id, p, !anchor.output)).is_err() {
        return false;
    }
    if !explicit
        && anchor.output
        && kind.input_labels().len() == 1
        && kind.output_labels().len() == 1
    {
        let mut inserted = g.clone();
        if consumers
            .iter()
            .all(|to| inserted.connect((id, 0), *to).is_ok())
        {
            *g = inserted;
        }
    }
    true
}

pub(super) fn active_nodes(g: &CreationGraph) -> BTreeSet<u32> {
    let mut active = g.active_nodes();
    loop {
        let before = active.len();
        for id in active.clone() {
            if let Some(CreationNode {
                kind: CreationKind::Form(NodeKind::Iterate { body, .. }),
                ..
            }) = g.node(id)
            {
                active.extend(body);
            }
        }
        if before == active.len() {
            return active;
        }
    }
}
pub(super) fn branch(g: &CreationGraph, selected: &[u32]) -> BTreeSet<u32> {
    // Walk upstream and downstream separately, so sibling branches stay dim.
    let mut result: BTreeSet<_> = selected.iter().copied().collect();
    for upstream in [true, false] {
        let mut seen: BTreeSet<_> = selected.iter().copied().collect();
        let mut todo = selected.to_vec();
        while let Some(id) = todo.pop() {
            for l in &g.links {
                let (a, b) = if upstream {
                    (l.to.0, l.from.0)
                } else {
                    (l.from.0, l.to.0)
                };
                if a == id && seen.insert(b) {
                    todo.push(b);
                }
            }
            if let Some(CreationNode {
                kind: CreationKind::Form(NodeKind::Iterate { body, .. }),
                ..
            }) = g.node(id)
            {
                for &b in body {
                    if seen.insert(b) {
                        todo.push(b);
                    }
                }
            }
        }
        result.extend(seen);
    }
    result
}

/// Longest-path layers, followed by barycentric sweeps to untangle branches.
/// Card heights determine spacing; loop bodies occupy their own reserved lane.
pub(super) fn arrange(g: &mut CreationGraph) {
    let bodies: BTreeSet<_> = g
        .nodes
        .iter()
        .filter_map(|n| match &n.kind {
            CreationKind::Form(NodeKind::Iterate { body, .. }) => Some(body.iter().copied()),
            _ => None,
        })
        .flatten()
        .collect();
    let mut ranks = BTreeMap::<u32, usize>::new();
    let mut layers = BTreeMap::<usize, Vec<u32>>::new();
    for id in g
        .order()
        .unwrap_or_else(|_| g.nodes.iter().map(|n| n.id).collect())
    {
        if bodies.contains(&id) {
            continue;
        }
        let rank = g
            .links
            .iter()
            .filter(|l| l.to.0 == id)
            .filter_map(|l| ranks.get(&l.from.0).map(|r| r + 1))
            .max()
            .unwrap_or(0);
        ranks.insert(id, rank);
        layers.entry(rank).or_default().push(id);
    }
    let mut positions = BTreeMap::<u32, f32>::new();
    for layer in layers.values() {
        for (i, id) in layer.iter().enumerate() {
            positions.insert(*id, i as f32);
        }
    }
    for sweep in 0..6 {
        let mut keys: Vec<_> = layers.keys().copied().collect();
        let forward = sweep % 2 == 0;
        if !forward {
            keys.reverse();
        }
        for key in keys {
            let layer = layers.get_mut(&key).unwrap();
            let score = |id: &u32| {
                let values: Vec<_> = g
                    .links
                    .iter()
                    .filter_map(|l| {
                        let (a, b) = if forward {
                            (l.to.0, l.from.0)
                        } else {
                            (l.from.0, l.to.0)
                        };
                        (a == *id).then(|| positions.get(&b).copied()).flatten()
                    })
                    .collect();
                if values.is_empty() {
                    positions[id]
                } else {
                    values.iter().sum::<f32>() / values.len() as f32
                }
            };
            layer.sort_by(|a, b| score(a).total_cmp(&score(b)).then(a.cmp(b)));
            for (i, id) in layer.iter().enumerate() {
                positions.insert(*id, i as f32);
            }
        }
    }
    let total = |ids: &[u32]| {
        ids.iter()
            .map(|id| height(g.node(*id).unwrap()) + 40.0)
            .sum::<f32>()
    };
    let tallest = layers.values().map(|ids| total(ids)).fold(0.0, f32::max);
    let sizes: BTreeMap<_, _> = layers.iter().map(|(r, ids)| (*r, total(ids))).collect();
    for (rank, ids) in layers {
        let mut y = 48.0 + (tallest - sizes[&rank]) * 0.5;
        for id in ids {
            let n = g.nodes.iter_mut().find(|n| n.id == id).unwrap();
            n.pos = [48.0 + rank as f32 * (WIDTH + 88.0), y];
            y += height(n) + 40.0;
        }
    }
    let loops: Vec<_> = g
        .nodes
        .iter()
        .filter_map(|n| match &n.kind {
            CreationKind::Form(NodeKind::Iterate { body, .. }) => Some((n.pos, body.clone())),
            _ => None,
        })
        .collect();
    let mut y = tallest + 160.0;
    let mut placed = BTreeSet::new();
    for (pos, body) in loops {
        let mut h: f32 = 0.0;
        for (i, id) in body.iter().enumerate() {
            if placed.insert(*id) {
                if let Some(n) = g.nodes.iter_mut().find(|n| n.id == *id) {
                    n.pos = [pos[0] + i as f32 * (WIDTH + 64.0), y];
                    h = h.max(height(n));
                }
            }
        }
        y += h + 96.0;
    }
}

pub(super) fn dissolve(g: &mut CreationGraph, selected: &[u32]) {
    for &id in selected {
        let Some(n) = g.node(id) else {
            continue;
        };
        if n.kind.input_labels().len() != 1 || n.kind.output_labels().len() != 1 {
            continue;
        }
        let Some(source) = g.input(id, 0) else {
            continue;
        };
        let consumers: Vec<_> = g
            .links
            .iter()
            .filter(|l| l.from.0 == id)
            .map(|l| l.to)
            .collect();
        let mut trial = g.clone();
        // Removing this unary node also removes the type it imposed downstream.
        remove(&mut trial, &mut vec![id]);
        if consumers
            .iter()
            .all(|to| trial.connect(source, *to).is_ok())
        {
            *g = trial;
        }
    }
    remove(g, &mut selected.to_vec());
}

/// Add a complete, editable branch. Existing geometry and source precision stay intact.
pub(super) fn compose(g: &mut CreationGraph, recipe: u8) -> Result<Vec<u32>, SceneError> {
    let mut next = g.clone();
    let out = next
        .nodes
        .iter()
        .find(|n| matches!(n.kind, CreationKind::Output))
        .map(|n| n.id)
        .ok_or_else(|| SceneError::Graph(label("La composition nécessite une sortie Scène.")))?;
    let bottom = next
        .nodes
        .iter()
        .map(|n| n.pos[1] + height(n))
        .fold(0.0, f32::max)
        + 80.0;
    let mut ids = Vec::new();
    let mut add = |g: &mut CreationGraph, kind| {
        let id = g.add(kind, [48.0 + ids.len() as f32 * (WIDTH + 72.0), bottom]);
        ids.push(id);
        id
    };
    let position = add(&mut next, CreationKind::Field(FieldOp::Position));
    match recipe {
        1 => {
            let a = add(
                &mut next,
                CreationKind::Field(FieldOp::PlaneWave {
                    wavevector: [5.0, 1.8, 0.0].into(),
                    frequency: 1.0,
                    phase: 0.0,
                }),
            );
            let b = add(
                &mut next,
                CreationKind::Field(FieldOp::GaussianWave {
                    center: [0.0; 3].into(),
                    width: 2.5,
                    wavevector: [-2.0, 4.0, 0.0].into(),
                    frequency: 1.0,
                }),
            );
            let sum = add(&mut next, CreationKind::Field(FieldOp::Add));
            let intensity = add(&mut next, CreationKind::Field(FieldOp::Intensity));
            let mut ramp = syfrack_core::scene::palette::neon_palette(3);
            ramp.frequency = 1.0;
            let palette = add(&mut next, CreationKind::Palette(ramp));
            for (from, to) in [
                ((position, 0), (a, 0)),
                ((position, 0), (b, 0)),
                ((a, 0), (sum, 0)),
                ((b, 0), (sum, 1)),
                ((sum, 0), (intensity, 0)),
                ((intensity, 0), (palette, 0)),
                ((palette, 0), (out, 1)),
            ] {
                next.connect(from, to)?;
            }
        }
        2 => {
            let flame = add(&mut next, CreationKind::Flame(FlameField::default()));
            next.connect((position, 0), (flame, 0))?;
            next.connect((flame, 0), (out, 1))?;
            next.connect((flame, 1), (out, 3))?;
        }
        _ => {
            let texture = FractalTexture {
                planar: Some(PlanarFractal::default()),
                scale: 1.0,
                octaves: 1,
                ..Default::default()
            };
            let pattern = add(&mut next, CreationKind::Texture(Box::new(texture)));
            let mut ramp = syfrack_core::scene::palette::neon_palette(2);
            ramp.frequency = 4.0;
            let palette = add(&mut next, CreationKind::Palette(ramp));
            next.connect((position, 0), (pattern, 0))?;
            next.connect((pattern, 0), (palette, 0))?;
            next.connect((palette, 0), (out, 1))?;
        }
    }
    next.types()?;
    *g = next;
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_colors_describe_fixed_mixed_and_inferred_socket_types() {
        use SocketType::*;
        assert_eq!(
            library_signature(&CreationKind::Field(FieldOp::Position), None),
            (vec![], vec![Vector])
        );
        assert_eq!(
            library_signature(&CreationKind::Form(NodeKind::Sphere { radius: 1.0 }), None),
            (vec![Point], vec![Distance])
        );
        assert_eq!(
            library_signature(&CreationKind::Palette(Ramp::default()), None),
            (vec![Scalar], vec![Vector])
        );
        assert_eq!(
            library_signature(&CreationKind::Flame(FlameField::default()), None),
            (vec![Vector], vec![Vector, Scalar])
        );
        assert_eq!(
            library_signature(&CreationKind::Mix, None),
            (vec![Vector, Scalar], vec![Vector])
        );
        assert_eq!(
            library_signature(&CreationKind::Field(FieldOp::Add), None),
            (vec![Scalar, Vector, Complex], vec![Scalar, Vector, Complex])
        );
        assert_eq!(
            library_signature(&CreationKind::Field(FieldOp::Add), Some(Vector)),
            (vec![Vector], vec![Vector])
        );
        assert_eq!(
            library_signature(&CreationKind::Field(FieldOp::Scale), Some(Vector)),
            (vec![Vector, Scalar], vec![Vector])
        );
    }

    fn chain() -> (CreationGraph, u32, u32, u32) {
        let mut g = CreationGraph::default();
        let a = g.add(
            CreationKind::Field(FieldOp::Scalar { value: 0.5 }),
            [0.0; 2],
        );
        let b = g.add(CreationKind::Field(FieldOp::Sine), [0.0; 2]);
        let c = g.add(CreationKind::Field(FieldOp::Cosine), [0.0; 2]);
        g.connect((a, 0), (b, 0)).unwrap();
        g.connect((a, 0), (c, 0)).unwrap();
        (g, a, b, c)
    }

    #[test]
    fn unary_insertion_preserves_all_consumers_and_can_be_undone_once() {
        let mut state = AppState::new(Scene::example_2d(), None);
        let mut g = graph(&state);
        let out = g
            .nodes
            .iter()
            .find(|n| matches!(n.kind, CreationKind::Output))
            .unwrap()
            .id;
        let source = g.add(
            CreationKind::Field(FieldOp::Scalar { value: 0.5 }),
            [0.0; 2],
        );
        g.connect((source, 0), (out, 2)).unwrap();
        g.connect((source, 0), (out, 3)).unwrap();
        commit(&mut state, g.clone()).unwrap();
        let before = state.scene.clone();
        let step = g.add(
            CreationKind::Field(FieldOp::Power { exponent: 2.0 }),
            [0.0; 2],
        );
        assert!(attach(&mut g, step, Socket::new(source, 0, true), false));
        assert_eq!(g.input(step, 0), Some((source, 0)));
        assert_eq!(g.input(out, 2), Some((step, 0)));
        assert_eq!(g.input(out, 3), Some((step, 0)));
        commit(&mut state, g).unwrap();
        assert!(state.undo());
        assert_eq!(state.scene, before);
    }

    #[test]
    fn explicit_connection_keeps_existing_branches_and_reverse_wiring_uses_the_right_output() {
        let (mut g, a, b, c) = chain();
        let extra = g.add(CreationKind::Field(FieldOp::Absolute), [0.0; 2]);
        assert!(attach(&mut g, extra, Socket::new(a, 0, true), true));
        assert_eq!(g.input(b, 0), Some((a, 0)));
        assert_eq!(g.input(c, 0), Some((a, 0)));
        let flame = CreationKind::Flame(FlameField::default());
        assert_eq!(attachment(&g, &flame, Socket::new(b, 0, false)), Some(1));
        let id = g.add(flame, [0.0; 2]);
        assert!(attach(&mut g, id, Socket::new(b, 0, false), true));
        assert_eq!(g.input(b, 0), Some((id, 1)), "density, not color");
    }

    #[test]
    fn incompatible_types_and_cycles_leave_every_wire_untouched() {
        let (mut g, a, b, _) = chain();
        let vector = g.add(
            CreationKind::Field(FieldOp::Vector {
                value: [1.0; 3].into(),
            }),
            [0.0; 2],
        );
        let before = g.clone();
        assert!(!compatible(
            &g,
            Socket::new(vector, 0, true),
            Socket::new(b, 0, false)
        ));
        assert!(connect(&mut g, Socket::new(b, 0, true), Socket::new(a, 0, false)).is_err());
        assert_eq!(g, before);
        assert_eq!(
            attachment(
                &g,
                &CreationKind::Palette(Ramp::default()),
                Socket::new(vector, 0, true)
            ),
            None
        );
    }

    #[test]
    fn insertion_infers_polymorphic_vector_inputs() {
        let mut g = CreationGraph::default();
        let vector = g.add(
            CreationKind::Field(FieldOp::Vector {
                value: [1.0; 3].into(),
            }),
            [0.0; 2],
        );
        let op = g.add(CreationKind::Field(FieldOp::Scale), [0.0; 2]);
        assert!(attach(&mut g, op, Socket::new(vector, 0, true), true));
        assert_eq!(g.types().unwrap()[&(op, 0)], SocketType::Vector);
        assert_eq!(
            input_types(&g, g.node(op).unwrap(), &g.types().unwrap()),
            [SocketType::Vector, SocketType::Scalar]
        );
    }

    #[test]
    fn cached_choices_follow_polymorphic_types_and_new_connections() {
        let (mut g, a, b, _) = chain();
        let mut editor = Editor::default();
        let vector = g.add(
            CreationKind::Field(FieldOp::Vector {
                value: [1.0; 3].into(),
            }),
            [0.0; 2],
        );
        let scale = g.add(CreationKind::Field(FieldOp::Scale), [0.0; 2]);
        let source = Socket::new(vector, 0, true);
        let choices = candidates(&g, source, &g.types().unwrap(), &mut editor);
        assert!(choices.contains(&Socket::new(scale, 0, false)));
        assert!(!choices.contains(&Socket::new(b, 0, false)));
        assert_eq!(
            choices,
            candidates(&g, source, &g.types().unwrap(), &mut editor)
        );
        g.connect((vector, 0), (scale, 0)).unwrap();
        let target = Socket::new(scale, 1, false);
        let choices = candidates(&g, target, &g.types().unwrap(), &mut editor);
        assert!(choices.contains(&Socket::new(a, 0, true)));
        assert!(!choices.contains(&source));
        g.nodes.iter_mut().find(|n| n.id == scale).unwrap().kind =
            CreationKind::Field(FieldOp::Add);
        let choices = candidates(&g, target, &g.types().unwrap(), &mut editor);
        assert!(!choices.contains(&Socket::new(a, 0, true)));
        assert!(choices.contains(&source));
    }

    #[test]
    fn dissolve_repairs_unary_chains_but_does_not_convert_vector_to_scalar() {
        let (mut g, a, b, c) = chain();
        let next = g.add(CreationKind::Field(FieldOp::Absolute), [0.0; 2]);
        g.connect((b, 0), (next, 0)).unwrap();
        dissolve(&mut g, &[b]);
        assert_eq!(g.input(next, 0), Some((a, 0)));
        assert_eq!(g.input(c, 0), Some((a, 0)));
        let vector = g.add(
            CreationKind::Field(FieldOp::Vector {
                value: [1.0; 3].into(),
            }),
            [0.0; 2],
        );
        let length = g.add(CreationKind::Field(FieldOp::Length), [0.0; 2]);
        g.connect((vector, 0), (length, 0)).unwrap();
        g.connect((length, 0), (next, 0)).unwrap();
        dissolve(&mut g, &[length]);
        assert_eq!(g.input(next, 0), None);
        g.types().unwrap();
    }

    #[test]
    fn layout_is_deterministic_and_never_changes_render_inputs() {
        let mut state = AppState::new(Scene::example_for("mandelbox").unwrap(), None);
        state.take_preview_dirty();
        let mut g = graph(&state);
        let before = g.clone();
        arrange(&mut g);
        let positioned = g.clone();
        arrange(&mut g);
        assert_eq!(g, positioned);
        assert_eq!(g.links, before.links);
        assert_eq!(g.settings, before.settings);
        for n in &g.nodes {
            let rect = Rect::from_min_size(Pos2::from(n.pos), Vec2::new(WIDTH, height(n)));
            for other in g.nodes.iter().filter(|o| o.id != n.id) {
                assert!(
                    !rect.intersects(Rect::from_min_size(
                        Pos2::from(other.pos),
                        Vec2::new(WIDTH, height(other))
                    )),
                    "{} overlaps {}",
                    n.id,
                    other.id
                );
            }
        }
        commit(&mut state, g).unwrap();
        assert!(
            !state.take_preview_dirty(),
            "organising preserves renderer inputs"
        );
    }

    #[test]
    fn focused_path_keeps_sibling_branches_dim_and_includes_loop_steps() {
        let (mut g, a, b, c) = chain();
        let next = g.add(CreationKind::Field(FieldOp::Absolute), [0.0; 2]);
        g.connect((b, 0), (next, 0)).unwrap();
        let path = branch(&g, &[b]);
        assert!(path.contains(&a) && path.contains(&b) && path.contains(&next));
        assert!(!path.contains(&c));
        let geometry = graph(&AppState::new(
            Scene::example_for("mandelbox").unwrap(),
            None,
        ));
        let active = active_nodes(&geometry);
        for n in &geometry.nodes {
            if let CreationKind::Form(NodeKind::Iterate { body, .. }) = &n.kind {
                assert!(body.iter().all(|id| active.contains(id)));
            }
        }
    }

    #[test]
    fn complete_recipes_compile_in_both_dimensions_and_preserve_existing_geometry() {
        for base in [
            Scene::example_2d(),
            Scene::example_for("mandelbox").unwrap(),
        ] {
            for recipe in 0..3 {
                let mut state = AppState::new(base.clone(), None);
                let original = state.scene.clone();
                let mut g = graph(&state);
                let old = g.clone();
                let out = g
                    .nodes
                    .iter()
                    .find(|n| matches!(n.kind, CreationKind::Output))
                    .unwrap()
                    .id;
                let geometry = g.input(out, 0);
                let added = compose(&mut g, recipe).unwrap();
                assert_eq!(g.input(out, 0), geometry);
                for n in &old.nodes {
                    assert_eq!(g.node(n.id), Some(n));
                }
                assert!(added.iter().all(|id| active_nodes(&g).contains(id)));
                commit(&mut state, g).unwrap();
                assert!(state.scene.creation_draft.is_none());
                assert!(state.undo());
                assert_eq!(state.scene, original);
            }
        }
    }
}
