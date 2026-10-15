//! Compiles a form graph into the WGSL `de_sample` function, a twin of the
//! CPU interpreter (`syfrack-core/src/scene/graph/eval.rs`): same operations,
//! same order. Scalar parameters are read from `graph_params` (binding 4),
//! so value changes never recompile; see `ParamLayout`.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::fmt::Write;
use std::hash::{Hash, Hasher};
use syfrack_core::graph::MAX_DEPTH;
use syfrack_core::{Graph, Measure, NodeId, NodeKind, Scene, SceneError, TrapMode};

/// Scalars in the parameter buffer, including reserved trap scales.
pub const PARAM_COUNT: usize = syfrack_core::graph::MAX_SCALARS;
/// The last four scalars hold the normalisation scale of each trap slot.
pub const TRAP_SCALE_BASE: usize = PARAM_COUNT - 4;

fn union_leaves(g: &Graph, id: NodeId, out: &mut Vec<NodeId>) {
    let n = g.node(id).unwrap();
    if n.enabled && matches!(n.kind, NodeKind::Union) {
        union_leaves(g, g.input_of(id, 0).unwrap().0, out);
        union_leaves(g, g.input_of(id, 1).unwrap().0, out);
    } else if !out.contains(&id) {
        out.push(id);
    }
}

/// Where each scalar parameter lives in the buffer; fixed by the topology.
#[derive(Clone, Debug, Default)]
pub struct ParamLayout {
    /// (node, field, first index, scalar count)
    slots: Vec<(NodeId, &'static str, usize, usize)>,
}

impl ParamLayout {
    fn add(&mut self, node: NodeId, field: &'static str, count: usize) -> usize {
        let index = self.len();
        self.slots.push((node, field, index, count));
        index
    }

    fn placements(&mut self, node: NodeId, count: usize) -> usize {
        if let Some(index) = self.index_of(node, "placements") {
            return index;
        }
        let pad = (4 - self.len() % 4) % 4;
        if pad != 0 {
            self.add(node, "alignment", pad);
        }
        self.add(node, "placements", count * 12)
    }

    pub fn index_of(&self, node: NodeId, field: &str) -> Option<usize> {
        self.slots
            .iter()
            .find(|s| s.0 == node && s.1 == field)
            .map(|s| s.2)
    }

    /// Scalars used, trap scales excluded.
    pub fn len(&self) -> usize {
        self.slots.iter().map(|s| s.2 + s.3).max().unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// The current values of `g` in layout order; trap scales at
    /// `TRAP_SCALE_BASE..`, from `Graph::trap_scales` (1.0 for an unwritten slot).
    pub fn pack(&self, g: &Graph) -> [f32; PARAM_COUNT] {
        let mut out = [0.0f32; PARAM_COUNT];
        for (id, field, index, _) in &self.slots {
            let Some(n) = g.node(*id) else { continue };
            use NodeKind::*;
            let vals: Vec<f32> = match (&n.kind, *field) {
                (Output, "union_count") => {
                    let mut leaves = Vec::new();
                    union_leaves(g, g.input_of(*id, 0).unwrap().0, &mut leaves);
                    vec![leaves.len() as f32]
                }
                (Instances { placements, .. }, "placement_count") => {
                    vec![placements.len() as f32]
                }
                (
                    Instances {
                        placements, half, ..
                    },
                    "cluster_bounds",
                ) => placements
                    .chunks(8)
                    .flat_map(|chunk| {
                        let mut lo = glam::Vec3::splat(f32::INFINITY);
                        let mut hi = glam::Vec3::splat(f32::NEG_INFINITY);
                        for p in chunk {
                            let a = p.angles * (std::f32::consts::PI / 180.0);
                            let r = glam::Quat::from_euler(glam::EulerRot::XYZ, a.x, a.y, a.z);
                            let h = *half * p.scale;
                            let e = (r * glam::Vec3::X).abs() * h.x
                                + (r * glam::Vec3::Y).abs() * h.y
                                + (r * glam::Vec3::Z).abs() * h.z;
                            lo = lo.min(p.position - e);
                            hi = hi.max(p.position + e);
                        }
                        let centre = (lo + hi) * 0.5;
                        let extent = (hi - lo) * 0.5 + glam::Vec3::splat(0.002);
                        centre
                            .to_array()
                            .into_iter()
                            .chain(extent.to_array())
                            .collect::<Vec<_>>()
                    })
                    .collect(),
                (
                    Instances {
                        placements, half, ..
                    },
                    "envelope_center" | "envelope_half",
                ) => {
                    let mut lo = glam::Vec3::splat(f32::INFINITY);
                    let mut hi = glam::Vec3::splat(f32::NEG_INFINITY);
                    for p in placements {
                        let a = p.angles * (std::f32::consts::PI / 180.0);
                        let r = glam::Quat::from_euler(glam::EulerRot::XYZ, a.x, a.y, a.z);
                        let h = *half * p.scale;
                        let e = (r * glam::Vec3::X).abs() * h.x
                            + (r * glam::Vec3::Y).abs() * h.y
                            + (r * glam::Vec3::Z).abs() * h.z;
                        lo = lo.min(p.position - e);
                        hi = hi.max(p.position + e);
                    }
                    (if *field == "envelope_center" {
                        (lo + hi) * 0.5
                    } else {
                        (hi - lo) * 0.5 + glam::Vec3::splat(0.002)
                    })
                    .to_array()
                    .to_vec()
                }
                (Translate { offset }, "offset") | (MengerFold { offset, .. }, "offset") => {
                    offset.to_array().to_vec()
                }
                (Rotate { angles }, "angles") => angles.to_array().to_vec(),
                (Scale { factor }, "factor") => vec![*factor],
                (ScaleAxes { factors }, "factors") => factors.to_array().to_vec(),
                (Mirror { normal, .. }, "normal") | (Plane { normal, .. }, "normal") => {
                    normal.to_array().to_vec()
                }
                (Mirror { offset, .. }, "offset") | (Plane { offset, .. }, "offset") => {
                    vec![*offset]
                }
                (BoxFold { limit }, "limit") => vec![*limit],
                (BoxFoldAxes { limit, .. }, "limit") => limit.to_array().to_vec(),
                (SphereInversion { radius, .. }, "radius") => vec![*radius],
                (SphereInversion { min_radius, .. }, "min_radius") => vec![*min_radius],
                (AddConstant { multiplier, .. } | AddSeed { multiplier }, "multiplier") => {
                    multiplier.to_array().to_vec()
                }
                (AddConstant { julia, .. }, "julia") => julia.to_array().to_vec(),
                (SphereFold { min_radius, .. }, "min_radius") => vec![*min_radius],
                (SphereFold { fixed_radius, .. }, "fixed_radius") => vec![*fixed_radius],
                (MengerFold { scale, .. }, "scale") => vec![*scale],
                (Kaleido { phase, .. }, "phase") => vec![*phase],
                (Power { power }, "power") => vec![*power],
                (Twist { rate }, "rate") | (Bend { rate }, "rate") => vec![*rate],
                (Tile { period } | RepeatFinite { period, .. }, "period") => {
                    period.to_array().to_vec()
                }
                (RepeatFinite { count, .. }, "count") => count.iter().map(|n| *n as f32).collect(),
                (Iterate { bailout, .. }, "bailout") => vec![*bailout],
                (Iterate { iterations, .. }, "iterations") => vec![*iterations as f32],
                (
                    Sphere { radius }
                    | RoundBox { radius, .. }
                    | Torus { radius, .. }
                    | Cylinder { radius, .. },
                    "radius",
                ) => vec![*radius],
                (Torus { tube, .. }, "tube") => vec![*tube],
                (Cylinder { half_height, .. }, "half_height") => vec![*half_height],
                (Rock { erosion, .. }, "erosion") => vec![*erosion],
                (Rock { seed, .. }, "seed") => vec![*seed as f32],
                (MaterialTag { index }, "index") => vec![*index as f32],
                (Instances { placements, .. }, "placements") => placements
                    .iter()
                    .flat_map(|p| {
                        let a = p.angles * (std::f32::consts::PI / 180.0);
                        let r = glam::Quat::from_euler(glam::EulerRot::XYZ, a.x, a.y, a.z);
                        let mut packed = Vec::with_capacity(12);
                        for (i, axis) in [glam::Vec3::X, glam::Vec3::Y, glam::Vec3::Z]
                            .into_iter()
                            .enumerate()
                        {
                            let row = (r * axis) / p.scale[i];
                            packed.extend_from_slice(&[row.x, row.y, row.z, -row.dot(p.position)]);
                        }

                        packed
                    })
                    .collect(),
                (Bounded { margin, .. } | Instances { margin, .. }, "margin") => vec![*margin],
                (
                    Box { half }
                    | RoundBox { half, .. }
                    | Rock { half, .. }
                    | Bounded { half, .. }
                    | Instances { half, .. },
                    "half",
                ) => half.to_array().to_vec(),
                (NormFence { inset }, "inset") => vec![*inset],
                (SmoothUnion { k }, "k") | (SmoothIntersection { k }, "k") => vec![*k],
                _ => Vec::new(),
            };
            for (j, v) in vals.iter().enumerate() {
                if index + j < TRAP_SCALE_BASE {
                    out[index + j] = *v;
                }
            }
        }
        out[TRAP_SCALE_BASE..].copy_from_slice(&g.trap_scales());
        out
    }
}

/// A compiled graph: the WGSL `de_sample` function, its parameter layout and
/// its topology fingerprint.
pub struct Compiled {
    pub wgsl: String,
    pub layout: ParamLayout,
    pub fingerprint: u64,
}

fn hash_node(g: &Graph, id: NodeId, depth: u32, h: &mut DefaultHasher) {
    // Bounded like the interpreter: a self-nesting (invalid) body cannot recurse forever.
    if depth > MAX_DEPTH + 1 {
        return;
    }
    if let Some(n) = g.node(id) {
        id.hash(h);
        n.enabled.hash(h);
        n.kind.topology_key().hash(h);
        for b in g.body_ids_of(id) {
            hash_node(g, *b, depth + 1, h);
        }
    }
}

/// Hash of everything that changes the generated shader: active nodes (kind
/// without scalar values, enabled flag, bodies) and the links between them.
pub fn fingerprint(g: &Graph) -> u64 {
    let mut h = DefaultHasher::new();
    "independent placements and bounded evaluation ABI 6".hash(&mut h);
    let active = g.active_nodes();
    for id in &active {
        hash_node(g, *id, 0, &mut h);
    }
    let mut links: Vec<_> = g
        .links
        .iter()
        .filter(|l| active.contains(&l.from.0) && active.contains(&l.to.0))
        .collect();
    links.sort_by_key(|l| (l.to, l.from));
    for l in links {
        l.from.hash(&mut h);
        l.to.hash(&mut h);
    }
    h.finish()
}

/// A scene pipeline depends on both graphs, but never on their scalar values.
pub fn scene_fingerprint(scene: &Scene) -> u64 {
    optics_fingerprint(
        nebula_fingerprint(
            pipeline_fingerprint(
                fingerprint(scene.fractal.graph()),
                scene
                    .material
                    .texture
                    .as_ref()
                    .map(|t| fingerprint(&t.graph)),
            ),
            scene.atmosphere.nebula.is_some(),
        ),
        scene_has_optics(scene),
    )
}

pub fn scene_has_optics(scene: &Scene) -> bool {
    scene.material.optics.is_some() || scene.material.finishes.iter().any(|f| f.optics.is_some())
}

pub fn optics_fingerprint(pipeline: u64, enabled: bool) -> u64 {
    if !enabled {
        return pipeline;
    }
    let mut h = DefaultHasher::new();
    "dielectric transmission and interior medium 1".hash(&mut h);
    pipeline.hash(&mut h);
    h.finish()
}

/// Enabling the medium changes compiled code; its numeric values are uniforms.
pub fn nebula_fingerprint(pipeline: u64, enabled: bool) -> u64 {
    if !enabled {
        return pipeline;
    }
    let mut h = DefaultHasher::new();
    "bounded nebula medium 1".hash(&mut h);
    pipeline.hash(&mut h);
    h.finish()
}

pub fn pipeline_fingerprint(form: u64, texture: Option<u64>) -> u64 {
    let Some(texture) = texture else { return form };
    let mut h = DefaultHasher::new();
    "fractal texture".hash(&mut h);
    form.hash(&mut h);
    texture.hash(&mut h);
    h.finish()
}

struct Emitter<'a> {
    g: &'a Graph,
    function: String,
    params: &'static str,
    out: String,
    layout: ParamLayout,
    /// `// node <id> <name>` of the node being emitted, appended to every
    /// statement line so that a naga error quoting the line names the node.
    node: Option<String>,
    seed: Option<(String, String)>,
    /// Union candidate probes need conservative bounds, not detailed surfaces.
    bounds_only: bool,
}

fn comment(id: NodeId, name: &str) -> String {
    format!("// node {id} {}", name.replace(['\n', '\r'], " "))
}

impl Emitter<'_> {
    fn gp(&mut self, node: NodeId, field: &'static str) -> String {
        let i = self
            .layout
            .index_of(node, field)
            .unwrap_or_else(|| self.layout.add(node, field, 1));
        format!("{}({i}u)", self.params)
    }

