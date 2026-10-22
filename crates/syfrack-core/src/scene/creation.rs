//! The authoritative, typed scene DAG. Its compiler lowers to the existing
//! geometric distance machine and field transport; there is no image compositor
//! or independent flame export path here.
use super::{phenomena::*, *};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_CREATION_NODES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SocketType {
    Point,
    Distance,
    Scalar,
    Vector,
    Complex,
    Texture,
    State,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CreationKind {
    Form(NodeKind),
    Field(FieldOp),
    Pattern {
        fractal: PlanarFractal,
        channels: OrbitColoring,
    },
    Texture(Box<FractalTexture>),
    Flame(FlameField),
    Simulation(Simulation),
    SampleState {
        channel: u32,
    },
    Palette(Ramp),
    Mix,
    Output,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CreationNode {
    pub id: u32,
    pub name: String,
    pub kind: CreationKind,
    pub pos: [f32; 2],
    /// Disabled geometric operations retain the native graph bypass semantics.
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
}
fn enabled_by_default() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CreationGraph {
    pub nodes: Vec<CreationNode>,
    pub links: Vec<Link>,
    pub safety: f32,
    pub bounding_radius: f32,
    /// Domain, optics and animation belong to this graph. `fields`, resources
    /// and bindings are derived by the compiler, never edited in parallel.
    pub settings: Phenomena,
}
impl Default for CreationGraph {
    fn default() -> Self {
        Self {
            nodes: vec![],
            links: vec![],
            safety: 0.7,
            bounding_radius: 4.0,
            settings: Phenomena::default(),
        }
    }
}
fn error(message: impl Into<String>) -> SceneError {
    SceneError::Graph(format!("creation: {}", message.into()))
}
fn socket(t: FieldType) -> SocketType {
    match t {
        FieldType::Scalar => SocketType::Scalar,
        FieldType::Vector => SocketType::Vector,
        FieldType::Complex => SocketType::Complex,
    }
}
fn field_type(t: SocketType) -> Result<FieldType, SceneError> {
    match t {
        SocketType::Scalar => Ok(FieldType::Scalar),
        SocketType::Vector => Ok(FieldType::Vector),
        SocketType::Complex => Ok(FieldType::Complex),
        _ => Err(error("a geometric state is not a scalar field")),
    }
}
impl CreationKind {
    pub fn label(&self) -> String {
        match self {
            Self::Form(k) => k.label().to_string(),
            Self::Pattern { .. } | Self::Texture(_) => "Motif fractal".into(),
            Self::Flame(_) => "Flamme fractale".into(),
            Self::Simulation(s) => s.name.clone(),
            Self::SampleState { .. } => "Échantillonner l’état".into(),
            Self::Palette(_) => "Palette".into(),
            Self::Mix => "Mélanger".into(),
            Self::Output => "Scène".into(),
            Self::Field(k) => match k {
                FieldOp::Position => "Position",
                FieldOp::Time => "Temps",
                FieldOp::Scalar { .. } => "Valeur",
                FieldOp::Vector { .. } => "Vecteur / couleur",
                FieldOp::Complex { .. } => "Complexe",
                FieldOp::Add => "Somme",
                FieldOp::Subtract => "Différence",
                FieldOp::Multiply => "Produit",
                FieldOp::Scale => "Multiplier par une valeur",
                FieldOp::Dot => "Produit scalaire",
                FieldOp::Cross => "Produit vectoriel",
                FieldOp::Length => "Longueur",
                FieldOp::Normalize => "Normaliser",
                FieldOp::Sine => "Sinus",
                FieldOp::Cosine => "Cosinus",
                FieldOp::Exponential => "Exponentielle",
                FieldOp::Absolute => "Valeur absolue",
                FieldOp::Clamp { .. } => "Borner",
                FieldOp::Power { .. } => "Puissance",
                FieldOp::Gradient => "Gradient",
                FieldOp::Divergence => "Divergence",
                FieldOp::Curl => "Rotationnel",
                FieldOp::ComplexFromParts => "Construire un complexe",
                FieldOp::ComplexProduct => "Produit complexe",
                FieldOp::ComplexPower { .. } => "Puissance complexe",
                FieldOp::Phase => "Phase",
                FieldOp::Intensity => "Intensité",
                FieldOp::PlaneWave { .. } => "Onde plane",
                FieldOp::GaussianWave { .. } => "Onde gaussienne",
                FieldOp::SphereDistance { .. } => "Distance sphérique",
                FieldOp::QuaternionJulia { .. } => "Julia quaternionique",
                FieldOp::Mobius { .. } => "Möbius",
                FieldOp::GeometryDistance => "Distance de la forme",
                FieldOp::GeometryTrap { .. } => "Piège de la forme",
                FieldOp::Snapshot { .. } => "Échantillonner l’état",
                FieldOp::Planar { .. } | FieldOp::Texture { .. } => "Motif fractal",
                FieldOp::Flame { .. } => "Flamme fractale",
                FieldOp::Palette { .. } => "Palette",
                FieldOp::Mix => "Mélanger les couleurs",
                FieldOp::Luminance => "Luminance",
            }
            .to_string(),
        }
    }
    pub fn input_labels(&self) -> Vec<&'static str> {
        match self {
            Self::Form(k) => k
                .inputs()
                .iter()
                .enumerate()
                .map(|(i, _)| if i == 0 { "Entrée" } else { "B" })
                .collect(),
            Self::Pattern { .. } | Self::Texture(_) | Self::Flame(_) => vec!["Position"],
            Self::Simulation(_) => vec!["Position", "Potentiel", "Source", "Masque"],
            Self::SampleState { .. } => vec!["État", "Position"],
            Self::Palette(_) => vec!["Valeur"],
            Self::Mix => vec!["A", "B", "Facteur"],
            Self::Output => vec![
                "Géométrie",
                "Couleur",
                "Émission",
                "Densité",
                "Déplacement",
                "Nacre",
                "Isosurface",
                "Surface",
            ],
            Self::Field(op) => match op {
                FieldOp::Position
                | FieldOp::Time
                | FieldOp::Scalar { .. }
                | FieldOp::Vector { .. }
                | FieldOp::Complex { .. } => vec![],
                FieldOp::Add
                | FieldOp::Subtract
                | FieldOp::Multiply
                | FieldOp::Scale
                | FieldOp::Dot
                | FieldOp::Cross
                | FieldOp::ComplexFromParts
                | FieldOp::ComplexProduct => vec!["A", "B"],
                FieldOp::Mix => vec!["A", "B", "Facteur"],
                _ => vec!["Entrée"],
            },
        }
    }
    pub fn output_labels(&self) -> Vec<&'static str> {
        match self {
            Self::Output => vec![],
            Self::Flame(_) => vec!["Couleur", "Densité"],
            Self::Texture(_) => vec!["Valeur", "Surface"],
            Self::Simulation(s) => {
                let mut labels = vec!["Canal 0", "Canal 1", "Canal 2", "Canal 3"]
                    [..s.channels() as usize]
                    .to_vec();
                labels.push("État");
                labels
            }
            _ => vec!["Sortie"],
        }
    }
}
impl CreationGraph {
    pub fn node(&self, id: u32) -> Option<&CreationNode> {
        self.nodes.iter().find(|n| n.id == id)
    }
    pub fn next_id(&self) -> u32 {
        self.nodes
            .iter()
            .map(|n| n.id)
            .max()
            .map_or(0, |n| n.saturating_add(1))
    }
    pub fn add(&mut self, kind: CreationKind, pos: [f32; 2]) -> u32 {
        let id = self.next_id();
        let name = kind.label();
        self.nodes.push(CreationNode {
            id,
            name,
            kind,
            pos,
            enabled: true,
        });
        id
    }
    pub fn connect(&mut self, from: (u32, u32), to: (u32, u32)) -> Result<(), SceneError> {
        let mut next = self.clone();
        next.links.retain(|l| l.to != to);
        next.links.push(Link { from, to });
        next.types()?;
        if next
            .node(to.0)
            .is_some_and(|n| matches!(n.kind, CreationKind::Output))
        {
            if to.1 == 6 && next.settings.bindings.geometry_mode == FieldGeometry::None {
                next.settings.bindings.geometry_mode = FieldGeometry::Replace;
            }
            if to.1 == 4 && next.settings.bindings.displacement_amount == 0.0 {
                next.settings.bindings.displacement_amount = 0.1;
            }
            if to.1 == 5 && next.settings.film.is_none() {
                next.settings.film = Some(ThinFilm::default());
            }
        }
        *self = next;
        Ok(())
    }
    pub fn input(&self, id: u32, port: u32) -> Option<(u32, u32)> {
        self.links
            .iter()
            .find(|l| l.to == (id, port))
            .map(|l| l.from)
    }
    pub fn active_nodes(&self) -> BTreeSet<u32> {
        let mut active = BTreeSet::new();
        let mut pending: Vec<_> = self
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, CreationKind::Output))
            .map(|n| n.id)
            .collect();
        while let Some(id) = pending.pop() {
            if active.insert(id) {
                pending.extend(self.links.iter().filter(|l| l.to.0 == id).map(|l| l.from.0));
            }
        }
        active
    }
    pub fn resource_nodes(&self) -> Result<(Vec<u32>, Vec<u32>), SceneError> {
        let active = self.active_nodes();
        let mut sims = vec![];
        let mut flames = vec![];
        for id in self.order()?.into_iter().filter(|id| active.contains(id)) {
            match self.node(id).unwrap().kind {
                CreationKind::Simulation(_) => sims.push(id),
                CreationKind::Flame(_) => flames.push(id),
                _ => {}
            }
        }
        Ok((sims, flames))
    }
    pub fn order(&self) -> Result<Vec<u32>, SceneError> {
        if self.nodes.len() > MAX_CREATION_NODES {
            return Err(error("at most 256 nodes"));
        }
        let ids: BTreeSet<_> = self.nodes.iter().map(|n| n.id).collect();
        if ids.len() != self.nodes.len() {
            return Err(error("duplicate node id"));
        }
        let mut destinations = BTreeSet::new();
        for l in &self.links {
            if !ids.contains(&l.from.0) || !ids.contains(&l.to.0) {
                return Err(error("wire references a missing node"));
            }
            if !destinations.insert(l.to) {
                return Err(error("two wires feed one socket"));
            }
        }
        let mut done = BTreeSet::new();
        let mut result = vec![];
        while done.len() < ids.len() {
            let before = done.len();
            for &id in &ids {
                if !done.contains(&id)
                    && self
                        .links
                        .iter()
                        .filter(|l| l.to.0 == id)
                        .all(|l| done.contains(&l.from.0))
                {
                    done.insert(id);
                    result.push(id);
                }
            }
            if before == done.len() {
                return Err(error(
                    "cycle: feedback must use an explicit simulation state",
                ));
            }
        }
        Ok(result)
    }
    /// Validate existing wires even while a new, unconnected node is being built.
    pub fn types(&self) -> Result<BTreeMap<(u32, u32), SocketType>, SceneError> {
        use SocketType::*;
        let mut types = BTreeMap::new();
        for id in self.order()? {
            let n = self.node(id).unwrap();
            if !n.pos.iter().all(|v| v.is_finite()) {
                return Err(error("nonfinite node position"));
            }
            let connected: Vec<_> = (0..n.kind.input_labels().len() as u32)
                .map(|p| self.input(id, p).and_then(|s| types.get(&s).copied()))
                .collect();
            let first = connected.first().copied().flatten().unwrap_or(Scalar);
            let (inputs, outputs) = match &n.kind {
                CreationKind::Form(k) => (
                    k.inputs()
                        .iter()
                        .map(|t| match t {
                            PortType::Point => Point,
                            PortType::Distance => Distance,
                        })
                        .collect(),
                    k.outputs()
                        .iter()
                        .map(|t| match t {
                            PortType::Point => Point,
                            PortType::Distance => Distance,
                        })
                        .collect(),
                ),
                CreationKind::Pattern { .. } => (vec![Vector], vec![Scalar]),
                CreationKind::Texture(_) => (vec![Vector], vec![Scalar, Texture]),
                CreationKind::Flame(_) => (vec![Vector], vec![Vector, Scalar]),
                CreationKind::Simulation(s) => {
                    let mut out = vec![Scalar; s.channels() as usize];
                    out.push(State);
                    (vec![Vector, Scalar, Scalar, Scalar], out)
                }
                CreationKind::SampleState { channel } => {
                    if let Some((source, _)) = self.input(id, 0) {
                        if let CreationKind::Simulation(s) = &self.node(source).unwrap().kind {
                            if *channel >= s.channels() {
                                return Err(error(format!(
                                    "node {id}: this state has only {} channels",
                                    s.channels()
                                )));
                            }
                        }
                    }
                    (vec![State, Vector], vec![Scalar])
                }
                CreationKind::Palette(_) => (vec![Scalar], vec![Vector]),
                CreationKind::Mix => (vec![Vector, Vector, Scalar], vec![Vector]),
                CreationKind::Output => (
                    vec![
                        Distance, Vector, Scalar, Scalar, Scalar, Scalar, Scalar, Texture,
                    ],
                    vec![],
                ),
                CreationKind::Field(op) => {
                    let ft = field_type(first)?;
                    let (ins, out) = op.signature(ft);
                    (ins.into_iter().map(socket).collect(), vec![socket(out)])
                }
            };
            for l in self.links.iter().filter(|l| l.to.0 == id) {
                let expected = inputs
                    .get(l.to.1 as usize)
                    .ok_or_else(|| error(format!("node {id}: input socket out of range")))?;
                let actual = types
                    .get(&l.from)
                    .ok_or_else(|| error("output socket out of range"))?;
                if expected != actual {
                    return Err(error(format!(
                        "node {id}, socket {}: expected {expected:?}, received {actual:?}",
                        l.to.1
                    )));
                }
            }
            for (p, t) in outputs.into_iter().enumerate() {
                types.insert((id, p as u32), t);
            }
        }
        Ok(types)
    }
    /// One source of truth, lowered for *both* the live GPU and final export.
    pub fn compile(&self, base: &Scene) -> Result<Scene, SceneError> {
        self.types()?;
        if !self.settings.fields.nodes.is_empty()
            || !self.settings.simulations.is_empty()
            || !self.settings.flames.is_empty()
            || self.settings.editor.is_some()
            || [
                self.settings.bindings.rgb,
                self.settings.bindings.color,
                self.settings.bindings.emission,
                self.settings.bindings.density,
                self.settings.bindings.displacement,
                self.settings.bindings.film_thickness,
                self.settings.bindings.geometry,
            ]
            .iter()
            .any(Option::is_some)
        {
            return Err(error(
                "graph settings cannot contain hidden sources or connections; use nodes and wires",
            ));
        }
        if self.nodes.iter().any(|n| {
            matches!(
                n.kind,
                CreationKind::Field(FieldOp::Snapshot { .. } | FieldOp::Flame { .. })
            )
        }) {
            return Err(error(
                "resource fields require an explicit Flame or SampleState node",
            ));
        }
        let outputs: Vec<_> = self
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, CreationKind::Output))
            .collect();
        if outputs.len() != 1 {
            return Err(error("exactly one Scene output is required"));
        }
        let out = outputs[0].id;
        let mut active = BTreeSet::new();
        let mut pending = vec![out];
        while let Some(id) = pending.pop() {
            if active.insert(id) {
                pending.extend(self.links.iter().filter(|l| l.to.0 == id).map(|l| l.from.0));
            }
        }
        let mut scene = base.clone();
        scene.creation = None;
        scene.creation_draft = None;
        scene.canvas.flame = None;
        scene.material.texture = None;
        if let Some((id, 1)) = self.input(out, 7) {
            let CreationKind::Texture(texture) = &self.node(id).unwrap().kind else {
                return Err(error("Surface requires a texture source"));
            };
            scene.material.texture = Some((**texture).clone());
        }
        let mut p = self.settings.clone();
        p.fields.nodes.clear();
        p.simulations.clear();
        p.flames.clear();
        p.editor = None;
        let old = p.bindings.clone();
        p.bindings = FieldBindings {
            palette: old.palette,
            ..Default::default()
        };
        p.bindings.isovalue = old.isovalue;
        p.bindings.displacement_amount = old.displacement_amount;
        p.bindings.density_gain = old.density_gain;
        p.bindings.emission_gain = old.emission_gain;
        p.bindings.geometry_mode = old.geometry_mode;
        let mut mapped = BTreeMap::<(u32, u32), u32>::new();
        let mut states = BTreeMap::<(u32, u32), u32>::new();
        let mut next = 0u32;
        let emit =
            |p: &mut Phenomena, next: &mut u32, op: FieldOp, inputs: Vec<u32>, n: &CreationNode| {
                let id = *next;
                *next += 1;
                p.fields.nodes.push(FieldNode {
                    id,
                    name: n.name.clone(),
                    op,
                    inputs,
                    pos: n.pos,
                });
                id
            };
        let mut geo = base.fractal.graph().clone();
        if let Some((root, 0)) = self.input(out, 0) {
            // Include loop body cards as well: their dependency is the Iterate
            // body list, rather than ordinary wires.
            let mut forms: BTreeSet<_> = active
                .iter()
                .filter(|id| matches!(self.node(**id).unwrap().kind, CreationKind::Form(_)))
                .copied()
                .collect();
            let mut todo: Vec<_> = forms.iter().copied().collect();
            while let Some(id) = todo.pop() {
                if let CreationKind::Form(NodeKind::Iterate { body, .. }) = &self
                    .node(id)
                    .ok_or_else(|| error("missing loop body node"))?
                    .kind
                {
                    for &b in body {
                        if !matches!(self.node(b).map(|n| &n.kind), Some(CreationKind::Form(_))) {
                            return Err(error(
                                "a geometric loop body must contain geometric operations",
                            ));
                        }
                        if forms.insert(b) {
                            todo.push(b);
                        }
                    }
                }
            }
            geo.nodes = self
                .nodes
                .iter()
                .filter(|n| forms.contains(&n.id))
                .map(|n| {
                    let CreationKind::Form(kind) = &n.kind else {
                        unreachable!()
                    };
                    Node {
                        id: n.id,
                        kind: kind.clone(),
                        name: n.name.clone(),
                        pos: n.pos,
                        enabled: n.enabled,
                    }
                })
                .collect();
            geo.links = self
                .links
                .iter()
                .filter(|l| forms.contains(&l.from.0) && forms.contains(&l.to.0))
                .cloned()
                .collect();
            let id = out;
            geo.nodes.push(Node {
                id,
                kind: NodeKind::Output,
                name: outputs[0].name.clone(),
                pos: outputs[0].pos,
                enabled: true,
            });
            geo.links.push(Link {
                from: (root, 0),
                to: (id, 0),
            });
            geo.safety = self.safety;
            geo.bounding_radius_hint = self.bounding_radius;
            geo.validate()?;
            scene.fractal = Fractal::Graph(geo);
        } else if base.dimension == Dimension::ThreeD
            && (self.input(out, 6).is_none()
                || self.settings.bindings.geometry_mode == FieldGeometry::None)
        {
            return Err(error("connect geometry to the Scene output"));
        }
        for id in self.order()?.into_iter().filter(|id| active.contains(id)) {
            let n = self.node(id).unwrap();
            if matches!(n.kind, CreationKind::Form(_) | CreationKind::Output) {
                continue;
            }
            if matches!(n.kind, CreationKind::Texture(_))
                && !self
                    .links
                    .iter()
                    .any(|l| l.from == (id, 0) && active.contains(&l.to.0))
                && !(base.dimension == Dimension::TwoD
                    && self.input(out, 7) == Some((id, 1))
                    && self.input(out, 1).is_none())
            {
                continue;
            }
            if let CreationKind::SampleState { channel } = &n.kind {
                let state = self
                    .input(id, 0)
                    .and_then(|s| states.get(&s).copied())
                    .ok_or_else(|| error(format!("node {id}: connect a simulation state")))?;
                let position = self
                    .input(id, 1)
                    .and_then(|s| mapped.get(&s).copied())
                    .ok_or_else(|| error(format!("node {id}: connect sample position")))?;
                let i = emit(
                    &mut p,
                    &mut next,
                    FieldOp::Snapshot {
                        simulation: state,
                        channel: *channel,
                    },
                    vec![position],
                    n,
                );
                mapped.insert((id, 0), i);
                continue;
            }
            let mut ins = vec![];
            let required = match &n.kind {
                CreationKind::Simulation(_) => 1,
                _ => n.kind.input_labels().len(),
            };
            for port in 0..required as u32 {
                let src = self.input(id, port).ok_or_else(|| {
                    error(format!(
                        "node {id}: connect {}",
                        n.kind.input_labels()[port as usize]
                    ))
                })?;
                ins.push(
                    *mapped
                        .get(&src)
                        .ok_or_else(|| error("a field input cannot carry a geometric state"))?,
                );
            }
            match &n.kind {
                CreationKind::Field(op) => {
                    let i = emit(&mut p, &mut next, op.clone(), ins, n);
                    mapped.insert((id, 0), i);
                }
                CreationKind::Pattern { fractal, channels } => {
                    let i = emit(
                        &mut p,
                        &mut next,
                        FieldOp::Planar {
                            fractal: fractal.clone(),
                            channels: *channels,
                        },
                        ins,
                        n,
                    );
                    mapped.insert((id, 0), i);
                }
                CreationKind::Texture(texture) => {
                    let i = emit(
                        &mut p,
                        &mut next,
                        FieldOp::Texture {
                            texture: texture.clone(),
                        },
                        ins,
                        n,
                    );
                    mapped.insert((id, 0), i);
                }
                CreationKind::Palette(ramp) => {
                    let i = emit(
                        &mut p,
                        &mut next,
                        FieldOp::Palette { ramp: ramp.clone() },
                        ins,
                        n,
                    );
                    mapped.insert((id, 0), i);
                }
                CreationKind::Mix => {
                    let i = emit(&mut p, &mut next, FieldOp::Mix, ins, n);
                    mapped.insert((id, 0), i);
                }
                CreationKind::Flame(f) => {
                    let source = p.flames.len() as u32;
                    p.flames.push(f.clone());
                    for (port, channel) in [FlameChannel::Color, FlameChannel::Density]
                        .into_iter()
                        .enumerate()
                    {
                        let i = emit(
                            &mut p,
                            &mut next,
                            FieldOp::Flame { source, channel },
                            ins.clone(),
                            n,
                        );
                        mapped.insert((id, port as u32), i);
                    }
                }
                CreationKind::Simulation(s) => {
                    let mut s = s.clone();
                    let refs: [Option<u32>; 3] = std::array::from_fn(|j| {
                        self.input(id, j as u32 + 1)
                            .and_then(|a| mapped.get(&a).copied())
                    });
                    s.potential = refs[0];
                    s.source = refs[1];
                    s.mask = refs[2];
                    let source = p.simulations.len() as u32;
                    let channels = s.channels();
                    p.simulations.push(s);
                    states.insert((id, channels), source);
                    for channel in 0..channels {
                        if self
                            .links
                            .iter()
                            .any(|l| l.from == (id, channel) && active.contains(&l.to.0))
                        {
                            let i = emit(
                                &mut p,
                                &mut next,
                                FieldOp::Snapshot {
                                    simulation: source,
                                    channel,
                                },
                                ins.clone(),
                                n,
                            );
                            mapped.insert((id, channel), i);
                        }
                    }
                }
                _ => unreachable!(),
            }
        }
        let roots: [Option<u32>; 5] = std::array::from_fn(|j| {
            self.input(out, j as u32 + 1)
                .and_then(|s| mapped.get(&s).copied())
        });
        p.bindings.rgb = roots[0];
        p.bindings.emission = roots[1];
        p.bindings.density = roots[2];
        p.bindings.displacement = roots[3];
        p.bindings.film_thickness = roots[4];
        if base.dimension == Dimension::TwoD && p.bindings.rgb.is_none() {
            if let Some((id, 1)) = self.input(out, 7) {
                let n = self.node(id).unwrap();
                let CreationKind::Texture(t) = &n.kind else {
                    unreachable!()
                };
                let input = mapped[&(id, 0)];
                let rgb = emit(
                    &mut p,
                    &mut next,
                    FieldOp::Palette {
                        ramp: t.ramp.clone(),
                    },
                    vec![input],
                    n,
                );
                p.bindings.rgb = Some(rgb);
            }
        }
        p.bindings.geometry = self.input(out, 6).and_then(|s| mapped.get(&s).copied());
        if p.bindings.geometry.is_none() {
            p.bindings.geometry_mode = FieldGeometry::None;
        }
        // The canonical graph owns all generation. A connected colour output
        // replaces the old standalone texture source.
        if p.bindings.rgb.is_some() && base.dimension == Dimension::TwoD {
            scene.material.texture = None;
        }
        let used = p.bindings.rgb.is_some()
            || !p.fields.nodes.is_empty()
            || p.film.is_some()
            || p.dispersion.is_some()
            || p.lens.is_some();
        scene.phenomena = used.then_some(p);
        scene.validate()?;
        Ok(scene)
    }
    /// Lift an old scene into the same canvas without discarding any generator.
    pub fn from_scene(scene: &Scene) -> Self {
        if let Some(g) = &scene.creation {
            return g.clone();
        }
        let mut g = Self::default();
        g.safety = scene.fractal.graph().safety;
        g.bounding_radius = scene.fractal.graph().bounding_radius_hint;
        let mut geom_out = None;
        for n in &scene.fractal.graph().nodes {
            if matches!(n.kind, NodeKind::Output) {
                geom_out = scene
                    .fractal
                    .graph()
                    .links
                    .iter()
                    .find(|l| l.to == (n.id, 0))
                    .map(|l| l.from);
                continue;
            }
            g.nodes.push(CreationNode {
                id: n.id,
                name: n.name.clone(),
                kind: CreationKind::Form(n.kind.clone()),
                pos: n.pos,
                enabled: n.enabled,
            });
        }
        g.links = scene
            .fractal
            .graph()
            .links
            .iter()
            .filter(|l| g.node(l.from.0).is_some() && g.node(l.to.0).is_some())
            .cloned()
            .collect();
        let output = if let Some(n) = scene
            .fractal
            .graph()
            .nodes
            .iter()
            .find(|n| matches!(n.kind, NodeKind::Output))
        {
            g.nodes.push(CreationNode {
                id: n.id,
                name: n.name.clone(),
                kind: CreationKind::Output,
                pos: n.pos,
                enabled: true,
            });
            n.id
        } else {
            g.add(CreationKind::Output, [1100.0, 220.0])
        };
        if let Some(from) = geom_out {
            g.links.push(Link {
                from,
                to: (output, 0),
            });
        }
        let position = g.add(CreationKind::Field(FieldOp::Position), [0.0, 620.0]);
        if let Some(p) = &scene.phenomena {
            g.settings = p.clone();
            g.settings.fields.nodes.clear();
            g.settings.editor = None;
            g.settings.simulations.clear();
            g.settings.flames.clear();
            g.settings.bindings = FieldBindings {
                palette: p.bindings.palette.clone(),
                isovalue: p.bindings.isovalue,
                displacement_amount: p.bindings.displacement_amount,
                density_gain: p.bindings.density_gain,
                emission_gain: p.bindings.emission_gain,
                geometry_mode: p.bindings.geometry_mode,
                ..Default::default()
            };
            // Resource cards are first-class graph nodes; old snapshot readers
            // are aliases for their typed output sockets.
            let sims: Vec<_> = p
                .simulations
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let mut s = s.clone();
                    s.potential = None;
                    s.source = None;
                    s.mask = None;
                    g.add(
                        CreationKind::Simulation(s),
                        [250.0 + i as f32 * 270.0, 900.0],
                    )
                })
                .collect();
            let mut flame_readers = BTreeMap::new();
            let mut aliases = BTreeMap::new();
            let mut position_claimed = false;
            for n in &p.fields.nodes {
                if matches!(n.op, FieldOp::Position) {
                    let id = if !position_claimed {
                        position_claimed = true;
                        position
                    } else {
                        g.add(
                            CreationKind::Field(FieldOp::Position),
                            [n.pos[0], n.pos[1] + 600.0],
                        )
                    };
                    aliases.insert(n.id, (id, 0));
                } else if let FieldOp::Snapshot {
                    simulation,
                    channel,
                } = n.op
                {
                    if let Some(&resource) = sims.get(simulation as usize) {
                        let id = g.add(
                            CreationKind::SampleState { channel },
                            [n.pos[0], n.pos[1] + 600.0],
                        );
                        aliases.insert(n.id, (id, 0));
                        g.links.push(Link {
                            from: (resource, p.simulations[simulation as usize].channels()),
                            to: (id, 0),
                        });
                    }
                } else if let FieldOp::Flame { source, channel } = n.op {
                    if let Some(f) = p.flames.get(source as usize) {
                        let id = *flame_readers
                            .entry((source, n.inputs.clone()))
                            .or_insert_with(|| {
                                g.add(CreationKind::Flame(f.clone()), [n.pos[0], n.pos[1] + 600.0])
                            });
                        aliases.insert(
                            n.id,
                            (id, if channel == FlameChannel::Color { 0 } else { 1 }),
                        );
                    }
                } else {
                    let id = g.add(
                        CreationKind::Field(n.op.clone()),
                        [n.pos[0], n.pos[1] + 600.0],
                    );
                    aliases.insert(n.id, (id, 0));
                }
            }
            let mut named = BTreeSet::new();
            for n in &p.fields.nodes {
                if let Some(&(id, _)) = aliases.get(&n.id) {
                    let target = g.nodes.iter_mut().find(|n| n.id == id).unwrap();
                    if named.insert(id) {
                        target.name = n.name.clone();
                        target.pos = [n.pos[0], n.pos[1] + 600.0];
                    } else if target.name != n.name {
                        target.name.push_str(" / ");
                        target.name.push_str(&n.name);
                    }
                }
            }
            for n in &p.fields.nodes {
                let Some(&(to, _)) = aliases.get(&n.id) else {
                    continue;
                };
                if matches!(n.op, FieldOp::Position) {
                    continue;
                }
                let max = if matches!(n.op, FieldOp::Snapshot { .. }) {
                    1
                } else {
                    n.inputs.len()
                };
                for (port, source) in n.inputs.iter().take(max).enumerate() {
                    if let Some(&from) = aliases.get(source) {
                        let port = if matches!(n.op, FieldOp::Snapshot { .. }) {
                            1
                        } else {
                            port as u32
                        };
                        if !g.links.iter().any(|l| l.to == (to, port)) {
                            g.links.push(Link {
                                from,
                                to: (to, port),
                            });
                        }
                    }
                }
            }
            for (i, s) in p.simulations.iter().enumerate() {
                if !g.links.iter().any(|l| l.to == (sims[i], 0)) {
                    g.links.push(Link {
                        from: (position, 0),
                        to: (sims[i], 0),
                    });
                }
                for (j, id) in [s.potential, s.source, s.mask].into_iter().enumerate() {
                    if let Some(from) = id.and_then(|id| aliases.get(&id).copied()) {
                        g.links.push(Link {
                            from,
                            to: (sims[i], j as u32 + 1),
                        });
                    }
                }
            }
            if let Some(rgb) = p.bindings.rgb.and_then(|id| aliases.get(&id).copied()) {
                g.links.push(Link {
                    from: rgb,
                    to: (output, 1),
                });
            } else if let Some(color) = p.bindings.color.and_then(|id| aliases.get(&id).copied()) {
                let palette = g.add(
                    CreationKind::Palette(p.bindings.palette.clone()),
                    [850.0, 620.0],
                );
                g.links.push(Link {
                    from: color,
                    to: (palette, 0),
                });
                g.links.push(Link {
                    from: (palette, 0),
                    to: (output, 1),
                });
            }
            for (port, id) in [
                (2, p.bindings.emission),
                (3, p.bindings.density),
                (4, p.bindings.displacement),
                (5, p.bindings.film_thickness),
                (6, p.bindings.geometry),
            ] {
                if let Some(from) = id.and_then(|id| aliases.get(&id).copied()) {
                    g.links.push(Link {
                        from,
                        to: (output, port),
                    });
                }
            }
            for track in &mut g.settings.timeline.tracks {
                if let Some(rest) = track.path.strip_prefix("phenomena.simulations[") {
                    if let Some((index, suffix)) = rest.split_once("].") {
                        if let Ok(index) = index.parse::<usize>() {
                            if let Some(id) = sims.get(index) {
                                track.path = format!("creation.nodes[{id}].{suffix}");
                            }
                        }
                    }
                } else if let Some(rest) = track.path.strip_prefix("phenomena.fields.nodes[") {
                    if let Some((index, suffix)) = rest.split_once("].") {
                        if let Ok(index) = index.parse::<u32>() {
                            if let Some((id, _)) = aliases.get(&index) {
                                track.path = format!("creation.nodes[{id}].{suffix}");
                            }
                        }
                    }
                } else if let Some(rest) = track.path.strip_prefix("phenomena.") {
                    track.path = format!("creation.settings.{rest}");
                }
            }
        }
        if let Some(flame) = &scene.canvas.flame {
            let mut f = FlameField::default();
            f.flame = flame.clone();
            f.center = scene.canvas.center;
            f.half = [3.0 / scene.canvas.zoom; 2];
            f.angle = scene.canvas.rotation;
            f.seed = scene.seed;
            let n = g.add(CreationKind::Flame(f), [300.0, 500.0]);
            g.links.push(Link {
                from: (position, 0),
                to: (n, 0),
            });
            // Existing simultaneous sources are composed, never selected by a
            // renderer-level `if flame` branch.
            g.combine_color(output, (n, 0));
        }
        if let Some(t) = &scene.material.texture {
            let n = g.add(CreationKind::Texture(Box::new(t.clone())), [300.0, 500.0]);
            g.links.push(Link {
                from: (position, 0),
                to: (n, 0),
            });
            if scene.dimension == Dimension::ThreeD {
                g.links.push(Link {
                    from: (n, 1),
                    to: (output, 7),
                });
            } else {
                let palette = g.add(CreationKind::Palette(t.ramp.clone()), [610.0, 500.0]);
                g.links.push(Link {
                    from: (n, 0),
                    to: (palette, 0),
                });
                // An old flame toggle hid its pattern. Keep that editable
                // branch without changing the active legacy flame image.
                if scene.canvas.flame.is_none() {
                    g.combine_color(output, (palette, 0));
                }
            }
        }
        g
    }
    fn combine_color(&mut self, output: u32, source: (u32, u32)) {
        if let Some(previous) = self.input(output, 1) {
            let mix = self.add(CreationKind::Field(FieldOp::Add), [850.0, 450.0]);
            self.links.retain(|l| l.to != (output, 1));
            self.links.extend([
                Link {
                    from: previous,
                    to: (mix, 0),
                },
                Link {
                    from: source,
                    to: (mix, 1),
                },
                Link {
                    from: (mix, 0),
                    to: (output, 1),
                },
            ]);
        } else {
            self.links.push(Link {
                from: source,
                to: (output, 1),
            });
        }
    }

    /// Adapter for existing geometry tools and library assets. Field/source
    /// cards and their connections survive a replacement of the form branch.
    pub fn replace_form(&mut self, form: &Graph) {
        let removed: BTreeSet<_> = self
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, CreationKind::Form(_)))
            .map(|n| n.id)
            .collect();
        self.nodes.retain(|n| !removed.contains(&n.id));
        self.links
            .retain(|l| !removed.contains(&l.from.0) && !removed.contains(&l.to.0));
        // Preserve the IDs used by geometric tools. Move colliding field
        // cards, including their wires and animation paths, out of that range.
        let incoming: BTreeSet<_> = form
            .nodes
            .iter()
            .filter(|n| !matches!(n.kind, NodeKind::Output))
            .map(|n| n.id)
            .collect();
        let mut next = self.next_id().max(
            form.nodes
                .iter()
                .map(|n| n.id)
                .max()
                .unwrap_or(0)
                .saturating_add(1),
        );
        let shifts: BTreeMap<_, _> = self
            .nodes
            .iter()
            .filter(|n| incoming.contains(&n.id))
            .map(|n| {
                let pair = (n.id, next);
                next += 1;
                pair
            })
            .collect();
        for n in &mut self.nodes {
            if let Some(&id) = shifts.get(&n.id) {
                n.id = id;
            }
        }
        for l in &mut self.links {
            if let Some(&id) = shifts.get(&l.from.0) {
                l.from.0 = id;
            }
            if let Some(&id) = shifts.get(&l.to.0) {
                l.to.0 = id;
            }
        }
        for track in &mut self.settings.timeline.tracks {
            for (old, new) in &shifts {
                if let Some(suffix) = track.path.strip_prefix(&format!("creation.nodes[{old}].")) {
                    track.path = format!("creation.nodes[{new}].{suffix}");
                    break;
                }
            }
        }
        let Some(out) = self
            .nodes
            .iter()
            .find(|n| matches!(n.kind, CreationKind::Output))
            .map(|n| n.id)
        else {
            return;
        };
        let ids: BTreeMap<_, _> = incoming.into_iter().map(|id| (id, id)).collect();
        for n in &form.nodes {
            if let Some(&id) = ids.get(&n.id) {
                let mut kind = n.kind.clone();
                if let NodeKind::Iterate { body, .. } = &mut kind {
                    for b in body {
                        if let Some(&id) = ids.get(b) {
                            *b = id;
                        }
                    }
                }
                self.nodes.push(CreationNode {
                    id,
                    name: n.name.clone(),
                    kind: CreationKind::Form(kind),
                    pos: n.pos,
                    enabled: n.enabled,
                });
            }
        }
        for l in &form.links {
            if let Some(&from) = ids.get(&l.from.0) {
                let to = ids.get(&l.to.0).copied().or_else(|| {
                    form.node(l.to.0)
                        .filter(|n| matches!(n.kind, NodeKind::Output))
                        .map(|_| out)
                });
                if let Some(to) = to {
                    self.links.push(Link {
                        from: (from, l.from.1),
                        to: (to, l.to.1),
                    });
                }
            }
        }
        self.safety = form.safety;
        self.bounding_radius = form.bounding_radius_hint;
    }
}

