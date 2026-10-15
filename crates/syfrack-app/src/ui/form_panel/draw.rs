//! Drawing the canvas: grid, frames, wires, cards and ports.

use super::*;

impl FormPanel {
    pub(super) fn canvas(&mut self, ui: &mut Ui, state: &mut AppState, g: &mut Graph) {
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        self.rect = rect;
        let mut view = state.form.view;
        if self.input(ui, state, g, &resp, rect, &mut view) {
            *g = state.scene.fractal.to_graph();
        }
        state.form.view = view;
        // A library entry dropped on the canvas joins the graph there.
        if let Some(drop) = resp.dnd_release_payload::<LibraryDrag>() {
            let at = ui
                .input(|i| i.pointer.latest_pos())
                .map(|p| view.to_world(rect.min, p))
                .unwrap_or(Pos2::ZERO);
            let mut added = Vec::new();
            let ok = state.edit_graph(|g| {
                added = graph_edit::insert_subgraph(g, &drop.graph, [snap(at.x), snap(at.y)]);
                Ok(())
            });
            if ok {
                state.form.selected = added;
                *g = state.scene.fractal.to_graph();
            }
        }
        let rows = inline_values(g);
        let now = ui.input(|i| i.time);

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, theme::BG);
        grid(&painter, rect, &view);
        let to_screen = |w: Pos2| view.to_screen(rect.min, w);
        let world_rect = |r: Rect| Rect::from_min_max(to_screen(r.min), to_screen(r.max));
        let z = view.zoom;

        // Frames, outermost first so nested ones sit on top.
        let mut frames: Vec<(&Node, Rect)> = g
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Iterate { .. }))
            .map(|n| (n, canvas::frame_rect(g, n)))
            .collect();
        frames.sort_by(|a, b| b.1.area().total_cmp(&a.1.area()));
        for (n, r) in &frames {
            let r = world_rect(*r);
            painter.rect_filled(r, CORNER * z, FRAME_FILL);
            let stroke = if state.form.selected.contains(&n.id) {
                theme::ACCENT
            } else if self.hover == Hit::Frame(n.id) {
                theme::TEXT_DIM
            } else {
                frame_border()
            };
            painter.rect_stroke(r, CORNER * z, Stroke::new(1.5, stroke), StrokeKind::Inside);
        }

        // Wires, coloured by the type of the port they leave. A lifted wire
        // is drawn by the pointer instead.
        let lifted = match (self.wire(), &self.menu) {
            (Some(w), _) => w.lifted,
            (
                None,
                Some(Menu {
                    kind: MenuKind::Add { wire: Some(w), .. },
                    ..
                }),
            ) => w.lifted,
            _ => None,
        };
        for (i, l) in g.links.iter().enumerate() {
            if Some(l.to) == lifted {
                continue;
            }
            let (Some(a), Some(b)) = (g.node(l.from.0), g.node(l.to.0)) else {
                continue;
            };
            let ty = a.kind.outputs().get(l.from.1 as usize).copied();
            let bz = canvas::bezier(
                canvas::out_port_pos(a, l.from.1 as usize),
                canvas::in_port_pos(b, l.to.1 as usize),
            );
            painter.add(CubicBezierShape::from_points_stroke(
                bz.map(to_screen),
                false,
                Color32::TRANSPARENT,
                Stroke::new(
                    if self.hover == Hit::Wire(i) { 3.0 } else { 2.0 },
                    port_color(ty),
                ),
            ));
        }
        // Cards with their inline sliders, then the ports on top.
        for n in &g.nodes {
            let r = world_rect(canvas::card_rect(n));
            self.card(&painter, state, n, r, z);
            let inline = rows.get(&n.id).map_or(&[][..], Vec::as_slice);
            let live = self.hover == Hit::Card(n.id) && self.drag.is_none();
            card_rows(ui, &painter, state, n, inline, r, z, live);
        }
        // Ports, except on body cards (they carry no wire). An active input
        // that no wire feeds, the Sortie's included, is marked in red.
        let unwired = g.unwired_inputs();
        for n in g.nodes.iter().filter(|n| canvas::has_ports(g, n.id)) {
            for (i, ty) in n.kind.inputs().iter().enumerate() {
                port(
                    &painter,
                    to_screen(canvas::in_port_pos(n, i)),
                    *ty,
                    z,
                    self.hover == Hit::InPort(n.id, i),
                    unwired.contains(&(n.id, i as u32)),
                );
            }
            for (i, ty) in n.kind.outputs().iter().enumerate() {
                port(
                    &painter,
                    to_screen(canvas::out_port_pos(n, i)),
                    *ty,
                    z,
                    self.hover == Hit::OutPort(n.id, i),
                    false,
                );
            }
        }

        // The wire being drawn follows the pointer (or waits at the add
        // menu it opened), over the cards; red over a port it cannot take.
        let loose = match (self.wire(), &self.menu) {
            (Some(w), _) => ui.input(|i| i.pointer.latest_pos()).map(|p| (w, p)),
            (
                None,
                Some(Menu {
                    kind: MenuKind::Add { wire: Some(w), .. },
                    world,
                    ..
                }),
            ) => Some((*w, to_screen(*world))),
            _ => None,
        };
        if let Some((w, p)) = loose {
            if let Some(n) = g.node(w.end.0) {
                let fixed = if w.from_output {
                    canvas::out_port_pos(n, w.end.1 as usize)
                } else {
                    canvas::in_port_pos(n, w.end.1 as usize)
                };
                let fixed = to_screen(fixed);
                let bz = if w.from_output {
                    canvas::bezier(fixed, p)
                } else {
                    canvas::bezier(p, fixed)
                };
                let color = if self.drag.is_some() && refuses(g, &w, self.hover) {
                    theme::ERROR
                } else {
                    port_color(Some(w.ty))
                };
                painter.add(CubicBezierShape::from_points_stroke(
                    bz,
                    false,
                    Color32::TRANSPARENT,
                    Stroke::new(2.0, color),
                ));
            }
        }

        // Lasso.
        if let (Some(from), Some(at)) = (self.lasso_from, ui.input(|i| i.pointer.latest_pos())) {
            let r = Rect::from_two_pos(to_screen(from), at);
            painter.rect_filled(r, 0.0, theme::ACCENT.gamma_multiply(0.08));
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, theme::ACCENT), StrokeKind::Inside);
        }

        // The red flash of a refused wire.
        if let Some((at, t0)) = self.flash {
            let age = now - t0;
            if age < FLASH_S {
                let a = 1.0 - (age / FLASH_S) as f32;
                painter.circle_stroke(
                    to_screen(at),
                    (canvas::PORT_R + 6.0) * z,
                    Stroke::new(2.5, theme::ERROR.gamma_multiply(a)),
                );
                painter.circle_filled(
                    to_screen(at),
                    canvas::PORT_R * z,
                    theme::ERROR.gamma_multiply(a),
                );
                ui.ctx().request_repaint();
            } else {
                self.flash = None;
            }
        }

        self.menu_ui(ui, state, g);

        // Error message of the hovered card.
        if let Hit::Card(id) = self.hover {
            if let Some((_, msg)) = state.form.errors.iter().find(|(e, _)| *e == id) {
                resp.on_hover_text_at_pointer(msg.clone());
            }
        }
    }

    pub(super) fn card(
        &self,
        painter: &egui::Painter,
        state: &AppState,
        n: &Node,
        r: Rect,
        z: f32,
    ) {
        let error = state.form.errors.iter().any(|(id, _)| *id == n.id);
        let selected = state.form.selected.contains(&n.id);
        painter.rect_filled(r, CORNER * z, theme::PANEL);
        let (border, width) = if error {
            (theme::ERROR, 1.5)
        } else if selected {
            (theme::ACCENT, 1.5)
        } else if self.hover == Hit::Card(n.id) {
            (theme::TEXT_DIM, 1.0)
        } else {
            (theme::SEPARATOR, 1.0)
        };
        painter.rect_stroke(
            r,
            CORNER * z,
            Stroke::new(width, border),
            StrokeKind::Inside,
        );
        let title_color = if n.enabled {
            theme::TEXT
        } else {
            theme::TEXT_DIM
        };
        let title = match &n.kind {
            NodeKind::Iterate { iterations, .. } => format!("{}  ×{iterations}", n.name),
            _ => n.name.clone(),
        };
        let pad = theme::GRID * z;
        painter.text(
            Pos2::new(r.min.x + 1.5 * pad, r.min.y + canvas::HEADER_H * 0.5 * z),
            Align2::LEFT_CENTER,
            title,
            FontId::proportional(13.0 * z),
            title_color,
        );
    }
}