    fn gp3(&mut self, node: NodeId, field: &'static str) -> String {
        let i = self
            .layout
            .index_of(node, field)
            .unwrap_or_else(|| self.layout.add(node, field, 3));
        let gp = self.params;
        format!("vec3f({gp}({}u), {gp}({}u), {gp}({}u))", i, i + 1, i + 2)
    }

    fn line(&mut self, s: &str) {
        match &self.node {
            Some(c) => {
                let _ = writeln!(self.out, "    {s} {c}");
            }
            None => {
                let _ = writeln!(self.out, "    {s}");
            }
        }
    }

    /// A line holding only the node comment (a disabled node emits nothing else).
    fn comment_line(&mut self, id: NodeId, name: &str) {
        let _ = writeln!(self.out, "    {} (désactivé)", comment(id, name));
    }

    fn input(&self, id: NodeId, port: u32) -> Result<NodeId, SceneError> {
        self.g
            .input_of(id, port)
            .map(|(from, _)| from)
            .ok_or_else(|| {
                SceneError::Graph(format!("port d'entrée {port} du nœud {id} non relié"))
            })
    }

    /// Point → Point effect of `id` on the variables `{v}_p`, `{v}_dr`, `{v}_it`, `{v}_tr`.
    fn point_op(&mut self, id: NodeId, v: &str) -> Result<(), SceneError> {
        let n = self
            .g
            .node(id)
            .ok_or_else(|| SceneError::Graph(format!("nœud inconnu {id}")))?;
        if !n.enabled {
            self.comment_line(id, &n.name);
            return Ok(());
        }
        let outer = self.node.replace(comment(id, &n.name));
        let r = self.point_body(id, v);
        self.node = outer;
        r
    }

