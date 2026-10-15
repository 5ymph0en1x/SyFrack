//! Forme: a small, declarative language compiled into the existing graph.
//! One statement per line; sources are named, parameters follow semicolons.
//! No evaluation of host code. Validation is shared with the CPU/GPU graph.
use std::collections::{HashMap, HashSet};
use std::fmt;

use glam::Vec3;

use super::{Graph, Link, Measure, Node, NodeId, NodeKind, TrapMode, MAX_NODES};
use crate::Params;

pub const LEGACY_EXAMPLE: &str = "# Forme 1 · les dimensions des boîtes sont des demi-tailles\nespace = départ()\nsocle = boîte(espace; taille=(3, 0.3, 3))\nposition = déplacer(espace; position=(0, 1.2, 0))\nboule = sphère(position; rayon=1)\nensemble = unir(socle, boule)\nimage = montrer(ensemble)\n";

/// Portable English vocabulary; legacy French source remains readable.
pub const EXAMPLE: &str = "# Forme 1 · box dimensions are half-sizes\nspace = input()\nbase = box(space; half=(3, 0.3, 3))\nposition = translate(space; offset=(0, 1.2, 0))\nball = sphere(position; radius=1)\nassembly = union(base, ball)\nimage = output(assembly)\n";

#[derive(Clone, Debug, PartialEq)]
pub struct Error {
    pub line: usize,
    pub message: String,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            write!(f, "{}", self.message)
        } else {
            write!(
                f,
                "{}",
                syfrack_i18n::tr_format!("Ligne {} · {}", self.line, self.message)
            )
        }
    }
}
impl std::error::Error for Error {}
fn error(line: usize, message: impl Into<String>) -> Error {
    Error {
        line,
        message: message.into(),
    }
}

/// The same vocabulary is used by the construction view and source editor.
pub fn verb(kind: &NodeKind) -> &'static str {
    use NodeKind::*;
    match kind {
        Input => "départ",
        Output => "montrer",
        Translate { .. } => "déplacer",
        Rotate { .. } => "tourner",
        Scale { .. } => "agrandir",
        ScaleAxes { .. } => "étirer",
        Mirror { .. } => "miroir",
        BoxFold { .. } => "plier_boîte",
        BoxFoldAxes { .. } => "plier_axes",
        SphereInversion { .. } => "inverser_sphère",
        SphereFold { .. } => "plier_sphère",
        Abs { .. } => "symétrie",
        MengerFold { .. } => "menger",
        Kaleido { .. } => "kaléidoscope",
        Power { .. } => "bulbe",
        AddOrigin => "ajouter_origine",
        AddSeed { .. } => "réinjecter",
        AddConstant { .. } => "constante",
        Twist { .. } => "tordre",
        Bend { .. } => "courber",
        Tile { .. } => "paver",
        RepeatFinite { .. } => "répartir",
        Iterate { .. } => "répéter",
        Trap { .. } => "mesurer",
        Sphere { .. } => "sphère",
        Torus { .. } => "tore",
        Cylinder { .. } => "cylindre",
        Box { .. } => "boîte",
        RoundBox { .. } => "boîte_arrondie",
        Rock { .. } => "roche",
        Plane { .. } => "plan",
        NormFence { .. } => "surface",
        BulbFence => "surface_bulbe",
        MaterialTag { .. } => "matériau",
        Bounded { .. } => "borner",
        Instances { .. } => "dupliquer",
        Union => "unir",
        Intersection => "croiser",
        Subtraction => "creuser",
        SmoothUnion { .. } => "fondre",
        SmoothIntersection { .. } => "croiser_doux",
    }
}

