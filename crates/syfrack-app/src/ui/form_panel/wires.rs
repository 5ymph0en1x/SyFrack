//! Ending a wire drag: link, flash or the add menu.

use super::*;

impl FormPanel {
    /// A wire released at `world`: linked on a port that takes it, a flash on
    /// one that does not, the add menu on the empty canvas.
    pub(super) fn finish_wire(
        &mut self,
        state: &mut AppState,
        w: Wire,
        screen: Pos2,
        world: Pos2,
        now: f64,
    ) -> bool {
        let g = state.scene.fractal.to_graph();
        let hit = canvas::hit_test(&g, world);
        let flash_at = |p: Pos2| Some((p, now));
        // The two ends of the link, from output to input.
        let ends = match hit {
            Hit::InPort(b, j) if w.from_output => Some((w.end, (b, j as u32))),
            Hit::OutPort(a, i) if !w.from_output => Some(((a, i as u32), w.end)),
            Hit::InPort(id, i) | Hit::OutPort(id, i) => {
                let n = g.node(id).expect("hit on a node");
                let p = if matches!(hit, Hit::InPort(..)) {
                    canvas::in_port_pos(n, i)
                } else {
                    canvas::out_port_pos(n, i)
                };
                self.flash = flash_at(p);
                return false;
            }
            // Dropped on a card body: its first port of the right type.
            Hit::Card(id) if id != w.end.0 => {
                let n = g.node(id).expect("hit on a node");
                let port = if w.from_output {
                    free_input(&g, n, w.ty)
                } else {
                    graph_edit::output_port_of(&n.kind, w.ty)
                };
                match port {
                    Some(p) if w.from_output => Some((w.end, (id, p))),
                    Some(p) => Some(((id, p), w.end)),
                    None => {
                        self.flash = flash_at(world);
                        return false;
                    }
                }
            }
            Hit::Card(_) => return false,
            Hit::Empty | Hit::Wire(_) | Hit::Frame(_) => {
                self.menu = Some(Menu {
                    screen,
                    world,
                    kind: MenuKind::Add {
                        query: String::new(),
                        wire: Some(w),
                    },
                    fresh: true,
                });
                return false;
            }
        };
        let Some((from, to)) = ends else {
            return false;
        };
        let out_ty = g
            .node(from.0)
            .and_then(|n| n.kind.outputs().get(from.1 as usize).copied());
        let in_ty = g
            .node(to.0)
            .and_then(|n| n.kind.inputs().get(to.1 as usize).copied());
        if out_ty.is_none() || out_ty != in_ty {
            // A mistyped wire writes nothing: the flash says why.
            let n = g.node(to.0).expect("hit on a node");
            self.flash = flash_at(canvas::in_port_pos(n, to.1 as usize));
            return false;
        }
        if w.lifted == Some(to) && g.input_of(to.0, to.1) == Some(from) {
            return false; // put back where it was
        }
        state.edit_graph(|g| {
            if let Some(l) = w.lifted.filter(|l| *l != to) {
                graph_edit::disconnect(g, l);
            }
            graph_edit::connect(g, from, to)
        })
    }
}

/// The input of `n` of type `ty` a dropped wire takes: the first free one,
/// else the first.
pub(super) fn free_input(g: &Graph, n: &Node, ty: PortType) -> Option<u32> {
    let ports: Vec<u32> = n
        .kind
        .inputs()
        .iter()
        .enumerate()
        .filter(|(_, t)| **t == ty)
        .map(|(i, _)| i as u32)
        .collect();
    ports
        .iter()
        .copied()
        .find(|p| g.input_of(n.id, *p).is_none())
        .or(ports.first().copied())
}

/// True when the port under the pointer cannot take the wire.
pub(super) fn refuses(g: &Graph, w: &Wire, hover: Hit) -> bool {
    let ty = match hover {
        Hit::InPort(id, i) if w.from_output => g.node(id).and_then(|n| n.kind.inputs().get(i)),
        Hit::OutPort(id, i) if !w.from_output => g.node(id).and_then(|n| n.kind.outputs().get(i)),
        Hit::InPort(..) | Hit::OutPort(..) => return true,
        _ => return false,
    };
    ty != Some(&w.ty)
}