    fn point_body(&mut self, id: NodeId, v: &str) -> Result<(), SceneError> {
        let n = self
            .g
            .node(id)
            .ok_or_else(|| SceneError::Graph(format!("nœud inconnu {id}")))?;
        use NodeKind::*;
        let (p, dr, tr) = (format!("{v}_p"), format!("{v}_dr"), format!("{v}_tr"));
        let kind = n.kind.clone();
        match &kind {
            Translate { .. } => {
                let o = self.gp3(id, "offset");
                self.line(&format!("{p} = {p} + {o};"));
            }
            Rotate { .. } => {
                let a = self.gp3(id, "angles");
                self.line(&format!("{p} = rotate_euler_xyz({p}, radians({a}));"));
            }
            Scale { .. } => {
                let f = self.gp(id, "factor");
                self.line(&format!("{p} = {p} * {f}; {dr} = {dr} * abs({f});"));
            }
            ScaleAxes { .. } => {
                let f = self.gp3(id, "factors");
                self.line(&format!(
                    "{p} = {p} * {f}; {dr} = {dr} * max(max({f}.x, {f}.y), {f}.z);"
                ));
            }
            Mirror { .. } => {
                let nrm = self.gp3(id, "normal");
                let o = self.gp(id, "offset");
                self.line(&format!(
                    "{{ let nn = safe_normalize({nrm}); let h = dot({p}, nn) - {o}; \
                     if (h < 0.0) {{ {p} = {p} - 2.0 * h * nn; }} }}"
                ));
            }
            BoxFold { .. } => {
                let l = self.gp(id, "limit");
                self.line(&format!(
                    "{p} = clamp({p}, vec3f(-{l}), vec3f({l})) * 2.0 - {p};"
                ));
            }
            SphereFold { .. } => {
                let mr = self.gp(id, "min_radius");
                let fr = self.gp(id, "fixed_radius");
                self.line(&format!(
                    "{{ let r2 = dot({p}, {p}); let mr2 = {mr} * {mr}; let fr2 = {fr} * {fr}; \
                     var k = 1.0; if (r2 < mr2) {{ k = fr2 / mr2; }} else if (r2 < fr2) {{ k = fr2 / r2; }} \
                     {p} = {p} * k; {dr} = {dr} * k; }}"
                ));
            }
            BoxFoldAxes { x, y, z, .. } => {
                let l = self.gp3(id, "limit");
                self.line(&format!(
                    "{{ let folded = clamp({p}, -{l}, {l}) * 2.0 - {p};"
                ));
                for (on, axis) in [(*x, 'x'), (*y, 'y'), (*z, 'z')] {
                    if on {
                        self.line(&format!("{p}.{axis} = folded.{axis};"));
                    }
                }
                self.line("}");
            }
            SphereInversion { .. } => {
                let r = self.gp(id, "radius");
                let m = self.gp(id, "min_radius");
                self.line(&format!(
                    "{{ let k = {r} * {r} / max(dot({p}, {p}), {m} * {m}); {p} *= k; {dr} *= k; }}"
                ));
            }
            AddConstant { .. } => {
                let m = self.gp3(id, "multiplier");
                let j = self.gp3(id, "julia");
                self.line(&format!("{{ let m = {m}; {p} += c * m + {j}; {dr} += max(max(abs(m.x), abs(m.y)), abs(m.z)); }}"));
            }
            Abs { x, y, z } => {
                for (on, axis) in [(*x, 'x'), (*y, 'y'), (*z, 'z')] {
                    if on {
                        self.line(&format!("{p}.{axis} = abs({p}.{axis});"));
                    }
                }
            }
            MengerFold { .. } => {
                let s = self.gp(id, "scale");
                let o = self.gp3(id, "offset");
                self.line(&format!(
                    "{{ var z = abs({p}); if (z.x < z.y) {{ z = vec3f(z.y, z.x, z.z); }} \
                     if (z.x < z.z) {{ z = vec3f(z.z, z.y, z.x); }} if (z.y < z.z) {{ z = vec3f(z.x, z.z, z.y); }} \
                     let off = {o}; z = z * {s} - off * ({s} - 1.0); \
                     if (z.z < -0.5 * off.z * ({s} - 1.0)) {{ z.z += off.z * ({s} - 1.0); }} \
                     {p} = z; {dr} = {dr} * {s}; }}"
                ));
            }
            Kaleido { planes, .. } => {
                let ph = self.gp(id, "phase");
                let planes = (*planes).max(1);
                self.line(&format!(
                    "{{ let sector = 6.2831855 / {planes}.0; var a = atan2({p}.z, {p}.x) + radians({ph}); \
                     a = abs(modf_euclid(a + sector * 0.5, sector) - sector * 0.5); \
                     let r = length({p}.xz); {p}.x = r * cos(a); {p}.z = r * sin(a); }}"
                ));
            }
            Power { power } if *power == 6.0 || *power == 8.0 => {
                let pw = self.gp(id, "power");
                self.line(&format!("{{ let radius=length({p}); if (radius>=1e-12) {{ let powered=bulb_power68({p},radius,{pw}); {p}=powered.xyz; {dr}*=powered.w; }} else {{ {p}=vec3f(0.0); {dr}=0.0; }} }}"));
            }
            Power { .. } => {
                let pw = self.gp(id, "power");
                self.line(&format!(
                    "{{ let radius=length({p}); let r=max(radius,1e-12); \
                     var theta=acos(clamp({p}.z/r,-1.0,1.0)); if (radius>=1e-12) {{ theta=atan2(length({p}.xy),{p}.z); }} \
                     let phi = atan2({p}.y, {p}.x); {dr} = {dr} * (pow(r, {pw} - 1.0) * {pw}); \
                     let zr = pow(r, {pw}); let st = sin(theta * {pw}); let ct = cos(theta * {pw}); \
                     let sp = sin(phi * {pw}); let cp = cos(phi * {pw}); {p} = vec3f(st * cp, st * sp, ct) * zr; }}"
                ));
            }
            AddOrigin => self.line(&format!("{p} = {p} + c; {dr} = {dr} + 1.0;")),
            AddSeed { .. } => {
                let m = self.gp3(id, "multiplier");
                let (origin, derivative) =
                    self.seed.clone().unwrap_or_else(|| (p.clone(), dr.clone()));
                self.line(&format!("{p} = {p} + {origin} * {m}; {dr} = {dr} + {derivative} * max(max(abs({m}.x), abs({m}.y)), abs({m}.z));"));
            }
            Twist { .. } => {
                let r = self.gp(id, "rate");
                self.line(&format!(
                    "{{ let v = rot2({p}.xz, {r} * {p}.y); {p}.x = v.x; {p}.z = v.y; }}"
                ));
            }
            Bend { .. } => {
                let r = self.gp(id, "rate");
                self.line(&format!(
                    "{{ let v = rot2({p}.xy, {r} * {p}.x); {p}.x = v.x; {p}.y = v.y; }}"
                ));
            }
            Tile { .. } => {
                let per = self.gp3(id, "period");
                self.line(&format!(
                    "{{ let per = {per}; let half = per * 0.5; {p} = modf_euclid3({p} + half, per) - half; }}"
                ));
            }
            RepeatFinite { .. } => {
                let per = self.gp3(id, "period");
                let count = self.gp3(id, "count");
                self.line(&format!("{{ let per = {per}; let last = {count} - vec3f(1.0); let half = last * 0.5; let cell = clamp(floor({p} / per + half + vec3f(0.5)), vec3f(0.0), last) - half; {p} -= cell * per; }}"));
            }
            Iterate { body, .. } => {
                let b = self.gp(id, "bailout");
                let count = self.gp(id, "iterations");
                self.line(&format!("let seed{id} = {p}; let seed{id}_dr = {dr};"));
                let outer_seed = self
                    .seed
                    .replace((format!("seed{id}"), format!("seed{id}_dr")));
                self.line(&format!(
                    "for (var i{id} = 0u; i{id} < u32({count}); i{id}++) {{"
                ));
                self.line(&format!("    if (length({p}) > {b}) {{ break; }}"));
                for child in body {
                    self.point_op(*child, v)?;
                }
                // point_op restored this node's comment for the closing lines.
                self.line(&format!("    {v}_it = {v}_it + 1u;"));
                self.line("}");
                self.seed = outer_seed;
            }
            Trap {
                slot,
                measure,
                mode,
                ..
            } => {
                let m = match measure {
                    Measure::Norm => format!("length({p})"),
                    // Twin of eval.rs `measure`.
                    Measure::AxisX => format!("length({p}.yz)"),
                    Measure::AxisY => format!("length({p}.xz)"),
                    Measure::AxisZ => format!("length({p}.xy)"),
                    Measure::PlaneY => format!("abs({p}.y)"),
                    Measure::AngleXY => format!("orbit_angle({p}.xy)"),
                    Measure::AngleXZ => format!("orbit_angle({p}.xz)"),
                    Measure::AngleYZ => format!("orbit_angle({p}.yz)"),
                    Measure::LogNorm => format!("log(1.0+length({p}))"),
                };
                let s = (*slot).min(3);
                let expr = match mode {
                    TrapMode::Minimum => format!("min({tr}[{s}], {m})"),
                    TrapMode::Maximum => {
                        format!("select(max({tr}[{s}], {m}), {m}, {tr}[{s}] >= 1e30)")
                    }
                    TrapMode::Last => m,
                };
                self.line(&format!("{tr}[{s}] = {expr};"));
            }
            other => {
                return Err(SceneError::Graph(format!(
                    "nœud {id} « {} » n'est pas Point → Point",
                    other.label()
                )))
            }
        }
        Ok(())
    }

    fn fence(&mut self, id: NodeId, kind: &NodeKind, src: NodeId) -> String {
        use NodeKind::*;
        match kind {
            Sphere { .. } => {
                let r = self.gp(id, "radius");
                format!("(length(p{src}_p) - {r}) / p{src}_dr")
            }
            Torus { .. } => {
                let r = self.gp(id, "radius");
                let tube = self.gp(id, "tube");
                format!(
                    "(length(vec2f(length(p{src}_p.xz) - {r}, p{src}_p.y)) - {tube}) / p{src}_dr"
                )
            }
            Cylinder { .. } => {
                let r = self.gp(id, "radius");
                let h = self.gp(id, "half_height");
                format!("cylinder_distance(p{src}_p, {r}, {h}) / p{src}_dr")
            }
            Box { .. } => {
                let h = self.gp3(id, "half");
                format!("box_distance(p{src}_p, {h}) / p{src}_dr")
            }
            RoundBox { .. } => {
                let h = self.gp3(id, "half");
                let r = self.gp(id, "radius");
                format!("round_box_distance(p{src}_p, {h}, {r}) / p{src}_dr")
            }
            Rock { .. } => {
                let h = self.gp3(id, "half");
                let e = self.gp(id, "erosion");
                let seed = self.gp(id, "seed");
                format!("rock_distance(p{src}_p, {h}, {e}, u32({seed})) / p{src}_dr")
            }
            Plane { .. } => {
                let nrm = self.gp3(id, "normal");
                let o = self.gp(id, "offset");
                format!("(dot(p{src}_p, safe_normalize({nrm})) - {o}) / p{src}_dr")
            }
            NormFence { .. } => {
                let i = self.gp(id, "inset");
                format!("(length(p{src}_p) - {i}) / abs(p{src}_dr)")
            }
            BulbFence => format!("bulb_distance(p{src}_p, p{src}_dr)"),
            _ => "1e30".to_string(),
        }
    }