/// Stable English operation names used when exporting source.
pub fn english_verb(kind: &NodeKind) -> &'static str {
    match verb(kind) {
        "départ" => "input",
        "montrer" => "output",
        "déplacer" => "translate",
        "tourner" => "rotate",
        "agrandir" => "scale",
        "étirer" => "scale_axes",
        "miroir" => "mirror",
        "plier_boîte" => "box_fold",
        "plier_axes" => "box_fold_axes",
        "inverser_sphère" => "sphere_inversion",
        "plier_sphère" => "sphere_fold",
        "symétrie" => "abs",
        "menger" => "menger",
        "kaléidoscope" => "kaleidoscope",
        "bulbe" => "power",
        "ajouter_origine" => "add_origin",
        "réinjecter" => "add_seed",
        "constante" => "add_constant",
        "tordre" => "twist",
        "courber" => "bend",
        "paver" => "tile",
        "répartir" => "repeat_finite",
        "répéter" => "iterate",
        "mesurer" => "trap",
        "sphère" => "sphere",
        "tore" => "torus",
        "cylindre" => "cylinder",
        "boîte" => "box",
        "boîte_arrondie" => "round_box",
        "roche" => "rock",
        "plan" => "plane",
        "surface" => "norm_surface",
        "surface_bulbe" => "bulb_surface",
        "matériau" => "material",
        "borner" => "bounded",
        "dupliquer" => "instances",
        "unir" => "union",
        "croiser" => "intersect",
        "creuser" => "subtract",
        "fondre" => "smooth_union",
        "croiser_doux" => "smooth_intersection",
        other => other,
    }
}
/// Localized captions only; the parser always accepts both vocabularies.
pub fn display_verb(kind: &NodeKind) -> &'static str {
    if syfrack_i18n::language() == syfrack_i18n::Language::French {
        verb(kind)
    } else {
        english_verb(kind)
    }
}

pub fn operations() -> Vec<NodeKind> {
    use NodeKind::*;
    vec![
        Input,
        Output,
        Translate { offset: Vec3::ZERO },
        Rotate { angles: Vec3::ZERO },
        Scale { factor: 2.0 },
        ScaleAxes { factors: Vec3::ONE },
        Mirror {
            normal: Vec3::X,
            offset: 0.0,
        },
        BoxFold { limit: 1.0 },
        BoxFoldAxes {
            limit: Vec3::ONE,
            x: true,
            y: true,
            z: false,
        },
        SphereInversion {
            radius: 1.0,
            min_radius: 0.1,
        },
        SphereFold {
            min_radius: 0.5,
            fixed_radius: 1.0,
        },
        Abs {
            x: true,
            y: true,
            z: true,
        },
        MengerFold {
            scale: 3.0,
            offset: Vec3::ONE,
        },
        Kaleido {
            planes: 6,
            phase: 0.0,
        },
        Power { power: 8.0 },
        AddOrigin,
        AddSeed {
            multiplier: Vec3::ONE,
        },
        AddConstant {
            multiplier: Vec3::ONE,
            julia: Vec3::ZERO,
        },
        Twist { rate: 0.2 },
        Bend { rate: 0.2 },
        Tile {
            period: Vec3::splat(4.0),
        },
        RepeatFinite {
            period: Vec3::splat(4.0),
            count: [3, 1, 3],
        },
        Iterate {
            iterations: 8,
            bailout: 4.0,
            body: vec![],
        },
        Trap {
            slot: 0,
            measure: Measure::Norm,
            scale: 2.0,
            mode: Default::default(),
        },
        Sphere { radius: 1.0 },
        Torus {
            radius: 1.0,
            tube: 0.15,
        },
        Cylinder {
            radius: 1.0,
            half_height: 1.0,
        },
        MaterialTag { index: 0 },
        Bounded {
            half: Vec3::ONE,
            margin: 0.2,
        },
        Instances {
            placements: vec![super::Placement::default()],
            half: Vec3::ONE,
            margin: 0.2,
        },
        Box { half: Vec3::ONE },
        RoundBox {
            half: Vec3::ONE,
            radius: 0.1,
        },
        Rock {
            half: Vec3::ONE,
            erosion: 0.25,
            seed: 17,
        },
        Plane {
            normal: Vec3::Y,
            offset: 0.0,
        },
        NormFence { inset: 0.0 },
        BulbFence,
        Union,
        Intersection,
        Subtraction,
        SmoothUnion { k: 0.2 },
        SmoothIntersection { k: 0.2 },
    ]
}

