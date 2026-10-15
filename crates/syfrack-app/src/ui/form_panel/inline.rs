//! The inline sliders of the cards.

use super::*;

/// One inline parameter of a card: a scalar, or the three of a vec3.
#[derive(Clone, Debug)]
pub struct InlineRow {
    /// Field name (`factor`, `offset`…).
    pub field: String,
    /// One value per component.
    pub values: Vec<f32>,
    pub bounds: Bounds,
}

/// The first `INLINE_PARAMS` parameters of every node, a vec3 as one row,
/// read in one pass over the frame's own copy of the graph.
pub fn inline_values(g: &mut Graph) -> HashMap<NodeId, Vec<InlineRow>> {
    let mut fields: HashMap<NodeId, Vec<InlineRow>> = HashMap::new();
    g.visit_params("", &mut |path, v, b| {
        let Some((id, field)) = split_node_path(path) else {
            return;
        };
        if field.starts_with("placements[") {
            return;
        }
        let name = field.split('.').next().unwrap_or(field).to_string();
        let list = fields.entry(id).or_default();
        match list.last_mut() {
            Some(row) if row.field == name => row.values.push(*v),
            _ => list.push(InlineRow {
                field: name,
                values: vec![*v],
                bounds: b,
            }),
        }
    });
    for list in fields.values_mut() {
        list.truncate(canvas::INLINE_PARAMS);
    }
    fields
}

/// "nodes[ID].field[.x]" → (ID, "field[.x]").
pub(super) fn split_node_path(path: &str) -> Option<(NodeId, &str)> {
    let rest = path.strip_prefix("nodes[")?;
    let (id, rest) = rest.split_once("].")?;
    Some((id.parse().ok()?, rest))
}

/// Every parameter of one node, as "field" or "field.x", with its bounds.
pub(super) fn node_params(n: &Node) -> Vec<(String, Bounds)> {
    let mut g = Graph::new_minimal();
    g.nodes = vec![n.clone()];
    g.links.clear();
    let mut out = Vec::new();
    g.visit_params("", &mut |path, _, b| {
        if let Some((_, field)) = split_node_path(path) {
            out.push((field.to_string(), b));
        }
    });
    out
}

/// Rail of component `c` of `count` in inline row `k` of a card at screen
/// rectangle `r` and zoom `z`.
pub fn row_rail(r: Rect, k: usize, c: usize, count: usize, z: f32) -> Rect {
    let pad = theme::GRID * z;
    let y = r.min.y + (canvas::HEADER_H + (k as f32 + 0.5) * canvas::ROW_H) * z;
    let rail_y = y + 0.875 * pad;
    let left = r.min.x + 11.0 * pad;
    let right = r.max.x - 1.5 * pad;
    let gap = pad;
    let w = (right - left - gap * (count as f32 - 1.0)) / count as f32;
    let x0 = left + c as f32 * (w + gap);
    Rect::from_min_max(
        Pos2::new(x0, rail_y - 1.0 * z),
        Pos2::new(x0 + w, rail_y + 1.0 * z),
    )
}