    fn combine(&mut self, id: NodeId, kind: &NodeKind, a: NodeId, b: NodeId) -> String {
        use NodeKind::*;
        match kind {
            Union => format!(
                "let d{id} = min(d{a}, d{b}); let d{id}_it = select(d{b}_it, d{a}_it, d{a} <= d{b}); \
                 let d{id}_tr = select(d{b}_tr, d{a}_tr, d{a} <= d{b});"
            ),
            Intersection => format!(
                "let d{id} = max(d{a}, d{b}); let d{id}_it = select(d{b}_it, d{a}_it, d{a} >= d{b}); \
                 let d{id}_tr = select(d{b}_tr, d{a}_tr, d{a} >= d{b});"
            ),
            Subtraction => format!(
                "let d{id} = max(d{a}, -d{b}); let d{id}_it = d{a}_it; let d{id}_tr = d{a}_tr;"
            ),
            // An empty operand (>= 1e30: a disabled fence) overrides the blend,
            // as in Graph::evaluate: for a union the other side wins, distance
            // and payload; an intersection becomes empty with the other side's
            // payload.
            SmoothUnion { .. } => {
                let k = self.gp(id, "k");
                format!(
                    "let h{id} = clamp(0.5 + 0.5 * (d{b} - d{a}) / {k}, 0.0, 1.0); \
                     var d{id} = d{b} + (d{a} - d{b}) * h{id} - {k} * h{id} * (1.0 - h{id}); \
                     var d{id}_it = select(d{b}_it, d{a}_it, h{id} >= 0.5); \
                     var d{id}_tr = blend_traps(d{a}_tr, d{b}_tr, h{id}); \
                     if (d{a} >= 1e30) {{ d{id} = d{b}; d{id}_it = d{b}_it; d{id}_tr = d{b}_tr; }} \
                     else if (d{b} >= 1e30) {{ d{id} = d{a}; d{id}_it = d{a}_it; d{id}_tr = d{a}_tr; }}"
                )
            }
            SmoothIntersection { .. } => {
                let k = self.gp(id, "k");
                format!(
                    "let h{id} = clamp(0.5 - 0.5 * (d{b} - d{a}) / {k}, 0.0, 1.0); \
                     var d{id} = d{b} + (d{a} - d{b}) * h{id} + {k} * h{id} * (1.0 - h{id}); \
                     var d{id}_it = select(d{b}_it, d{a}_it, h{id} >= 0.5); \
                     var d{id}_tr = blend_traps(d{a}_tr, d{b}_tr, h{id}); \
                     if (d{a} >= 1e30) {{ d{id} = 1e30; d{id}_it = d{b}_it; d{id}_tr = d{b}_tr; }} \
                     else if (d{b} >= 1e30) {{ d{id} = 1e30; d{id}_it = d{a}_it; d{id}_tr = d{a}_tr; }}"
                )
            }
            // Not a combination: pass the first input through (unreachable after validation).
            _ => format!("let d{id} = d{a}; let d{id}_it = d{a}_it; let d{id}_tr = d{a}_tr;"),
        }
    }