fn legacy_key(field: &str) -> &str {
    match field {
        "offset" => "position",
        "angles" => "angles",
        "factor" => "facteur",
        "factors" => "facteurs",
        "tube" => "tube",
        "half_height" => "demi_hauteur",
        "index" => "indice",
        "margin" => "marge",
        "normal" => "normale",
        "limit" => "limite",
        "multiplier" => "multiplicateur",
        "julia" => "julia",
        "min_radius" => "rayon_min",
        "fixed_radius" => "rayon_fixe",
        "scale" => "échelle",
        "planes" => "secteurs",
        "phase" => "phase",
        "power" => "puissance",
        "rate" => "force",
        "period" => "période",
        "count" => "nombre",
        "iterations" => "fois",
        "bailout" => "seuil",
        "slot" => "canal",
        "radius" => "rayon",
        "half" => "taille",
        "inset" => "retrait",
        "k" => "douceur",
        other => other,
    }
}

fn key(field: &str) -> &str {
    field
}
fn canonical_parameter(name: &str) -> &str {
    match name {
        "actif" => "enabled",
        "nom" => "name",
        "étapes" => "body",
        "mesure" => "measure",
        other => {
            const FIELDS: &[&str] = &[
                "offset",
                "angles",
                "factor",
                "factors",
                "tube",
                "half_height",
                "index",
                "margin",
                "normal",
                "limit",
                "multiplier",
                "julia",
                "min_radius",
                "fixed_radius",
                "scale",
                "planes",
                "phase",
                "power",
                "rate",
                "period",
                "count",
                "iterations",
                "bailout",
                "slot",
                "radius",
                "half",
                "inset",
                "k",
            ];
            FIELDS
                .iter()
                .copied()
                .find(|field| legacy_key(field) == other)
                .unwrap_or(other)
        }
    }
}

pub fn aliases(g: &Graph) -> HashMap<NodeId, String> {
    g.nodes
        .iter()
        .map(|n| {
            let stem: String = n
                .name
                .to_lowercase()
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            let stem = stem.trim_matches('_');
            let stem = if stem.is_empty() { verb(&n.kind) } else { stem };
            (
                n.id,
                format!(
                    "{}{stem}_{}",
                    if stem.chars().next().is_some_and(char::is_numeric) {
                        "element_"
                    } else {
                        ""
                    },
                    n.id
                ),
            )
        })
        .collect()
}

