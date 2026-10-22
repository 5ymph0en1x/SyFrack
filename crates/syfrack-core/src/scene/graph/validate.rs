use super::{Graph, NodeId, NodeKind, MAX_DEPTH, MAX_ITER_PRODUCT, MAX_NODES, MAX_SCALARS};
use crate::params::validate_params;
use crate::scene::SceneError;
use std::collections::{HashMap, HashSet};

fn err(msg: impl Into<String>) -> SceneError {
    SceneError::Graph(msg.into())
}

impl Graph {
    /// Every node id that lies in some Iterate body (recursively), with its owner.
    fn body_owners(&self) -> HashMap<NodeId, NodeId> {
        let mut owners = HashMap::new();
        for n in &self.nodes {
            if let NodeKind::Iterate { body, .. } = &n.kind {
                for b in body {
                    owners.insert(*b, n.id);
                }
            }
        }
        owners
    }

    pub fn active_nodes(&self) -> Vec<NodeId> {
        let Some(out) = self.nodes.iter().find(|n| n.kind == NodeKind::Output) else {
            return Vec::new();
        };
        let mut seen = HashSet::new();
        let mut stack = vec![out.id];
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            for l in self.links.iter().filter(|l| l.to.0 == id) {
                stack.push(l.from.0);
            }
        }
        let mut v: Vec<NodeId> = seen.into_iter().collect();
        v.sort_unstable();
        v
    }

    /// Active nodes, dependencies first. Body nodes are not listed (their
    /// Iterate is). Errors on a cycle.
    pub fn topo_order(&self) -> Result<Vec<NodeId>, SceneError> {
        let active: HashSet<NodeId> = self.active_nodes().into_iter().collect();
        let owners = self.body_owners();
        let mut indeg: HashMap<NodeId, usize> = active
            .iter()
            .filter(|id| !owners.contains_key(id))
            .map(|id| (*id, 0))
            .collect();
        for l in &self.links {
            if indeg.contains_key(&l.to.0) && active.contains(&l.from.0) {
                *indeg.get_mut(&l.to.0).unwrap() += 1;
            }
        }
        let mut ready: Vec<NodeId> = indeg
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(id, _)| *id)
            .collect();
        ready.sort_unstable_by(|a, b| b.cmp(a));
        let mut order = Vec::new();
        while let Some(id) = ready.pop() {
            order.push(id);
            let mut next: Vec<NodeId> = Vec::new();
            for l in self.links.iter().filter(|l| l.from.0 == id) {
                if let Some(d) = indeg.get_mut(&l.to.0) {
                    *d -= 1;
                    if *d == 0 {
                        next.push(l.to.0);
                    }
                }
            }
            next.sort_unstable_by(|a, b| b.cmp(a));
            ready.extend(next);
            ready.sort_unstable_by(|a, b| b.cmp(a));
        }
        if order.len() != indeg.len() {
            return Err(err(syfrack_i18n::tr!("cycle dans le graphe")));
        }
        Ok(order)
    }

    pub fn iterate_depth(&self) -> u32 {
        self.depth_of_all()
    }

    fn depth_of_all(&self) -> u32 {
        fn depth(g: &Graph, id: NodeId, seen: &mut Vec<NodeId>) -> u32 {
            if seen.contains(&id) {
                return 99;
            }
            seen.push(id);
            let d = match g.node(id).map(|n| &n.kind) {
                Some(NodeKind::Iterate { body, .. }) => {
                    1 + body.iter().map(|b| depth(g, *b, seen)).max().unwrap_or(0)
                }
                _ => 0,
            };
            seen.pop();
            d
        }
        self.nodes
            .iter()
            .map(|n| depth(self, n.id, &mut Vec::new()))
            .max()
            .unwrap_or(0)
    }

    pub fn iteration_product(&self) -> u32 {
        fn product(g: &Graph, id: NodeId, seen: &mut Vec<NodeId>) -> u32 {
            if seen.contains(&id) {
                return u32::MAX;
            }
            seen.push(id);
            let p = match g.node(id).map(|n| &n.kind) {
                Some(NodeKind::Iterate {
                    iterations, body, ..
                }) => iterations
                    .saturating_mul(body.iter().map(|b| product(g, *b, seen)).max().unwrap_or(1)),
                _ => 1,
            };
            seen.pop();
            p
        }
        // Active nodes only, like the fingerprint: a dangling Repeat neither
        // counts toward the limit nor changes the shader's clamp.
        let owners = self.body_owners();
        self.active_nodes()
            .into_iter()
            .filter(|id| !owners.contains_key(id))
            .map(|id| product(self, id, &mut Vec::new()))
            .max()
            .unwrap_or(1)
    }

    /// Everything an editor must keep true, even mid-edit: ids, one Entrée
    /// and one Sortie, bounds, port types, one wire per input, body rules,
    /// limits, and no cycle anywhere (dangling nodes included). Unwired
    /// ports are allowed: see `validate`.
    pub fn validate_structure(&self) -> Result<(), SceneError> {
        if self.nodes.len() > MAX_NODES {
            return Err(err(syfrack_i18n::tr_format!(
                "plus de {MAX_NODES} nœuds",
                MAX_NODES = MAX_NODES
            )));
        }
        let mut ids = HashSet::new();
        for n in &self.nodes {
            if !ids.insert(n.id) {
                return Err(err(syfrack_i18n::tr_format!(
                    "identifiant de nœud dupliqué {}",
                    n.id
                )));
            }
        }
        let inputs = self
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Input)
            .count();
        let outputs = self
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Output)
            .count();
        if inputs != 1 {
            return Err(err(syfrack_i18n::tr!(
                "le graphe doit avoir exactement une Entrée"
            )));
        }
        if outputs != 1 {
            return Err(err(syfrack_i18n::tr!(
                "le graphe doit avoir exactement une Sortie"
            )));
        }
        validate_params(self, "fractal").map_err(|e| {
            let text = e.to_string();
            match node_in_path(&text) {
                Some(id) => node_err(id, text),
                None => err(text),
            }
        })?;
        for n in &self.nodes {
            if let NodeKind::Scale { factor } = n.kind {
                if factor.abs() < 1e-3 {
                    return Err(node_err(
                        n.id,
                        syfrack_i18n::tr_format!(
                            "nodes[{}].factor : l'échelle ne peut pas être nulle",
                            n.id
                        ),
                    ));
                }
            }
            if let NodeKind::Instances { placements, .. } = &n.kind {
                if placements.is_empty() || placements.len() > super::MAX_PLACEMENTS {
                    return Err(node_err(
                        n.id,
                        syfrack_i18n::tr_format!(
                            "Copies : entre 1 et {} placements",
                            super::MAX_PLACEMENTS
                        ),
                    ));
                }
            }
        }
        // Each placement uploads three inverse affine rows (12 scalars)
        // instead of the nine editable transform values. Four slots are
        // reserved for trap normalisation and one for the root union loop by the renderer.
        let affine_extra: usize = self
            .nodes
            .iter()
            .map(|n| match &n.kind {
                NodeKind::Instances { placements, .. } => {
                    3 * placements.len() + 10 + (2 * placements.len() - 1) * 8
                }
                NodeKind::Iterate { .. } => 1,
                _ => 0,
            })
            .sum();
        if self.scalar_count() + affine_extra + 5 > MAX_SCALARS {
            return Err(err(syfrack_i18n::tr_format!(
                "plus de {MAX_SCALARS} paramètres",
                MAX_SCALARS = MAX_SCALARS
            )));
        }
        // A node belongs to at most one body, once: otherwise its frame is
        // ambiguous (`body_owners` keeps one owner).
        let mut member_of: HashMap<NodeId, NodeId> = HashMap::new();
        for n in &self.nodes {
            if let NodeKind::Iterate { body, .. } = &n.kind {
                for b in body {
                    if let Some(first) = member_of.insert(*b, n.id) {
                        let msg = if first == n.id {
                            syfrack_i18n::tr_format!(
                                "le nœud {b} figure deux fois dans le corps de Répéter {first}",
                                b = b,
                                first = first
                            )
                        } else {
                            syfrack_i18n::tr_format!(
                                "le nœud {b} est dans deux corps (Répéter {first} et Répéter {})",
                                n.id,
                                b = b,
                                first = first
                            )
                        };
                        return Err(node_err(*b, msg));
                    }
                }
            }
        }
        let owners = self.body_owners();
        for (b, owner) in &owners {
            let Some(n) = self.node(*b) else {
                return Err(node_err(
                    *owner,
                    syfrack_i18n::tr_format!(
                        "le corps de Répéter {owner} cite le nœud inconnu {b}",
                        b = b,
                        owner = owner
                    ),
                ));
            };
            if !n.kind.is_point_to_point() {
                return Err(node_err(
                    *b,
                    syfrack_i18n::tr_format!(
                        "le corps de Répéter {owner} contient « {} », qui n'est pas Point → Point",
                        n.kind.label(),
                        owner = owner
                    ),
                ));
            }
            if self.links.iter().any(|l| l.from.0 == *b || l.to.0 == *b) {
                return Err(node_err(
                    *b,
                    syfrack_i18n::tr_format!(
                        "le nœud {b} du corps de Répéter {owner} ne peut pas être relié par un fil",
                        b = b,
                        owner = owner
                    ),
                ));
            }
        }
        for l in &self.links {
            let from = self.node(l.from.0).ok_or_else(|| {
                err(syfrack_i18n::tr_format!(
                    "fil depuis un nœud inconnu {}",
                    l.from.0
                ))
            })?;
            let to = self.node(l.to.0).ok_or_else(|| {
                err(syfrack_i18n::tr_format!(
                    "fil vers un nœud inconnu {}",
                    l.to.0
                ))
            })?;
            let out_ty = from
                .kind
                .outputs()
                .get(l.from.1 as usize)
                .copied()
                .ok_or_else(|| {
                    node_err(
                        from.id,
                        syfrack_i18n::tr_format!(
                            "port de sortie {} absent sur {}",
                            l.from.1,
                            from.name
                        ),
                    )
                })?;
            let in_ty = to
                .kind
                .inputs()
                .get(l.to.1 as usize)
                .copied()
                .ok_or_else(|| {
                    node_err(
                        to.id,
                        syfrack_i18n::tr_format!("port d'entrée {} absent sur {}", l.to.1, to.name),
                    )
                })?;
            if out_ty != in_ty {
                return Err(node_err(
                    to.id,
                    syfrack_i18n::tr_format!(
                        "fil de type {:?} vers un port {:?} ({} → {})",
                        out_ty,
                        in_ty,
                        from.name,
                        to.name
                    ),
                ));
            }
        }
        // A cycle is reported before duplicate ports: a loop usually feeds a port twice.
        self.check_acyclic()?;
        // BVH-pruned instance expansion has its own budget, independent of
        // fractal iteration counts. Nesting depth remains bounded.
        fn copies(g: &Graph, id: NodeId, cache: &mut HashMap<NodeId, (u32, u32)>) -> (u32, u32) {
            if let Some(&v) = cache.get(&id) {
                return v;
            }
            let n = g.node(id).expect("links checked");
            let mut budget = (0, 1);
            for port in 0..n.kind.inputs().len() as u32 {
                if let Some((src, _)) = g.input_of(id, port) {
                    let b = copies(g, src, cache);
                    budget.0 = budget.0.max(b.0);
                    budget.1 = budget.1.max(b.1);
                }
            }
            if let NodeKind::Instances { placements, .. } = &n.kind {
                if n.enabled {
                    budget.0 += 1;
                    budget.1 = budget.1.saturating_mul(placements.len() as u32);
                }
            }
            cache.insert(id, budget);
            budget
        }
        let mut budgets = HashMap::new();
        for n in &self.nodes {
            let (depth, product) = copies(self, n.id, &mut budgets);
            if depth > MAX_DEPTH || product > super::MAX_INSTANCE_PRODUCT {
                return Err(node_err(n.id,syfrack_i18n::tr_format!("Copies imbriquées : maximum {MAX_DEPTH} niveaux et {MAX_ITER_PRODUCT} placements par branche", MAX_DEPTH = MAX_DEPTH, MAX_ITER_PRODUCT = super::MAX_INSTANCE_PRODUCT)));
            }
        }
        let mut seen_to = HashSet::new();
        for l in &self.links {
            if !seen_to.insert(l.to) {
                let to = self.node(l.to.0).expect("checked above");
                return Err(node_err(
                    to.id,
                    syfrack_i18n::tr_format!("deux fils sur le port {} de {}", l.to.1, to.name),
                ));
            }
        }
        let depth = self.iterate_depth();
        if depth > MAX_DEPTH {
            return Err(err(syfrack_i18n::tr_format!(
                "profondeur de Répéter {depth} > {MAX_DEPTH}",
                MAX_DEPTH = MAX_DEPTH,
                depth = depth
            )));
        }
        let product = self.iteration_product();
        if product > MAX_ITER_PRODUCT {
            return Err(err(syfrack_i18n::tr_format!(
                "produit des itérations {product} > {MAX_ITER_PRODUCT}",
                MAX_ITER_PRODUCT = MAX_ITER_PRODUCT,
                product = product
            )));
        }
        Ok(())
    }

    /// Kahn's algorithm over every node and link, active or not.
    fn check_acyclic(&self) -> Result<(), SceneError> {
        let mut indeg: HashMap<NodeId, usize> = self.nodes.iter().map(|n| (n.id, 0)).collect();
        for l in &self.links {
            if let Some(d) = indeg.get_mut(&l.to.0) {
                *d += 1;
            }
        }
        let mut ready: Vec<NodeId> = indeg
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(id, _)| *id)
            .collect();
        let mut done = 0;
        while let Some(id) = ready.pop() {
            done += 1;
            for l in self.links.iter().filter(|l| l.from.0 == id) {
                if let Some(d) = indeg.get_mut(&l.to.0) {
                    *d -= 1;
                    if *d == 0 {
                        ready.push(l.to.0);
                    }
                }
            }
        }
        if done != indeg.len() {
            return Err(err(syfrack_i18n::tr!("cycle dans le graphe")));
        }
        Ok(())
    }

    /// The input ports, among active nodes, that no wire feeds (the Sortie
    /// first when it is unwired).
    pub fn unwired_inputs(&self) -> Vec<(NodeId, u32)> {
        let mut out = Vec::new();
        if let Some(o) = self.nodes.iter().find(|n| n.kind == NodeKind::Output) {
            if self.input_of(o.id, 0).is_none() {
                out.push((o.id, 0));
            }
        }
        for id in self.topo_order().unwrap_or_default() {
            let Some(n) = self.node(id) else { continue };
            for (port, _) in n.kind.inputs().iter().enumerate() {
                if self.input_of(id, port as u32).is_none() && !out.contains(&(id, port as u32)) {
                    out.push((id, port as u32));
                }
            }
        }
        out
    }

    /// What compiling and rendering need: the structure, and every port of
    /// every active node wired, the Sortie included.
    pub fn validate(&self) -> Result<(), SceneError> {
        self.validate_structure()?;
        let out = self
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Output)
            .expect("checked by the structure");
        if let Some((id, port)) = self.unwired_inputs().first() {
            if *id == out.id {
                return Err(node_err(
                    *id,
                    syfrack_i18n::tr!("la Sortie n'est pas reliée"),
                ));
            }
            let n = self.node(*id).expect("an active node");
            return Err(node_err(
                *id,
                syfrack_i18n::tr_format!(
                    "port d'entrée {port} de « {} » non relié",
                    n.name,
                    port = port
                ),
            ));
        }
        Ok(())
    }
}