    fn emit(
        mut self,
        target: NodeId,
        fast: &HashSet<NodeId>,
        normalise: bool,
    ) -> Result<Compiled, SceneError> {
        let g = self.g;
        g.validate()?;
        let order = g.topo_order()?;
        let mut needed = HashSet::new();
        needed_nodes(g, target, fast, &mut needed);
        let _ = writeln!(self.out, "fn {}(p: vec3f) -> DeSample {{", self.function);
        self.line("let c = p;");
        let mut out_var: Option<String> = None;
        let mut out_comment = None;
        for id in order {
            if !needed.contains(&id) {
                continue;
            }
            let n = g
                .node(id)
                .ok_or_else(|| SceneError::Graph(format!("nœud inconnu {id}")))?;
            self.node = Some(comment(id, &n.name));
            match &n.kind {
                NodeKind::Input => {
                    self.line(&format!(
                        "var p{id}_p = c; var p{id}_dr = 1.0; var p{id}_it = 0u; var p{id}_tr = vec4f(1e30);"
                    ));
                }
                NodeKind::Output => {
                    out_var = Some(format!("d{}", self.input(id, 0)?));
                    out_comment = self.node.clone();
                }
                k if k.is_point_to_point() => {
                    let src = self.input(id, 0)?;
                    self.line(&format!(
                        "var p{id}_p = p{src}_p; var p{id}_dr = p{src}_dr; var p{id}_it = p{src}_it; var p{id}_tr = p{src}_tr;"
                    ));
                    self.point_op(id, &format!("p{id}"))?;
                }
                k if k.is_fence() => {
                    let src = self.input(id, 0)?;
                    // A disabled fence is empty space.
                    let expr = if n.enabled {
                        self.fence(id, k, src)
                    } else {
                        "1e30".to_string()
                    };
                    self.line(&format!(
                        "let d{id} = {expr}; let d{id}_it = p{src}_it; let d{id}_tr = p{src}_tr;"
                    ));
                    self.line(&format!("let d{id}_mat = 0u; let d{id}_flags = 0u;"));
                }
                NodeKind::MaterialTag { .. } => {
                    let a = self.input(id, 0)?;
                    let mat = if n.enabled {
                        format!("u32({})", self.gp(id, "index"))
                    } else {
                        format!("d{a}_mat")
                    };
                    self.line(&format!("let d{id} = d{a}; let d{id}_it = d{a}_it; let d{id}_tr = d{a}_tr; let d{id}_mat = {mat}; let d{id}_flags = d{a}_flags;"));
                }
                NodeKind::Instances { placements, .. } => {
                    if !n.enabled {
                        let a = self.input(id, 0)?;
                        self.line(&format!("let d{id} = d{a}; let d{id}_it = d{a}_it; let d{id}_tr = d{a}_tr; let d{id}_mat = d{a}_mat; let d{id}_flags = d{a}_flags;"));
                    } else {
                        let base = self.layout.placements(id, placements.len());
                        let clusters =
                            self.layout
                                .index_of(id, "cluster_bounds")
                                .unwrap_or_else(|| {
                                    self.layout.add(
                                        id,
                                        "cluster_bounds",
                                        placements.len().div_ceil(8) * 6,
                                    )
                                });
                        let half = self.gp3(id, "half");
                        let gp = self.params;
                        // Read loop bounds from the parameter buffer. Some older
                        // GPU compilers unroll constant loops containing a full
                        // fractal evaluator, producing enormous shader programs.
                        let count_param = self.gp(id, "placement_count");
                        let count = format!("count{id}");
                        self.line(&format!("let {count}=u32({count_param});"));
                        let template = self.input(id, 0)?;
                        self.line(&format!("var best{id} = DeSample(1e30,vec4f(1e30),0u,0u,0u); var chosen{id} = {count};"));
                        if fast.contains(&id) {
                            let margin = self.gp(id, "margin");
                            let centre = self.gp3(id, "envelope_center");
                            let extent = self.gp3(id, "envelope_half");
                            if self.bounds_only {
                                self.line(&format!("best{id}=DeSample(box_distance(c-{centre},{extent}),vec4f(1e30),0u,0u,1u);"));
                            } else {
                                self.line(&format!("let envelope{id}=box_distance(c-{centre},{extent}); if (envelope{id}>{margin}) {{ best{id}=DeSample(envelope{id},vec4f(1e30),0u,0u,1u); }} else {{"));
                                self.line(&format!("for (var j=0u; j<{count}; j++) {{ if (j%8u==0u) {{ let cluster={gp}_cluster_bound(c,{clusters}u+(j/8u)*6u); if (cluster>best{id}.distance) {{ j+=7u; continue; }} if (cluster>{margin}) {{ if (cluster<best{id}.distance || (cluster==best{id}.distance && j<chosen{id})) {{ best{id}=DeSample(cluster,vec4f(1e30),0u,0u,1u); chosen{id}=j; }} j+=7u; continue; }} }} let local={gp}_placement(c,{base}u+j*12u); let bound=box_distance(local.p.xyz*local.s,{half}*local.s); if (bound>best{id}.distance) {{ continue; }} var detail=DeSample(bound,vec4f(1e30),0u,0u,1u); if (bound<={margin}) {{ detail={gp}_instances_{template}_fast(local.p.xyz); detail.distance=max(detail.distance*local.p.w,bound); }} if (detail.distance<best{id}.distance || (detail.distance==best{id}.distance && j<chosen{id})) {{ best{id}=detail; chosen{id}=j; }} }} }}"));
                            }
                        } else {
                            // Seed with the closest box, then prune only using
                            // rigorous lower bounds. Overlapping copies remain exact.
                            self.line(&format!("var closest{id}=0u; var lower{id}=1e30; var nearestCluster{id}=0u; var clusterLower{id}=1e30; for (var k=0u; k<({count}+7u)/8u; k++) {{ let b={gp}_cluster_bound(c,{clusters}u+k*6u); if (b<clusterLower{id}) {{ clusterLower{id}=b; nearestCluster{id}=k; }} }} for (var j=nearestCluster{id}*8u; j<min((nearestCluster{id}+1u)*8u,{count}); j++) {{ let local={gp}_placement(c,{base}u+j*12u); let bound=box_distance(local.p.xyz*local.s,{half}*local.s); if (bound<lower{id}) {{ lower{id}=bound; closest{id}=j; }} }} for (var j=0u; j<{count}; j++) {{ if (j%8u==0u && {gp}_cluster_bound(c,{clusters}u+(j/8u)*6u)>lower{id}) {{ j+=7u; continue; }} let local={gp}_placement(c,{base}u+j*12u); let bound=box_distance(local.p.xyz*local.s,{half}*local.s); if (bound<lower{id} || (bound==lower{id} && j<closest{id})) {{ lower{id}=bound; closest{id}=j; }} }}"));
                            // Swap the nearest copy into the first iteration so
                            // the detailed evaluator appears only once in code.
                            self.line(&format!("for (var rank=0u; rank<{count}; rank++) {{ if (rank>0u && rank%8u==0u && {gp}_cluster_bound(c,{clusters}u+(rank/8u)*6u)>best{id}.distance) {{ rank+=7u; continue; }} var j=rank; if (rank==0u) {{ j=closest{id}; }} else if (rank==closest{id}) {{ j=0u; }} let local={gp}_placement(c,{base}u+j*12u); let bound=box_distance(local.p.xyz*local.s,{half}*local.s); if (bound>best{id}.distance) {{ continue; }} var detail={gp}_instances_{template}_exact(local.p.xyz); detail.distance=max(detail.distance*local.p.w,bound); if (detail.distance<best{id}.distance || (detail.distance==best{id}.distance && j<chosen{id})) {{ best{id}=detail; chosen{id}=j; }} }}"));
                        }
                        self.line(&format!("let d{id}=best{id}.distance; let d{id}_it=best{id}.iterations; let d{id}_tr=best{id}.traps; let d{id}_mat=best{id}.material; let d{id}_flags=best{id}.flags;"));
                    }
                }
                NodeKind::Bounded { .. } => {
                    let src = self.input(id, 0)?;
                    if !n.enabled {
                        let a = self.input(id, 1)?;
                        self.line(&format!("let d{id} = d{a}; let d{id}_it = d{a}_it; let d{id}_tr = d{a}_tr; let d{id}_mat = d{a}_mat; let d{id}_flags = d{a}_flags;"));
                    } else {
                        let half = self.gp3(id, "half");
                        self.line(&format!(
                            "let bound{id} = box_distance(p{src}_p, {half}) / p{src}_dr;"
                        ));
                        if fast.contains(&id) {
                            let margin = self.gp(id, "margin");
                            self.line(&format!(
                                "var detail{id} = DeSample(bound{id}, vec4f(1e30), 0u, 0u, 1u);"
                            ));
                            if !self.bounds_only {
                                self.line(&format!(
                                "if (bound{id} <= {margin}) {{ detail{id} = {}_bound_{id}(c); }}",
                                self.params
                            ));
                            }
                            self.line(&format!("let d{id} = max(bound{id}, detail{id}.distance); let d{id}_it = detail{id}.iterations; let d{id}_tr = detail{id}.traps; let d{id}_mat = detail{id}.material; let d{id}_flags = detail{id}.flags;"));
                        } else {
                            let a = self.input(id, 1)?;
                            self.line(&format!("let d{id} = max(bound{id}, d{a}); let d{id}_it = d{a}_it; let d{id}_tr = d{a}_tr; let d{id}_mat = d{a}_mat; let d{id}_flags = d{a}_flags;"));
                        }
                    }
                }
                k => {
                    let a = self.input(id, 0)?;
                    let code = if n.enabled {
                        let b = self.input(id, 1)?;
                        self.combine(id, k, a, b)
                    } else {
                        // A disabled combination passes its first input through.
                        format!("let d{id} = d{a}; let d{id}_it = d{a}_it; let d{id}_tr = d{a}_tr;")
                    };
                    self.line(&code);
                    let b = if n.enabled { self.input(id, 1)? } else { a };
                    let take = match k {
                        NodeKind::Union => format!("d{a} <= d{b}"),
                        NodeKind::Intersection => format!("d{a} >= d{b}"),
                        NodeKind::SmoothUnion { .. } | NodeKind::SmoothIntersection { .. }
                            if n.enabled =>
                        {
                            format!("h{id} >= 0.5")
                        }
                        _ => "true".into(),
                    };
                    self.line(&format!(
                        "var d{id}_mat = select(d{b}_mat, d{a}_mat, {take});"
                    ));
                    let flags = if matches!(k, NodeKind::Union) {
                        // If an exact branch beats a conservative lower bound,
                        // it also beats that branch's exact distance.
                        format!("select(d{b}_flags, d{a}_flags, d{a} <= d{b})")
                    } else {
                        format!("d{a}_flags | d{b}_flags")
                    };
                    self.line(&format!("let d{id}_flags = {flags};"));
                    if n.enabled
                        && matches!(
                            k,
                            NodeKind::SmoothUnion { .. } | NodeKind::SmoothIntersection { .. }
                        )
                    {
                        self.line(&format!("if (d{a} >= 1e30) {{ d{id}_mat = d{b}_mat; }} else if (d{b} >= 1e30) {{ d{id}_mat = d{a}_mat; }}"));
                    }
                }
            }
        }
        let out = out_var.unwrap_or_else(|| format!("d{target}"));
        self.node = out_comment;
        if normalise {
            let b = TRAP_SCALE_BASE;
            let gp = self.params;
            self.line(&format!(
                "let scales = vec4f({gp}({b}u), {gp}({}u), {gp}({}u), {gp}({}u));",
                b + 1,
                b + 2,
                b + 3
            ));
            self.line(&format!(
            "let tr = select(min({out}_tr / scales, vec4f(1.0)), vec4f(1.0), {out}_tr >= vec4f(1e30));"
        ));
            // Nested Repeat counts every level; clamp like Graph::evaluate.
            self.line(&format!(
                "return DeSample({out}, tr, min({out}_it, {}u), {out}_mat, {out}_flags);",
                g.iteration_product()
            ));
        } else {
            self.line(&format!(
                "return DeSample({out}, {out}_tr, {out}_it, {out}_mat, {out}_flags);"
            ));
        }
        let _ = writeln!(self.out, "}}");
        if self.layout.len() > TRAP_SCALE_BASE {
            return Err(SceneError::Graph(format!(
                "plus de {TRAP_SCALE_BASE} scalaires de paramètres"
            )));
        }
        Ok(Compiled {
            wgsl: self.out,
            layout: self.layout,
            fingerprint: fingerprint(g),
        })
    }
}

/// The `<id> <name>` of the generated node a shader error points at: from a
/// quoted source line carrying its `// node` comment, else from the first
/// `:LINE:COL` position of the error, walking back in `source` (the whole
/// module) to the nearest node comment inside `de_sample`.
pub fn node_of_error(text: &str, source: &str) -> Option<String> {
    let tail = |l: &str| {
        l.split("// node ")
            .nth(1)
            .map(|t| t.trim_end().trim_end_matches(" (désactivé)").to_string())
    };
    if let Some(n) = text.lines().find_map(tail) {
        return Some(n);
    }
    let line = error_line(text)?;
    let lines: Vec<&str> = source.lines().collect();
    for l in lines.get(..line)?.iter().rev() {
        if let Some(n) = tail(l) {
            return Some(n);
        }
        // Function boundaries sit at column 0: the position is outside de_sample.
        if l.starts_with("fn ") || l.starts_with('}') {
            return None;
        }
    }
    None
}

/// The LINE of the first `:LINE:COL` in an error text (naga/codespan format).
fn error_line(text: &str) -> Option<usize> {
    let b = text.as_bytes();
    for i in 0..b.len() {
        if b[i] != b':' {
            continue;
        }
        let start = i + 1;
        let mut j = start;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > start && b.get(j) == Some(&b':') && b.get(j + 1).is_some_and(u8::is_ascii_digit) {
            return text[start..j].parse().ok();
        }
    }
    None
}