/// The inline sliders of a card: label, then one rail per component with
/// its value above. The rails of the card under the pointer (or the one
/// being dragged) are live: a drag or click writes through `set_param`.
#[allow(clippy::too_many_arguments)]
pub(super) fn card_rows(
    ui: &Ui,
    painter: &egui::Painter,
    state: &mut AppState,
    n: &Node,
    rows: &[InlineRow],
    r: Rect,
    z: f32,
    live: bool,
) {
    let pad = theme::GRID * z;
    let small = FontId::proportional(11.0 * z);
    let ink = if n.enabled {
        theme::TEXT
    } else {
        theme::TEXT_DIM
    };
    if let NodeKind::MaterialTag { index } = n.kind {
        let name = if index == 0 {
            syfrack_i18n::tr!("Surface globale")
        } else {
            state
                .scene
                .material
                .finishes
                .get(index as usize - 1)
                .map(|f| f.name.as_str())
                .unwrap_or(syfrack_i18n::tr!("Matériau absent"))
        };
        painter.text(
            Pos2::new(
                r.min.x + 1.5 * pad,
                r.min.y + (canvas::HEADER_H + canvas::ROW_H * 0.5) * z,
            ),
            Align2::LEFT_CENTER,
            name,
            small,
            ink,
        );
        return;
    }
    for (k, row) in rows.iter().enumerate() {
        let y = r.min.y + (canvas::HEADER_H + (k as f32 + 0.5) * canvas::ROW_H) * z;
        let whole = is_integer(&row.field);
        let label = if whole {
            format!("{} {}", widgets::TOPOLOGY_MARK, field_label(&row.field))
        } else {
            field_label(&row.field).to_string()
        };
        painter.text(
            Pos2::new(r.min.x + 1.5 * pad, y),
            Align2::LEFT_CENTER,
            label,
            small.clone(),
            theme::TEXT_DIM,
        );
        let count = row.values.len();
        let axes = ["x", "y", "z"];
        for (c, value) in row.values.iter().enumerate() {
            let field = if count == 1 {
                row.field.clone()
            } else {
                format!("{}.{}", row.field, axes[c.min(2)])
            };
            let rail = row_rail(r, k, c, count, z);
            let mut v = *value;
            let id = Id::new(("inline", n.id, &field));
            let live = live || ui.ctx().is_being_dragged(id);
            let changed = widgets::card_rail(
                ui,
                live.then_some(id),
                rail,
                &mut v,
                row.bounds,
                is_log(row.bounds),
                whole,
                graph_edit::default_param(&n.kind, &field),
                z,
            );
            if changed {
                state.set_param(&format!("fractal.nodes[{}].{field}", n.id), v);
            }
            let decimals = if whole || v.abs() >= 100.0 { 0 } else { 2 };
            painter.text(
                Pos2::new(rail.max.x, y - 0.25 * pad),
                Align2::RIGHT_CENTER,
                widgets::format_value(v, decimals),
                small.clone(),
                ink,
            );
        }
    }
}

/// Integer fields: a change rebuilds the shader.
pub(crate) fn is_integer(field: &str) -> bool {
    matches!(field, "iterations" | "planes" | "slot" | "count")
}

/// Ranges over two decades read better on a logarithmic rail.
pub(super) fn is_log(b: Bounds) -> bool {
    b.min > 0.0 && b.max / b.min >= 100.0
}

pub(crate) fn field_label(field: &str) -> &str {
    match field {
        "offset" => syfrack_i18n::tr!("décalage"),
        "angles" => syfrack_i18n::tr!("angles"),
        "factor" => syfrack_i18n::tr!("facteur"),
        "factors" => syfrack_i18n::tr!("facteurs"),
        "tube" => syfrack_i18n::tr!("rayon du tube"),
        "half_height" => syfrack_i18n::tr!("demi-hauteur"),
        "erosion" => syfrack_i18n::tr!("érosion"),
        "seed" => syfrack_i18n::tr!("variation"),
        "index" => syfrack_i18n::tr!("matériau"),
        "margin" => syfrack_i18n::tr!("marge de calcul"),
        "normal" => syfrack_i18n::tr!("normale"),
        "limit" => syfrack_i18n::tr!("limite"),
        "multiplier" => syfrack_i18n::tr!("multiplicateur origine"),
        "julia" => syfrack_i18n::tr!("constante Julia"),
        "min_radius" => syfrack_i18n::tr!("rayon min"),
        "fixed_radius" => syfrack_i18n::tr!("rayon fixe"),
        "scale" => syfrack_i18n::tr!("échelle"),
        "planes" => syfrack_i18n::tr!("plans"),
        "phase" => syfrack_i18n::tr!("phase"),
        "power" => syfrack_i18n::tr!("puissance"),
        "rate" => syfrack_i18n::tr!("taux"),
        "period" => syfrack_i18n::tr!("période"),
        "count" => syfrack_i18n::tr!("nombre de copies"),
        "iterations" => syfrack_i18n::tr!("itérations"),
        "bailout" => syfrack_i18n::tr!("évasion"),
        "slot" => syfrack_i18n::tr!("piège"),
        "radius" => syfrack_i18n::tr!("rayon"),
        "half" => syfrack_i18n::tr!("demi-côté"),
        "inset" => syfrack_i18n::tr!("retrait"),
        other => other,
    }
}