/// Named parameters, in the graph's native visiting order, without rounding.
pub fn parameters(n: &Node) -> Vec<(String, String)> {
    let mut g = Graph::new_minimal();
    g.nodes = vec![n.clone()];
    let mut grouped: Vec<(String, Vec<f32>)> = Vec::new();
    g.visit_params("", &mut |path, v, _| {
        let Some((_, field)) = path.split_once("].") else {
            return;
        };
        if field.starts_with("placements[") {
            return;
        }
        let base = field.split('.').next().unwrap();
        if let Some((_, values)) = grouped.last_mut().filter(|(k, _)| k == base) {
            values.push(*v);
        } else {
            grouped.push((base.into(), vec![*v]));
        }
    });
    let mut result: Vec<_> = grouped
        .into_iter()
        .map(|(name, values)| {
            let value = if values.len() == 1 {
                values[0].to_string()
            } else {
                format!(
                    "({})",
                    values
                        .iter()
                        .map(f32::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            (key(&name).into(), value)
        })
        .collect();
    match &n.kind {
        NodeKind::Instances { placements, .. } => result.push((
            "copies".into(),
            ron::to_string(placements).expect("serialisable placements"),
        )),
        NodeKind::Abs { x, y, z } | NodeKind::BoxFoldAxes { x, y, z, .. } => result.push((
            "axes".into(),
            format!(
                "\"{}{}{}\"",
                if *x { "x" } else { "" },
                if *y { "y" } else { "" },
                if *z { "z" } else { "" }
            ),
        )),
        NodeKind::Trap { measure, .. } => result.push((
            "measure".into(),
            match measure {
                Measure::Norm => "norm",
                Measure::AxisX => "axis_x",
                Measure::AxisY => "axis_y",
                Measure::AxisZ => "axis_z",
                Measure::PlaneY => "plane_y",
                Measure::AngleXY => "angle_xy",
                Measure::AngleXZ => "angle_xz",
                Measure::AngleYZ => "angle_yz",
                Measure::LogNorm => "log_norm",
            }
            .into(),
        )),
        _ => {}
    }
    if let NodeKind::Trap { mode, .. } = &n.kind {
        result.push((
            "accumulation".into(),
            match mode {
                TrapMode::Minimum => "minimum",
                TrapMode::Maximum => "maximum",
                TrapMode::Last => "last",
            }
            .into(),
        ));
    }
    if !n.enabled {
        result.push(("enabled".into(), "false".into()));
    }
    result
}

fn order(g: &Graph) -> Vec<NodeId> {
    fn visit(g: &Graph, id: NodeId, seen: &mut HashSet<NodeId>, out: &mut Vec<NodeId>) {
        if !seen.insert(id) {
            return;
        }
        for port in 0..g.node(id).map_or(0, |n| n.kind.inputs().len()) {
            if let Some(src) = g.input_of(id, port as u32) {
                visit(g, src.0, seen, out);
            }
        }
        for b in g.body_ids_of(id) {
            visit(g, *b, seen, out);
        }
        out.push(id);
    }
    let mut out = vec![];
    let mut seen = HashSet::new();
    for n in &g.nodes {
        visit(g, n.id, &mut seen, &mut out);
    }
    out
}

/// Canonical source. With `compile(..., Some(graph))`, ids, names, positions
/// and original ordering survive a round trip; standalone sources are portable geometry.
pub fn format(g: &Graph) -> String {
    let names = aliases(g);
    let mut text = format!(
        "# Forme 1 · named sources; parameters follow semicolons\nsettings safety={} radius={}\n\n",
        g.safety, g.bounding_radius_hint
    );
    for id in order(g) {
        let n = g.node(id).unwrap();
        let sources: Vec<_> = if g
            .nodes
            .iter()
            .any(|owner| g.body_ids_of(owner.id).contains(&id))
        {
            vec![]
        } else {
            (0..n.kind.inputs().len())
                .map(|p| {
                    g.input_of(id, p as u32)
                        .map_or_else(|| "_".into(), |src| names[&src.0].clone())
                })
                .collect()
        };
        let mut params = parameters(n);
        if let NodeKind::Iterate { body, .. } = &n.kind {
            params.push((
                "body".into(),
                format!(
                    "[{}]",
                    body.iter()
                        .map(|b| names[b].clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }
        let suffix: String = params
            .into_iter()
            .map(|(k, v)| format!("; {k}={v}"))
            .collect();
        text.push_str(&format!(
            "{} = {}({}{suffix})\n",
            names[&id],
            english_verb(&n.kind),
            sources.join(", ")
        ));
    }
    text
}

/// Split outside vectors, lists and quoted text; '#' starts a comment.
fn split(text: &str, separator: char, line: usize) -> Result<Vec<String>, Error> {
    let mut out = vec![];
    let mut start = 0;
    let mut depth = Vec::new();
    let mut quoted = false;
    let mut escaped = false;
    for (i, c) in text.char_indices() {
        if quoted {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                quoted = false;
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            '(' | '[' => depth.push(c),
            ')' | ']' => {
                if depth.pop() != Some(if c == ')' { '(' } else { '[' }) {
                    return Err(error(
                        line,
                        syfrack_i18n::tr!("Parenthèses ou crochets déséquilibrés"),
                    ));
                }
            }
            '#' => {
                out.push(text[start..i].trim().into());
                return Ok(out);
            }
            c if c == separator && depth.is_empty() => {
                out.push(text[start..i].trim().into());
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    if quoted || !depth.is_empty() {
        return Err(error(
            line,
            syfrack_i18n::tr!("Texte, parenthèses ou crochets non fermés"),
        ));
    }
    out.push(text[start..].trim().into());
    Ok(out)
}
fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s != "_"
        && s.chars().all(|c| c.is_alphanumeric() || c == '_')
        && !s.chars().next().unwrap().is_numeric()
}

struct Statement {
    line: usize,
    node: Node,
    sources: Vec<String>,
    body: Vec<String>,
}

pub fn compile(text: &str, base: Option<&Graph>) -> Result<Graph, Error> {
    if text.len() > 1_048_576 {
        return Err(error(
            0,
            syfrack_i18n::tr!("Source trop longue (maximum 1 Mio)"),
        ));
    }
    let base_names: HashMap<String, NodeId> = base
        .map(aliases)
        .unwrap_or_default()
        .into_iter()
        .map(|(id, alias)| (alias, id))
        .collect();
    let mut statements = vec![];
    let mut names = HashMap::new();
    let mut used_ids = HashSet::new();
    let mut next = base.map_or(0, Graph::next_id);
    let mut g = base.cloned().unwrap_or_else(Graph::new_minimal);
    g.nodes.clear();
    g.links.clear();
    let mut settings = false;
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let parts = split(raw, '\0', line)?;
        let raw = parts[0].trim();
        if raw.is_empty() {
            continue;
        }
        if let Some(rest) = raw
            .strip_prefix("réglages ")
            .or_else(|| raw.strip_prefix("settings "))
        {
            if settings {
                return Err(error(
                    line,
                    syfrack_i18n::tr!("Une seule ligne réglages est permise"),
                ));
            }
            settings = true;
            let mut keys = HashSet::new();
            for value in rest.split_whitespace() {
                let (k, v) = value.split_once('=').ok_or_else(|| {
                    error(
                        line,
                        syfrack_i18n::tr!("Attendu : réglages pas=0.8 rayon=10"),
                    )
                })?;
                if !keys.insert(k) {
                    return Err(error(
                        line,
                        syfrack_i18n::tr_format!("Réglage {k} répété", k = k),
                    ));
                }
                let v: f32 = v
                    .parse()
                    .map_err(|_| error(line, syfrack_i18n::tr!("Nombre invalide")))?;
                if !v.is_finite() {
                    return Err(error(
                        line,
                        syfrack_i18n::tr!("Les valeurs doivent être finies"),
                    ));
                }
                match k {
                    "pas" | "safety" => g.safety = v,
                    "rayon" | "radius" => g.bounding_radius_hint = v,
                    _ => {
                        return Err(error(
                            line,
                            syfrack_i18n::tr_format!("Réglage inconnu : {k}", k = k),
                        ))
                    }
                }
            }
            continue;
        }
        if statements.len() >= MAX_NODES {
            return Err(error(
                line,
                syfrack_i18n::tr_format!("Maximum {MAX_NODES} étapes", MAX_NODES = MAX_NODES),
            ));
        }
        let (alias, expr) = raw.split_once('=').ok_or_else(|| {
            error(
                line,
                syfrack_i18n::tr!("Attendu : nom = opération(source; paramètre=valeur)"),
            )
        })?;
        let alias = alias.trim();
        if !identifier(alias) {
            return Err(error(
                line,
                syfrack_i18n::tr!("Nom invalide : utiliser lettres, chiffres et _"),
            ));
        }
        if names.contains_key(alias) {
            return Err(error(
                line,
                syfrack_i18n::tr_format!("Le nom {alias} existe déjà", alias = alias),
            ));
        }
        let (op, args) = expr.trim().split_once('(').ok_or_else(|| {
            error(
                line,
                syfrack_i18n::tr!("L’opération doit être suivie de (...)"),
            )
        })?;
        let args = args
            .strip_suffix(')')
            .ok_or_else(|| error(line, syfrack_i18n::tr!("Attendu : ) en fin d’opération")))?;
        let kind = operations()
            .into_iter()
            .find(|k| verb(k) == op.trim() || english_verb(k) == op.trim())
            .ok_or_else(|| {
                error(
                    line,
                    syfrack_i18n::tr_format!("Opération inconnue : {}", op.trim()),
                )
            })?;
        let id = if let Some(id) = base_names.get(alias) {
            *id
        } else {
            while used_ids.contains(&next) || base.is_some_and(|b| b.node(next).is_some()) {
                next = next
                    .checked_add(1)
                    .ok_or_else(|| error(line, syfrack_i18n::tr!("Identifiants épuisés")))?;
            }
            let id = next;
            next = next
                .checked_add(1)
                .ok_or_else(|| error(line, syfrack_i18n::tr!("Identifiants épuisés")))?;
            id
        };
        if !used_ids.insert(id) {
            return Err(error(line, syfrack_i18n::tr!("Identifiant déjà utilisé")));
        }
        names.insert(alias.to_string(), id);
        let old = base.and_then(|b| b.node(id));
        let mut node = Node {
            id,
            kind,
            name: old.map_or_else(|| alias.to_string(), |n| n.name.clone()),
            pos: old.map_or(
                [
                    240.0 * (statements.len() % 4) as f32,
                    136.0 * (statements.len() / 4) as f32,
                ],
                |n| n.pos,
            ),
            enabled: true,
        };
        let args = split(args, ';', line)?;
        let sources = if args[0].is_empty() {
            vec![]
        } else {
            split(&args[0], ',', line)?
        };
        let mut body = vec![];
        let mut keys = HashSet::new();
        for arg in &args[1..] {
            let (k, value) = arg.split_once('=').ok_or_else(|| {
                error(
                    line,
                    syfrack_i18n::tr!("Un paramètre doit s’écrire nom=valeur"),
                )
            })?;
            let (k, value) = (canonical_parameter(k.trim()), value.trim());
            if !keys.insert(k) {
                return Err(error(
                    line,
                    syfrack_i18n::tr_format!("Paramètre {k} répété", k = k),
                ));
            }
            match k {
                "enabled" => {
                    node.enabled = match value {
                        "oui" | "true" => true,
                        "non" | "false" => false,
                        _ => return Err(error(line, syfrack_i18n::tr!("actif attend oui ou non"))),
                    }
                }
                "name" => {
                    node.name = ron::from_str(value).map_err(|_| {
                        error(
                            line,
                            syfrack_i18n::tr!("nom attend un texte entre guillemets"),
                        )
                    })?
                }
                "body" if matches!(node.kind, NodeKind::Iterate { .. }) => {
                    let list = value
                        .strip_prefix('[')
                        .and_then(|s| s.strip_suffix(']'))
                        .ok_or_else(|| {
                            error(
                                line,
                                syfrack_i18n::tr!("étapes attend [première, deuxième]"),
                            )
                        })?;
                    body = if list.trim().is_empty() {
                        vec![]
                    } else {
                        split(list, ',', line)?
                    };
                }
                "copies" if matches!(node.kind, NodeKind::Instances { .. }) => {
                    if let NodeKind::Instances { placements, .. } = &mut node.kind {
                        *placements = ron::from_str(value).map_err(|e| {
                            error(
                                line,
                                syfrack_i18n::tr_format!(
                                    "copies attend une liste de placements : {e}",
                                    e = e
                                ),
                            )
                        })?;
                    }
                }
                "axes"
                    if matches!(
                        node.kind,
                        NodeKind::Abs { .. } | NodeKind::BoxFoldAxes { .. }
                    ) =>
                {
                    let axes: String = ron::from_str(value).map_err(|_| {
                        error(
                            line,
                            syfrack_i18n::tr!("axes attend un texte, par exemple \"xz\""),
                        )
                    })?;
                    if axes.chars().any(|c| !"xyz".contains(c)) {
                        return Err(error(line, syfrack_i18n::tr!("Axes autorisés : x, y et z")));
                    }
                    match &mut node.kind {
                        NodeKind::Abs { x, y, z } | NodeKind::BoxFoldAxes { x, y, z, .. } => {
                            *x = axes.contains('x');
                            *y = axes.contains('y');
                            *z = axes.contains('z');
                        }
                        _ => unreachable!(),
                    }
                }
                "measure" if matches!(node.kind, NodeKind::Trap { .. }) => {
                    let measure = match value {
                        "norme" | "norm" => Measure::Norm,
                        "axe_x" | "axis_x" => Measure::AxisX,
                        "axe_y" | "axis_y" => Measure::AxisY,
                        "axe_z" | "axis_z" => Measure::AxisZ,
                        "plan_y" | "plane_y" => Measure::PlaneY,
                        "angle_xy" => Measure::AngleXY,
                        "angle_xz" => Measure::AngleXZ,
                        "angle_yz" => Measure::AngleYZ,
                        "norme_log" | "log_norm" => Measure::LogNorm,
                        _ => return Err(error(line, syfrack_i18n::tr!("Mesure inconnue"))),
                    };
                    if let NodeKind::Trap { measure: m, .. } = &mut node.kind {
                        *m = measure;
                    }
                }
                "accumulation" if matches!(node.kind, NodeKind::Trap { .. }) => {
                    let mode = match value {
                        "minimum" => TrapMode::Minimum,
                        "maximum" => TrapMode::Maximum,
                        "dernière" | "derniere" | "last" => TrapMode::Last,
                        _ => return Err(error(line, syfrack_i18n::tr!("Accumulation inconnue"))),
                    };
                    if let NodeKind::Trap { mode: m, .. } = &mut node.kind {
                        *m = mode;
                    }
                }
                _ => set_number(&mut node, k, value, line)?,
            }
        }
        statements.push(Statement {
            line,
            node,
            sources,
            body,
        });
    }
    let mut owners = HashSet::new();
    for st in &mut statements {
        let ids: Result<Vec<_>, _> = st
            .body
            .iter()
            .map(|s| {
                names.get(s).copied().ok_or_else(|| {
                    error(
                        st.line,
                        syfrack_i18n::tr_format!("Étape inconnue : {s}", s = s),
                    )
                })
            })
            .collect();
        let ids = ids?;
        owners.extend(ids.iter().copied());
        if let NodeKind::Iterate { body, .. } = &mut st.node.kind {
            *body = ids;
        }
    }
    for st in &statements {
        let count = if owners.contains(&st.node.id) {
            0
        } else {
            st.node.kind.inputs().len()
        };
        if st.sources.len() != count {
            return Err(error(
                st.line,
                syfrack_i18n::tr_format!(
                    "{} attend {count} source(s), {} reçue(s)",
                    verb(&st.node.kind),
                    st.sources.len(),
                    count = count
                ),
            ));
        }
        for (port, source) in st
            .sources
            .iter()
            .enumerate()
            .filter(|(_, s)| s.as_str() != "_")
        {
            let src = names.get(source).ok_or_else(|| {
                error(
                    st.line,
                    syfrack_i18n::tr_format!("Source inconnue : {source}", source = source),
                )
            })?;
            g.links.push(Link {
                from: (*src, 0),
                to: (st.node.id, port as u32),
            });
        }
        g.nodes.push(st.node.clone());
    }
    // Preserve the editor layout and equality/undo for an unedited source.
    if let Some(base) = base {
        g.nodes.sort_by_key(|n| {
            base.nodes
                .iter()
                .position(|old| old.id == n.id)
                .unwrap_or(base.nodes.len() + n.id as usize)
        });
        g.links.sort_by_key(|l| {
            base.links
                .iter()
                .position(|old| old == l)
                .unwrap_or(base.links.len())
        });
    }
    g.validate_structure().map_err(|e| {
        let message = e.to_string();
        let line = statements
            .iter()
            .find(|s| {
                message.contains(&format!("nodes[{}]", s.node.id))
                    || message.contains(&format!("node {}:", s.node.id))
            })
            .map_or(0, |s| s.line);
        error(line, message)
    })?;
    Ok(g)
}

fn set_number(node: &mut Node, name: &str, text: &str, line: usize) -> Result<(), Error> {
    let values: Vec<f32> =
        if let Some(tuple) = text.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
            split(tuple, ',', line)?
                .iter()
                .map(|v| {
                    v.parse()
                        .map_err(|_| error(line, syfrack_i18n::tr!("Nombre invalide")))
                })
                .collect::<Result<_, _>>()?
        } else {
            vec![text.parse().map_err(|_| {
                error(
                    line,
                    syfrack_i18n::tr_format!("{name} attend un nombre", name = name),
                )
            })?]
        };
    let mut graph = Graph::new_minimal();
    graph.nodes = vec![node.clone()];
    let mut fields = vec![];
    graph.visit_params("", &mut |path, _, bounds| {
        if let Some((_, field)) = path.split_once("].") {
            if key(field.split('.').next().unwrap()) == name {
                fields.push((path.to_string(), bounds));
            }
        }
    });
    if fields.is_empty() {
        return Err(error(
            line,
            syfrack_i18n::tr_format!(
                "Paramètre inconnu pour {} : {name}",
                verb(&node.kind),
                name = name
            ),
        ));
    }
    if fields.len() != values.len() {
        return Err(error(
            line,
            syfrack_i18n::tr_format!("{name} attend {} valeur(s)", fields.len(), name = name),
        ));
    }
    for ((path, bounds), value) in fields.iter().zip(&values) {
        if !bounds.contains(*value) {
            return Err(error(
                line,
                syfrack_i18n::tr_format!(
                    "{name} doit être entre {} et {}",
                    bounds.min,
                    bounds.max,
                    name = name
                ),
            ));
        }
        if ["iterations", "planes", "slot", "count", "index"].contains(&name)
            && value.fract() != 0.0
        {
            return Err(error(
                line,
                syfrack_i18n::tr_format!("{name} attend un entier", name = name),
            ));
        }
        graph.visit_params("", &mut |p, v, _| {
            if p == path {
                *v = *value;
            }
        });
    }
    node.kind = graph.nodes.remove(0).kind;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hybrid_parameters_and_axes_roundtrip_without_resetting_widths() {
        let source = "e = départ()\nf = plier_axes(e; limite=(0.5, 1.2, 2); axes=\"xz\")\ni = inverser_sphère(f; radius=1.4; rayon_min=0.05)\nc = constante(i; multiplicateur=(-1, 0.2, 0); julia=(0.1, -0.3, 0.8))\ns = surface(c)\no = montrer(s)";
        let g = compile(source, None).unwrap();
        g.validate().unwrap();
        assert!(
            matches!(g.nodes[1].kind, NodeKind::BoxFoldAxes { limit, x: true, y: false, z: true } if limit == Vec3::new(0.5, 1.2, 2.0))
        );
        assert_eq!(compile(&format(&g), Some(&g)).unwrap(), g);
        assert!(compile(&source.replace("rayon_min=0.05", "rayon_min=0"), None).is_err());
        let mut changed = g.clone();
        if let NodeKind::BoxFoldAxes { y, .. } = &mut changed.nodes[1].kind {
            *y = true;
        }
        assert_ne!(
            g.nodes[1].kind.topology_key(),
            changed.nodes[1].kind.topology_key()
        );
    }
    #[test]
    fn all_bundled_forms_roundtrip_and_keep_editor_metadata() {
        for name in [
            "sphere",
            "mandelbox",
            "menger",
            "mandelbulb",
            "boite-bulbe",
            "menger-kaleido",
            "evidement",
            "cathedrale",
            "rosace-plis",
            "dentelle-inversee",
            "menger-torsade",
        ] {
            let g = Graph::bundled(name).unwrap();
            assert_eq!(compile(&format(&g), Some(&g)).unwrap(), g, "{name}");
            let rebuilt = compile(&format(&g), None).unwrap();
            rebuilt.validate().unwrap();
            for p in [Vec3::ZERO, Vec3::ONE, Vec3::new(0.2, -0.3, 1.5)] {
                assert_eq!(g.evaluate(p), rebuilt.evaluate(p), "{name}");
            }
        }
    }
    #[test]
    fn new_program_compiles_and_rejects_bad_links_values_and_cycles() {
        compile(EXAMPLE, None).unwrap().validate().unwrap();
        for source in [
            EXAMPLE.replace("union(base, ball)", "union(space, ball)"),
            EXAMPLE.replace("radius=1", "radius=NaN"),
            EXAMPLE.replace("radius=1", "radius=0"),
            EXAMPLE.replace("offset=(0, 1.2, 0)", "offset=(0, 1.2)"),
            EXAMPLE.replace("sphere(position", "sphere(absent"),
            EXAMPLE.replace("translate(space", "translate(position"),
        ] {
            assert!(compile(&source, None).is_err(), "{source}");
        }
        let unknown = EXAMPLE.replace("sphere(position", "dragon(position");
        assert_eq!(compile(&unknown, None).unwrap_err().line, 5);
    }
    #[test]
    fn incomplete_ports_disabled_nodes_and_all_variants_survive() {
        let mut g = Graph::new_minimal();
        g.links.clear();
        for kind in operations()
            .into_iter()
            .filter(|k| !matches!(k, NodeKind::Input | NodeKind::Output))
        {
            let id = g.next_id();
            g.nodes.push(Node {
                id,
                name: format!("Test {id}"),
                kind,
                pos: [12.0, 42.0],
                enabled: false,
            });
        }
        g.validate_structure().unwrap();
        assert_eq!(compile(&format(&g), Some(&g)).unwrap(), g);
    }
    #[test]
    fn integer_values_are_refused_instead_of_silently_clamped() {
        let source = "e = départ()\nb = répéter(e; fois=1.2)\ns = surface(b)\ni = montrer(s)";
        assert!(compile(source, None)
            .unwrap_err()
            .message
            .contains("integer"));
    }
}