/// Validates `g` and emits its `de_sample` function.
pub fn compile(g: &Graph) -> Result<Compiled, SceneError> {
    compile_functions(g, "de_sample", "gp")
}

/// Reuses the graph compiler with an independent function and parameter buffer.
pub fn compile_texture(g: &Graph) -> Result<Compiled, SceneError> {
    compile_functions(g, "texture_de_sample", "texture_gp")
}

fn needed_nodes(g: &Graph, id: NodeId, fast: &HashSet<NodeId>, needed: &mut HashSet<NodeId>) {
    if !needed.insert(id) {
        return;
    }
    if let Some(n) = g.node(id) {
        for port in 0..n.kind.inputs().len() as u32 {
            if (port == 1 && fast.contains(&id))
                || (n.enabled && matches!(n.kind, NodeKind::Instances { .. }) && port == 0)
            {
                continue;
            }
            if let Some((src, _)) = g.input_of(id, port) {
                needed_nodes(g, src, fast, needed);
            }
        }
    }
}

/// A lower bound is safe only through monotone CSG operands. Subtraction flips
/// the sign of its second input; any shared bound reached with a negative sign
/// is evaluated exactly in every context.
fn negative_bounds(
    g: &Graph,
    id: NodeId,
    negative: bool,
    visited: &mut HashSet<(NodeId, bool)>,
    unsafe_bounds: &mut HashSet<NodeId>,
) {
    if !visited.insert((id, negative)) {
        return;
    }
    let Some(n) = g.node(id) else {
        return;
    };
    if negative
        && matches!(
            n.kind,
            NodeKind::Bounded { .. } | NodeKind::Instances { .. }
        )
    {
        unsafe_bounds.insert(id);
    }
    for port in 0..n.kind.inputs().len() as u32 {
        let sign = negative ^ (n.enabled && matches!(n.kind, NodeKind::Subtraction) && port == 1);
        if let Some((src, _)) = g.input_of(id, port) {
            negative_bounds(g, src, sign, visited, unsafe_bounds);
        }
    }
}