impl Scene {
    pub fn render_scene(&self) -> Result<Self, SceneError> {
        let graph = self
            .creation
            .clone()
            .unwrap_or_else(|| CreationGraph::from_scene(self));
        let mut lowered = graph.compile(self)?;
        lowered.creation = Some(graph);
        lowered.creation_draft = self.creation_draft.clone().or_else(|| {
            if self.creation.is_some() {
                return None;
            }
            let draft = self.phenomena.as_ref()?.editor.clone()?;
            let mut source = self.clone();
            let p = source.phenomena.as_mut().unwrap();
            p.fields = draft;
            p.editor = None;
            Some(CreationGraph::from_scene(&source))
        });
        Ok(lowered)
    }
    pub fn unify(&mut self) -> Result<(), SceneError> {
        *self = self.render_scene()?;
        Ok(())
    }
}

impl crate::Params for CreationGraph {
    fn visit_params(&mut self, prefix: &str, f: &mut dyn FnMut(&str, &mut f32, crate::Bounds)) {
        use crate::{params::join, Bounds};
        f(
            &join(prefix, "safety"),
            &mut self.safety,
            Bounds::new(0.01, 1.0, &[]),
        );
        f(
            &join(prefix, "bounding_radius"),
            &mut self.bounding_radius,
            Bounds::new(0.01, 10000.0, &[]),
        );
        self.settings.visit_params(&join(prefix, "settings"), f);
        for n in &mut self.nodes {
            let path = join(prefix, &format!("nodes[{}]", n.id));
            match &mut n.kind {
                CreationKind::Form(k) => {
                    let mut g = Graph::new_minimal();
                    g.nodes = vec![Node {
                        id: 0,
                        kind: k.clone(),
                        name: String::new(),
                        pos: [0.0; 2],
                        enabled: true,
                    }];
                    g.visit_params("", &mut |p, v, b| {
                        if let Some(suffix) = p.strip_prefix("nodes[0].") {
                            f(&join(&path, suffix), v, b)
                        }
                    });
                    *k = g.nodes.remove(0).kind;
                }
                CreationKind::Pattern { fractal, channels } => {
                    fractal.visit_params(&join(&path, "pattern"), f);
                    channels.visit_params(&join(&path, "channels"), f);
                }
                CreationKind::Texture(t) => t.visit_params(&join(&path, "texture"), f),
                CreationKind::Palette(r) => r.visit_params(&join(&path, "palette"), f),
                CreationKind::Flame(s) => s.visit_params(&path, f),
                CreationKind::Simulation(s) => {
                    let mut p = Phenomena::default();
                    p.simulations = vec![s.clone()];
                    p.visit_params("", &mut |p, v, b| {
                        if let Some(suffix) = p.strip_prefix("simulations[0].") {
                            f(&join(&path, suffix), v, b)
                        }
                    });
                    *s = p.simulations.remove(0);
                }
                CreationKind::SampleState { channel } => crate::params::visit_u32(
                    &join(&path, "channel"),
                    channel,
                    Bounds::new(0.0, 3.0, &[]),
                    f,
                ),
                CreationKind::Field(op) => {
                    let mut p = Phenomena::default();
                    p.fields.nodes = vec![FieldNode {
                        id: 0,
                        name: String::new(),
                        op: op.clone(),
                        inputs: vec![],
                        pos: [0.0; 2],
                    }];
                    p.visit_params("", &mut |p, v, b| {
                        if let Some(suffix) = p.strip_prefix("fields.nodes[0].") {
                            f(&join(&path, suffix), v, b)
                        }
                    });
                    *op = p.fields.nodes.remove(0).op;
                }
                _ => {}
            }
        }
    }
}