/// The node id in a parameter path (`fractal.nodes[3].factor`).
fn node_in_path(text: &str) -> Option<NodeId> {
    let rest = text.split("nodes[").nth(1)?;
    rest.split(']').next()?.parse().ok()
}

fn node_err(id: NodeId, msg: impl Into<String>) -> SceneError {
    SceneError::Node {
        id,
        msg: msg.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::scene::SceneError;

    fn node(id: NodeId, kind: NodeKind) -> Node {
        Node {
            id,
            kind,
            name: format!("n{id}"),
            pos: [0.0, 0.0],
            enabled: true,
        }
    }
    fn link(a: NodeId, b: NodeId) -> Link {
        Link {
            from: (a, 0),
            to: (b, 0),
        }
    }

    #[test]
    fn the_minimal_graph_is_valid_and_topo_sorted() {
        let g = Graph::new_minimal();
        g.validate().unwrap();
        assert_eq!(g.topo_order().unwrap(), vec![0, 1, 2]);
    }

    #[test]
    fn a_cycle_is_refused() {
        let mut g = Graph::new_minimal();
        g.nodes.push(node(3, NodeKind::BoxFold { limit: 1.0 }));
        g.nodes.push(node(4, NodeKind::Scale { factor: 2.0 }));
        g.links = vec![link(0, 3), link(3, 4), link(4, 3), link(4, 1), link(1, 2)];
        let err = g.validate().unwrap_err().to_string();
        assert!(err.contains("Cycle"), "{err}");
    }

    #[test]
    fn a_mistyped_link_is_refused() {
        let mut g = Graph::new_minimal();
        g.links = vec![link(0, 2), link(1, 2)]; // Point → Output(Distance)
        let err = g.validate().unwrap_err().to_string();
        assert!(err.contains("type"), "{err}");
    }

    #[test]
    fn two_links_into_one_port_are_refused() {
        let mut g = Graph::new_minimal();
        g.nodes.push(node(3, NodeKind::Sphere { radius: 2.0 }));
        g.links.push(link(0, 3));
        g.links.push(link(3, 2));
        let err = g.validate().unwrap_err().to_string();
        assert!(err.contains("port"), "{err}");
    }

    #[test]
    fn missing_or_duplicate_output_is_refused() {
        let mut g = Graph::new_minimal();
        g.nodes.retain(|n| n.kind != NodeKind::Output);
        g.links.retain(|l| l.to.0 != 2);
        assert!(g.validate().unwrap_err().to_string().contains("Output"));
        let mut g = Graph::new_minimal();
        g.nodes.push(node(3, NodeKind::Output));
        assert!(g.validate().unwrap_err().to_string().contains("Output"));
    }

    #[test]
    fn an_iterate_body_only_holds_point_to_point_nodes() {
        let mut g = Graph::new_minimal();
        g.nodes.push(node(
            3,
            NodeKind::Iterate {
                iterations: 4,
                bailout: 4.0,
                body: vec![4],
            },
        ));
        g.nodes.push(node(4, NodeKind::Sphere { radius: 1.0 })); // Distance node inside a body
        g.links = vec![link(0, 3), link(3, 1), link(1, 2)];
        let err = g.validate().unwrap_err().to_string();
        assert!(err.contains("Repeat"), "{err}");
    }

    #[test]
    fn body_nodes_cannot_also_be_wired() {
        let mut g = Graph::new_minimal();
        g.nodes.push(node(
            3,
            NodeKind::Iterate {
                iterations: 4,
                bailout: 4.0,
                body: vec![4],
            },
        ));
        g.nodes.push(node(4, NodeKind::BoxFold { limit: 1.0 }));
        g.links = vec![link(0, 3), link(3, 1), link(1, 2), link(0, 4)];
        assert!(g.validate().is_err());
    }

    #[test]
    fn limits_are_enforced() {
        let mut g = Graph::new_minimal();
        let inner = node(
            3,
            NodeKind::Iterate {
                iterations: 16,
                bailout: 4.0,
                body: vec![],
            },
        );
        let outer = node(
            4,
            NodeKind::Iterate {
                iterations: 32,
                bailout: 4.0,
                body: vec![3],
            },
        );
        g.nodes.push(inner);
        g.nodes.push(outer);
        g.links = vec![link(0, 4), link(4, 1), link(1, 2)];
        let err = g.validate().unwrap_err().to_string();
        assert!(err.contains("Iteration"), "{err}"); // 16 × 32 = 512 > 256
        let mut g = Graph::new_minimal();
        g.nodes[1].kind = NodeKind::Scale { factor: 0.0 }; // not a fence any more and zero scale
        assert!(g.validate().is_err());
    }

    #[test]
    fn a_dangling_node_is_inactive_but_valid() {
        let mut g = Graph::new_minimal();
        g.nodes.push(node(3, NodeKind::BoxFold { limit: 1.0 }));
        g.validate().unwrap();
        assert!(!g.active_nodes().contains(&3));
        assert_eq!(g.topo_order().unwrap(), vec![0, 1, 2]);
    }

    #[test]
    fn out_of_bounds_parameter_is_reported_with_its_path() {
        let mut g = Graph::new_minimal();
        g.nodes[1].kind = NodeKind::Sphere { radius: 1000.0 };
        let err = g.validate().unwrap_err().to_string();
        assert!(err.contains("nodes[1].radius"), "{err}");
    }

    #[test]
    fn a_dangling_iterate_does_not_count_in_the_iteration_product() {
        let mut g = Graph::new_minimal();
        assert_eq!(g.iteration_product(), 1);
        g.nodes.push(node(
            3,
            NodeKind::Iterate {
                iterations: 64,
                bailout: 4.0,
                body: vec![],
            },
        ));
        g.validate().unwrap();
        assert!(!g.active_nodes().contains(&3));
        assert_eq!(g.iteration_product(), 1, "only active nodes count");
        // A dangling nest above the limit does not block validation either.
        g.nodes.push(node(
            4,
            NodeKind::Iterate {
                iterations: 64,
                bailout: 4.0,
                body: vec![3],
            },
        ));
        g.validate().unwrap();
        assert_eq!(g.iteration_product(), 1);
    }

    fn node_id_of(e: SceneError) -> Option<NodeId> {
        match e {
            SceneError::Node { id, .. } => Some(id),
            _ => None,
        }
    }

    #[test]
    fn a_node_in_two_bodies_or_twice_in_one_is_refused() {
        let iterate = |id, body: Vec<NodeId>| {
            node(
                id,
                NodeKind::Iterate {
                    iterations: 2,
                    bailout: 4.0,
                    body,
                },
            )
        };
        let mut g = Graph::new_minimal();
        g.nodes.push(iterate(3, vec![4]));
        g.nodes.push(node(4, NodeKind::Scale { factor: 2.0 }));
        g.nodes.push(iterate(5, vec![4]));
        g.links = vec![link(0, 3), link(3, 5), link(5, 1), link(1, 2)];
        let e = g.validate_structure().unwrap_err();
        assert!(e.to_string().contains("two bodies"), "{e}");
        assert_eq!(node_id_of(e), Some(4));

        let mut g = Graph::new_minimal();
        g.nodes.push(iterate(3, vec![4, 4]));
        g.nodes.push(node(4, NodeKind::Scale { factor: 2.0 }));
        g.links = vec![link(0, 3), link(3, 1), link(1, 2)];
        let e = g.validate_structure().unwrap_err();
        assert!(e.to_string().contains("twice"), "{e}");
        assert_eq!(node_id_of(e), Some(4));

        // Control: one body, once, is fine.
        g.nodes[3] = iterate(3, vec![4]);
        g.validate_structure().unwrap();
    }

    #[test]
    fn an_incomplete_graph_passes_the_structure_check_only() {
        let mut g = Graph::new_minimal();
        g.links.retain(|l| l.to.0 != 2); // Sortie unwired
        g.validate_structure().unwrap();
        assert_eq!(node_id_of(g.validate().unwrap_err()), Some(2));
        let mut g = Graph::new_minimal();
        g.nodes.push(node(3, NodeKind::Union));
        g.links.retain(|l| l.to.0 != 2);
        g.links.push(link(1, 3));
        g.links.push(link(3, 2)); // Union's second input unwired
        g.validate_structure().unwrap();
        assert_eq!(node_id_of(g.validate().unwrap_err()), Some(3));
    }

    #[test]
    fn a_dangling_cycle_is_refused_by_both_checks() {
        let mut g = Graph::new_minimal();
        g.nodes.push(node(3, NodeKind::BoxFold { limit: 1.0 }));
        g.nodes.push(node(4, NodeKind::Scale { factor: 2.0 }));
        g.links.push(link(3, 4));
        g.links.push(link(4, 3));
        assert!(g
            .validate_structure()
            .unwrap_err()
            .to_string()
            .contains("Cycle"));
        assert!(g.validate().unwrap_err().to_string().contains("Cycle"));
    }

    #[test]
    fn node_faults_name_their_node() {
        let mut g = Graph::new_minimal();
        g.nodes[1].kind = NodeKind::Sphere { radius: 1000.0 };
        assert_eq!(node_id_of(g.validate_structure().unwrap_err()), Some(1));
        let mut g = Graph::new_minimal();
        g.links = vec![link(0, 2), link(1, 2)];
        assert_eq!(node_id_of(g.validate_structure().unwrap_err()), Some(2));
        let mut g = Graph::new_minimal();
        g.nodes.push(node(3, NodeKind::Scale { factor: 0.0 }));
        assert_eq!(node_id_of(g.validate_structure().unwrap_err()), Some(3));
        let mut g = Graph::new_minimal();
        g.nodes.push(node(
            3,
            NodeKind::Iterate {
                iterations: 4,
                bailout: 4.0,
                body: vec![4],
            },
        ));
        g.nodes.push(node(4, NodeKind::Sphere { radius: 1.0 }));
        assert_eq!(node_id_of(g.validate_structure().unwrap_err()), Some(4));
        let msg = SceneError::Node {
            id: 4,
            msg: "x".into(),
        }
        .to_string();
        assert_eq!(msg, "graph: node 4: x");
    }
}