fn compile_functions(
    g: &Graph,
    function: &str,
    params: &'static str,
) -> Result<Compiled, SceneError> {
    g.validate()?;
    let output = g
        .nodes
        .iter()
        .find(|n| matches!(n.kind, NodeKind::Output))
        .unwrap()
        .id;
    let mut unsafe_bounds = HashSet::new();
    negative_bounds(g, output, false, &mut HashSet::new(), &mut unsafe_bounds);
    let fast: HashSet<_> = g
        .topo_order()?
        .into_iter()
        .filter(|id| {
            let n = g.node(*id).unwrap();
            n.enabled
                && matches!(
                    n.kind,
                    NodeKind::Bounded { .. } | NodeKind::Instances { .. }
                )
                && !unsafe_bounds.contains(id)
        })
        .collect();
    let mut layout = ParamLayout::default();
    let mut source = String::new();
    if g.nodes
        .iter()
        .any(|n| n.enabled && matches!(n.kind, NodeKind::Instances { .. }))
    {
        // Inverse affine rows are packed once on the CPU, avoiding per-ray trigonometry.
        let buffer = if params == "gp" {
            "graph_params"
        } else {
            "texture_params"
        };
        writeln!(source,"struct {params}_Placement {{ p: vec4f, s: vec3f }}; fn {params}_placement(p: vec3f, i: u32) -> {params}_Placement {{ let v=vec4f(p,1.0); let row=i/4u; let x={buffer}[row]; let y={buffer}[row+1u]; let z={buffer}[row+2u]; let s=inverseSqrt(vec3f(dot(x.xyz,x.xyz),dot(y.xyz,y.xyz),dot(z.xyz,z.xyz))); return {params}_Placement(vec4f(dot(x,v),dot(y,v),dot(z,v),min(min(s.x,s.y),s.z)),s); }}").unwrap();
        writeln!(source,"fn {params}_cluster_bound(p:vec3f,i:u32)->f32 {{ let centre=vec3f({params}(i),{params}(i+1u),{params}(i+2u)); let half=vec3f({params}(i+3u),{params}(i+4u),{params}(i+5u)); return box_distance(p-centre,half); }}").unwrap();
    }
    // Topological order makes nested detail functions available before callers.
    let mut templates = HashSet::new();
    for id in g.topo_order()? {
        let n = g.node(id).unwrap();
        let functions = match &n.kind {
            NodeKind::Bounded { .. } if fast.contains(&id) => {
                vec![(format!("{params}_bound_{id}"), 1, true)]
            }
            NodeKind::Instances { .. }
                if n.enabled && templates.insert(g.input_of(id, 0).unwrap().0) =>
            {
                let template = g.input_of(id, 0).unwrap().0;
                vec![
                    (format!("{params}_instances_{template}_fast"), 0, true),
                    (format!("{params}_instances_{template}_exact"), 0, false),
                ]
            }
            _ => vec![],
        };
        for (name, port, optimised) in functions {
            let target = g.input_of(id, port).unwrap().0;
            let exact = HashSet::new();
            let c = Emitter {
                g,
                function: name,
                params,
                out: String::new(),
                layout,
                node: None,
                seed: None,
                bounds_only: false,
            }
            .emit(target, if optimised { &fast } else { &exact }, false)?;
            source.push_str(&c.wgsl);
            layout = c.layout;
        }
    }
    let c = Emitter {
        g,
        function: function.into(),
        params,
        out: String::new(),
        layout,
        node: None,
        seed: None,
        bounds_only: false,
    }
    .emit(output, &fast, true)?;
    source.push_str(&c.wgsl);
    let layout = if fast.is_empty() {
        writeln!(
            source,
            "fn {function}_exact(p: vec3f) -> DeSample {{ return {function}(p); }}"
        )
        .unwrap();
        c.layout
    } else {
        let mut leaves = Vec::new();
        union_leaves(g, g.input_of(output, 0).unwrap().0, &mut leaves);
        if leaves.len() > 1 && leaves.len() <= 128 {
            // An exact union need only evaluate branches whose conservative
            // distance can beat the current winner. This prevents every normal
            // or near-bound hit from evaluating an entire architectural world.
            let mut layout = c.layout;
            let exact_bounds = HashSet::new();
            for &leaf in &leaves {
                for (suffix, bounds) in [("probe", &fast), ("exact", &exact_bounds)] {
                    let helper = Emitter {
                        g,
                        function: format!("{params}_union_{leaf}_{suffix}"),
                        params,
                        out: String::new(),
                        layout,
                        node: None,
                        seed: None,
                        bounds_only: suffix == "probe",
                    }
                    .emit(leaf, bounds, false)?;
                    source.push_str(&helper.wgsl);
                    layout = helper.layout;
                }
            }
            let count = leaves.len();
            let count_slot = layout.add(output, "union_count", 1);
            writeln!(source,"fn {function}_exact(p: vec3f) -> DeSample {{ var best=DeSample(1e30,vec4f(1e30),0u,0u,0u); let limit=u32({params}({count_slot}u)); var winner=limit; var lower:array<f32,{count}>; var done:array<bool,{count}>;").unwrap();
            for (i, leaf) in leaves.iter().enumerate() {
                writeln!(source,"{{ let sample={params}_union_{leaf}_probe(p); lower[{i}]=sample.distance; done[{i}]=sample.flags==0u; if (sample.flags==0u && (sample.distance<best.distance || (sample.distance==best.distance && {i}u<winner))) {{best=sample;winner={i}u;}} }}").unwrap();
            }
            writeln!(source,"for (var step=0u;step<limit;step++) {{ var chosen=limit;var bound=1e30;for (var i=0u;i<limit;i++) {{if (!done[i] && (lower[i]<bound || (lower[i]==bound && i<chosen))) {{chosen=i;bound=lower[i];}}}} if (chosen==limit || bound>best.distance) {{break;}} done[chosen]=true; var sample=DeSample(1e30,vec4f(1e30),0u,0u,0u); switch chosen {{").unwrap();
            for (i, leaf) in leaves.iter().enumerate() {
                writeln!(
                    source,
                    "case {i}u: {{sample={params}_union_{leaf}_exact(p);}}"
                )
                .unwrap();
            }
            writeln!(source,"default: {{}} }} if (sample.distance<best.distance || (sample.distance==best.distance && chosen<winner)) {{best=sample;winner=chosen;}} }}").unwrap();
            let b = TRAP_SCALE_BASE;
            writeln!(source,"let scales=vec4f({params}({b}u),{params}({}u),{params}({}u),{params}({}u));let tr=select(min(best.traps/scales,vec4f(1.0)),vec4f(1.0),best.traps>=vec4f(1e30)); return DeSample(best.distance,tr,min(best.iterations,{}u),best.material,0u); }}",b+1,b+2,b+3,g.iteration_product()).unwrap();
            return Ok(Compiled {
                wgsl: source,
                layout,
                fingerprint: fingerprint(g),
            });
        }
        let exact = Emitter {
            g,
            function: format!("{function}_exact"),
            params,
            out: String::new(),
            layout: c.layout,
            node: None,
            seed: None,
            bounds_only: false,
        }
        .emit(output, &HashSet::new(), true)?;
        source.push_str(&exact.wgsl);
        exact.layout
    };
    Ok(Compiled {
        wgsl: source,
        layout,
        fingerprint: fingerprint(g),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use syfrack_core::{Graph, NodeKind};

    #[test]
    fn the_minimal_graph_compiles_to_a_de_sample_function() {
        let c = compile(&Graph::new_minimal()).unwrap();
        assert!(c.wgsl.contains("fn de_sample(p: vec3f) -> DeSample"));
        assert!(c.wgsl.contains("// node 1 Sphere"));
        assert_eq!(c.layout.index_of(1, "radius"), Some(0));
        assert!(c.wgsl.contains("gp(0u)"));
    }

    #[test]
    fn scalar_values_do_not_change_the_fingerprint_but_topology_does() {
        let a = Graph::bundled("mandelbox").unwrap();
        let mut b = a.clone();
        for n in &mut b.nodes {
            if let NodeKind::Scale { factor } = &mut n.kind {
                *factor = -1.7;
            }
        }
        assert_eq!(fingerprint(&a), fingerprint(&b));
        let mut c = a.clone();
        for n in &mut c.nodes {
            if let NodeKind::Iterate { iterations, .. } = &mut n.kind {
                *iterations += 1;
            }
        }
        assert_ne!(fingerprint(&a), fingerprint(&c));
        let mut d = a.clone();
        d.nodes[2].enabled = false;
        assert_ne!(fingerprint(&a), fingerprint(&d));
    }

    #[test]
    fn pack_puts_values_in_layout_order_and_trap_scales_at_the_end() {
        let g = Graph::bundled("mandelbox").unwrap();
        let c = compile(&g).unwrap();
        let v = c.layout.pack(&g);
        assert_eq!(v[c.layout.index_of(2, "limit").unwrap()], 1.0);
        assert_eq!(v[TRAP_SCALE_BASE], 2.0);
        assert_eq!(v[TRAP_SCALE_BASE + 1], 1.0);
    }

    #[test]
    fn architectural_graphs_use_the_extended_buffer_without_overwriting_traps() {
        let mut g = Graph::bundled("sphere").unwrap();
        let sphere = g
            .nodes
            .iter()
            .find(|n| matches!(n.kind, NodeKind::Sphere { .. }))
            .unwrap()
            .id;
        let link = g.links.iter().position(|l| l.to == (sphere, 0)).unwrap();
        let mut previous = g.links.remove(link).from.0;
        let first = g.nodes.iter().map(|n| n.id).max().unwrap() + 1;
        for id in first..first + 100 {
            g.nodes.push(syfrack_core::Node {
                id,
                kind: NodeKind::Translate {
                    offset: glam::Vec3::splat(0.01),
                },
                name: "Profile".into(),
                pos: [0.0; 2],
                enabled: true,
            });
            g.links.push(syfrack_core::Link {
                from: (previous, 0),
                to: (id, 0),
            });
            previous = id;
        }
        g.links.push(syfrack_core::Link {
            from: (previous, 0),
            to: (sphere, 0),
        });
        g.validate().unwrap();
        let c = compile(&g).unwrap();
        assert!(c.layout.len() > 256 && c.layout.len() < TRAP_SCALE_BASE);
        assert_eq!(&c.layout.pack(&g)[TRAP_SCALE_BASE..], &g.trap_scales());
    }

    #[test]
    fn iterate_emits_a_bounded_loop_with_a_bailout_break() {
        let g = Graph::bundled("mandelbulb").unwrap();
        let c = compile(&g).unwrap();
        assert!(c.wgsl.contains("for (var i1 = 0u; i1 < u32(gp("));
        let slot = c.layout.index_of(1, "iterations").unwrap();
        assert_eq!(c.layout.pack(&g)[slot], 12.0);
        assert!(c.wgsl.contains("if (length(p1_p) > gp("));
        assert!(c.wgsl.contains("break;"));
    }

    #[test]
    fn an_invalid_graph_does_not_compile() {
        let mut g = Graph::new_minimal();
        g.links.clear();
        assert!(compile(&g).is_err());
    }

    #[test]
    fn emitted_wgsl_parses_with_naga() {
        let src = crate::shader_source::pathtrace(
            &compile(&Graph::bundled("menger").unwrap()).unwrap().wgsl,
        );
        wgpu::naga::front::wgsl::parse_str(&src).expect("wgsl parses");
    }

    fn validate_wgsl(src: &str) {
        use wgpu::naga::valid::{Capabilities, ValidationFlags, Validator};
        let module = wgpu::naga::front::wgsl::parse_str(src).expect("wgsl parses");
        Validator::new(ValidationFlags::all(), Capabilities::empty())
            .validate(&module)
            .expect("wgsl validates");
    }

    /// One chain through every Point → Point kind (a nested Repeat included)
    /// and every fence and combination, two of them disabled.
    fn every_kind() -> Graph {
        use glam::Vec3;
        use syfrack_core::{Link, Measure, Node};
        use NodeKind::*;
        let point_chain = vec![
            Translate { offset: Vec3::X },
            Rotate { angles: Vec3::ONE },
            Scale { factor: 1.1 },
            Mirror {
                normal: Vec3::Y,
                offset: 0.1,
            },
            BoxFold { limit: 1.0 },
            SphereFold {
                min_radius: 0.5,
                fixed_radius: 1.0,
            },
            Abs {
                x: true,
                y: true,
                z: false,
            },
            MengerFold {
                scale: 3.0,
                offset: Vec3::ONE,
            },
            Kaleido {
                planes: 5,
                phase: 0.0,
            },
            Power { power: 8.0 },
            AddOrigin,
            Twist { rate: 0.1 },
            Bend { rate: 0.1 },
            Tile {
                period: Vec3::splat(4.0),
            },
            Trap {
                slot: 3,
                measure: Measure::AxisY,
                scale: 1.0,
                mode: Default::default(),
            },
        ];
        let mut nodes = vec![Node {
            id: 0,
            kind: Input,
            name: "in
put"
            .into(),
            pos: [0.0; 2],
            enabled: true,
        }];
        let mut links = Vec::new();
        let mut prev = 0;
        let mut id = 1;
        let mut push = |nodes: &mut Vec<Node>, kind: NodeKind, enabled: bool| {
            let n = id;
            id += 1;
            nodes.push(Node {
                id: n,
                name: kind.label().into(),
                kind,
                pos: [0.0; 2],
                enabled,
            });
            n
        };
        for kind in point_chain {
            let n = push(&mut nodes, kind, true);
            links.push(Link {
                from: (prev, 0),
                to: (n, 0),
            });
            prev = n;
        }
        let inner_trap = push(
            &mut nodes,
            Trap {
                slot: 2,
                measure: Measure::Norm,
                scale: 2.0,
                mode: Default::default(),
            },
            true,
        );
        let inner = push(
            &mut nodes,
            Iterate {
                iterations: 2,
                bailout: 4.0,
                body: vec![inner_trap],
            },
            true,
        );
        let fold = push(&mut nodes, BoxFold { limit: 1.0 }, false);
        let outer = push(
            &mut nodes,
            Iterate {
                iterations: 3,
                bailout: 4.0,
                body: vec![inner, fold],
            },
            true,
        );
        links.push(Link {
            from: (prev, 0),
            to: (outer, 0),
        });
        let fences = [
            Sphere { radius: 1.0 },
            Box { half: Vec3::ONE },
            RoundBox {
                half: Vec3::ONE,
                radius: 0.15,
            },
            Plane {
                normal: Vec3::Y,
                offset: 0.0,
            },
            NormFence { inset: 0.5 },
            BulbFence,
        ];
        let mut ds = Vec::new();
        for (i, kind) in fences.into_iter().enumerate() {
            let n = push(&mut nodes, kind, i != 1);
            links.push(Link {
                from: (outer, 0),
                to: (n, 0),
            });
            ds.push(n);
        }
        let combos = [Union, Intersection, Subtraction, SmoothUnion { k: 0.1 }];
        let mut acc = ds[0];
        for (i, kind) in combos.into_iter().enumerate() {
            let n = push(&mut nodes, kind, i != 2);
            links.push(Link {
                from: (acc, 0),
                to: (n, 0),
            });
            links.push(Link {
                from: (ds[i + 1], 0),
                to: (n, 1),
            });
            acc = n;
        }
        let si = push(&mut nodes, SmoothIntersection { k: 0.1 }, true);
        links.push(Link {
            from: (acc, 0),
            to: (si, 0),
        });
        links.push(Link {
            from: (ds[0], 0),
            to: (si, 1),
        });
        let out = push(&mut nodes, Output, true);
        links.push(Link {
            from: (si, 0),
            to: (out, 0),
        });
        Graph {
            nodes,
            links,
            safety: 0.8,
            bounding_radius_hint: 4.0,
        }
    }

    #[test]
    fn every_node_kind_compiles_to_valid_wgsl() {
        let g = every_kind();
        g.validate().unwrap();
        let c = compile(&g).unwrap();
        validate_wgsl(&crate::shader_source::pathtrace(&c.wgsl));
        validate_wgsl(&crate::shader_source::de_probe(&c.wgsl));
        assert!(c.wgsl.contains("blend_traps("));
        assert!(c.wgsl.contains("rotate_euler_xyz("));
        assert!(
            c.wgsl.contains("// node 0 in put"),
            "names stay on one comment line"
        );
        // Nested repeat: clamped to the iteration product 3 * 2.
        assert!(c.wgsl.contains("6u),"));
        // Every scalar has a slot below the trap scales, and pack fills them.
        assert!(c.layout.len() <= TRAP_SCALE_BASE);
        let v = c.layout.pack(&g);
        assert_eq!(v[TRAP_SCALE_BASE + 2], 2.0);
        assert_eq!(v[TRAP_SCALE_BASE + 3], 1.0);
    }

    #[test]
    fn disabled_nodes_emit_their_comment_or_pass_through() {
        let g = every_kind();
        let c = compile(&g).unwrap();
        let disabled: Vec<_> = g
            .nodes
            .iter()
            .filter(|n| !n.enabled)
            .map(|n| n.id)
            .collect();
        assert_eq!(disabled.len(), 3);
        // The disabled fold inside the loop emits no arithmetic.
        let fold = disabled[0];
        let marker = format!("// node {fold} ");
        let lines: Vec<&str> = c.wgsl.lines().filter(|l| l.contains(&marker)).collect();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].trim().starts_with("// node"), "{}", lines[0]);
        let after = c.wgsl.split(&marker).nth(1).unwrap();
        let next_line = after.lines().nth(1).unwrap().trim();
        assert!(
            next_line.starts_with("p") && next_line.contains("_it + 1u"),
            "{next_line}"
        );
        // The disabled Box fence is empty space; the disabled Subtraction passes port 0.
        let bx = disabled[1];
        assert!(c.wgsl.contains(&format!("let d{bx} = 1e30;")));
        let sub = disabled[2];
        let port0 = g.input_of(sub, 0).unwrap().0;
        assert!(c.wgsl.contains(&format!(
            "let d{sub} = d{port0}; let d{sub}_it = d{port0}_it; let d{sub}_tr = d{port0}_tr;"
        )));
    }

    #[test]
    fn every_bundled_graph_validates_with_naga() {
        for (name, _) in syfrack_core::scene::graph::bundled::BUNDLED {
            let c = compile(&Graph::bundled(name).unwrap()).unwrap();
            validate_wgsl(&crate::shader_source::pathtrace(&c.wgsl));
        }
    }

    #[test]
    fn independent_texture_graphs_validate_with_naga_and_use_their_own_buffer() {
        let form = compile(&Graph::bundled("sphere").unwrap()).unwrap();
        for (name, _) in syfrack_core::scene::graph::bundled::BUNDLED {
            let texture = compile_texture(&Graph::bundled(name).unwrap()).unwrap();
            validate_wgsl(&crate::shader_source::pathtrace_with_texture(
                &form.wgsl,
                Some(&texture.wgsl),
            ));
            assert!(!texture.wgsl.contains(" gp("));
            assert!(texture.wgsl.contains("texture_gp("));
        }
    }

    #[test]
    fn texture_values_keep_the_scene_pipeline_but_texture_topology_changes_it() {
        let mut scene = syfrack_core::Scene::example();
        let untextured = scene_fingerprint(&scene);
        assert_eq!(untextured, fingerprint(scene.fractal.graph()));
        scene.material.texture = Some(syfrack_core::FractalTexture::default());
        let textured = scene_fingerprint(&scene);
        assert_ne!(textured, untextured);
        scene.set_param("material.texture.scale", 8.0).unwrap();
        scene
            .set_param("material.texture.graph.nodes[5].factor", 2.4)
            .unwrap();
        scene.set_param("material.texture.octaves", 4.0).unwrap();
        assert_eq!(scene_fingerprint(&scene), textured);
        scene.material.texture.as_mut().unwrap().graph = Graph::bundled("menger").unwrap();
        assert_ne!(scene_fingerprint(&scene), textured);
    }

    #[test]
    fn nebula_toggle_changes_the_pipeline_but_numeric_edits_reuse_it() {
        let mut scene = syfrack_core::Scene::example();
        let plain = scene_fingerprint(&scene);
        scene.atmosphere.nebula = Some(syfrack_core::Nebula::default());
        let enabled = scene_fingerprint(&scene);
        assert_ne!(plain, enabled);
        scene.set_param("atmosphere.nebula.steps", 32.0).unwrap();
        scene.set_param("atmosphere.nebula.seed", 920.0).unwrap();
        assert_eq!(enabled, scene_fingerprint(&scene));
        scene.atmosphere.nebula = None;
        assert_eq!(plain, scene_fingerprint(&scene));
    }

    #[test]
    fn optical_materials_specialize_the_pipeline_and_keep_numeric_edits_cached() {
        let mut scene = syfrack_core::Scene::example();
        let plain = scene_fingerprint(&scene);
        scene.material.optics = Some(syfrack_core::Optics::default());
        let optical = scene_fingerprint(&scene);
        assert_ne!(plain, optical);
        scene.set_param("material.optics.ior", 1.33).unwrap();
        assert_eq!(optical, scene_fingerprint(&scene));
        scene.material.optics = None;
        scene.material.finishes.push(syfrack_core::Finish {
            optics: Some(syfrack_core::Optics::default()),
            ..Default::default()
        });
        assert_eq!(optical, scene_fingerprint(&scene));
    }

    #[test]
    fn every_statement_line_carries_its_node_comment() {
        let c = compile(&every_kind()).unwrap();
        let body: Vec<&str> = c.wgsl.lines().skip(1).collect();
        for line in &body[..body.len() - 1] {
            let t = line.trim();
            if t.is_empty() || t == "}" || t.starts_with("let c = p;") {
                continue;
            }
            if t.starts_with("fn ")
                || t.starts_with("let scales")
                || t.starts_with("let tr =")
                || t.starts_with("return")
            {
                continue;
            }
            assert!(t.contains("// node "), "no node comment on: {t}");
        }
    }

    /// The emitted line of `id` replaced with an undefined call, parsed by
    /// naga: the error is mapped back to the node through its line number.
    #[test]
    fn a_naga_error_is_mapped_to_the_node() {
        let mut g = Graph::new_minimal();
        g.nodes[1].kind = NodeKind::Box {
            half: glam::Vec3::ONE,
        };
        g.nodes[1].name = "Ma boîte".into();
        let c = compile(&g).unwrap();
        let broken = c.wgsl.replace("box_distance(", "box_distanc(");
        let src = crate::shader_source::pathtrace(&broken);
        let err = wgpu::naga::front::wgsl::parse_str(&src)
            .unwrap_err()
            .emit_to_string(&src);
        assert_eq!(
            node_of_error(&err, &src).as_deref(),
            Some("1 Ma boîte"),
            "{err}"
        );
        // Synthetic codespan text with only a position, no quoted source.
        let line = src
            .lines()
            .position(|l| l.contains("box_distanc("))
            .unwrap()
            + 1;
        let text = format!(
            "error: no definition in scope
  ┌─ wgsl:{line}:15
"
        );
        assert_eq!(node_of_error(&text, &src).as_deref(), Some("1 Ma boîte"));
        // A position outside the generated function maps to nothing.
        assert_eq!(
            node_of_error(
                "error
  ┌─ wgsl:3:1
",
                &src
            ),
            None
        );
    }

    #[test]
    fn library_graphs_are_valid_and_compile() {
        // The shipped forms only (embedded), never the user's saves in graphs/.
        let shipped = syfrack_core::scene::graph::bundled::BUNDLED;
        assert_eq!(shipped.len(), 11);
        for (stem, text) in shipped {
            let g: Graph = ron::from_str(text).unwrap_or_else(|e| panic!("{stem}: {e}"));
            g.validate().unwrap_or_else(|e| panic!("{stem}: {e}"));
            compile(&g).unwrap_or_else(|e| panic!("{stem}: {e}"));
        }
    }
}