/// The 8 px grid, only when zoomed in enough to read it: faint lines, a
/// stronger one every 8 cells.
pub(super) fn grid(painter: &egui::Painter, rect: Rect, view: &CanvasView) {
    if view.zoom < GRID_MIN_ZOOM {
        return;
    }
    let step = theme::GRID;
    let w0 = view.to_world(rect.min, rect.min);
    let w1 = view.to_world(rect.min, rect.max);
    let stroke = |k: i64| {
        let c = if k % GRID_MAJOR == 0 {
            theme::SEPARATOR
        } else {
            theme::SEPARATOR.gamma_multiply(0.4)
        };
        Stroke::new(1.0, c)
    };
    let mut k = (w0.x / step).floor() as i64;
    while k as f32 * step <= w1.x {
        let sx = view.to_screen(rect.min, Pos2::new(k as f32 * step, 0.0)).x;
        painter.vline(sx, rect.y_range(), stroke(k));
        k += 1;
    }
    let mut k = (w0.y / step).floor() as i64;
    while k as f32 * step <= w1.y {
        let sy = view.to_screen(rect.min, Pos2::new(0.0, k as f32 * step)).y;
        painter.hline(rect.x_range(), sy, stroke(k));
        k += 1;
    }
}

/// Frame border: `SEPARATOR` lightened towards `TEXT_DIM` so the frame reads
/// against the canvas.
pub(super) fn frame_border() -> Color32 {
    theme::SEPARATOR.lerp_to_gamma(theme::TEXT_DIM, 0.35)
}

pub(super) fn port_color(ty: Option<PortType>) -> Color32 {
    match ty {
        Some(PortType::Distance) => canvas::WIRE_DIST,
        _ => canvas::WIRE_POINT,
    }
}

pub(super) fn port(
    painter: &egui::Painter,
    at: Pos2,
    ty: PortType,
    z: f32,
    hovered: bool,
    unwired: bool,
) {
    let r = canvas::PORT_R * z;
    let (fill, ring) = port_style(ty, hovered, unwired);
    painter.circle_filled(at, r, fill);
    if let Some(ring) = ring {
        painter.circle_stroke(at, r + 2.0, ring);
    }
}

/// Fill and ring of a port: its type colour, a red ring and fill when an
/// active input waits for a wire, a light ring under the pointer.
pub(super) fn port_style(ty: PortType, hovered: bool, unwired: bool) -> (Color32, Option<Stroke>) {
    if unwired {
        (theme::ERROR, Some(Stroke::new(1.5, theme::ERROR)))
    } else if hovered {
        (port_color(Some(ty)), Some(Stroke::new(1.0, theme::TEXT)))
    } else {
        (port_color(Some(ty)), None)
    }
}
