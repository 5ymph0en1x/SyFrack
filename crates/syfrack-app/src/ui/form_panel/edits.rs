//! Graph edits shared by menus, shortcuts and drags.

use super::*;

/// Adds `kind` where the menu opened, linked to `wire`: a wire lifted off an
/// input goes through the new node when it has the right output.
pub(super) fn add_node_at(
    state: &mut AppState,
    kind: NodeKind,
    world: Pos2,
    wire: Option<Wire>,
) -> bool {
    let half = canvas::HEADER_H * 0.5;
    let pos = match wire {
        Some(w) if !w.from_output => [snap(world.x - canvas::CARD_W), snap(world.y - half)],
        Some(_) => [snap(world.x), snap(world.y - half)],
        None => [snap(world.x), snap(world.y)],
    };
    let mut new_id = None;
    let ok = state.edit_graph(|g| {
        let id = graph_edit::add_node(g, kind.clone(), pos);
        new_id = Some(id);
        let Some(w) = wire else {
            return Ok(());
        };
        if !w.from_output {
            if let Some(o) = graph_edit::output_port_of(&kind, w.ty) {
                graph_edit::connect(g, (id, o), w.end)?;
            }
            return Ok(());
        }
        if let Some(i) = graph_edit::input_port_of(&kind, w.ty) {
            graph_edit::connect(g, w.end, (id, i))?;
        }
        if let Some(l) = w.lifted {
            let ty = g
                .node(l.0)
                .and_then(|n| n.kind.inputs().get(l.1 as usize).copied());
            match ty.and_then(|t| graph_edit::output_port_of(&kind, t)) {
                Some(o) => graph_edit::connect(g, (id, o), l)?,
                None => graph_edit::disconnect(g, l),
            }
        }
        Ok(())
    });
    if let (true, Some(id)) = (ok, new_id) {
        state.form.selected = vec![id];
    }
    ok
}

/// Deletes the selected cards (never the Entrée or the Sortie); a card
/// with one input and one output of the same type is bypassed, a Répéter
/// takes its body along.
pub(super) fn delete_selection(state: &mut AppState) -> bool {
    if state.form.selected.is_empty() {
        return false;
    }
    let sel = state.form.selected.clone();
    let ok = state.edit_graph(|g| {
        // A Répéter takes its body along.
        let doomed = with_bodies(g, &sel);
        graph_edit::dissolve_nodes(g, &doomed);
        Ok(())
    });
    if ok {
        let g = state.scene.fractal.to_graph();
        state.form.selected.retain(|id| g.node(*id).is_some());
    }
    ok
}

/// Duplicates the selected cards (bodies included) and selects the copies.
pub(super) fn duplicate_selection(state: &mut AppState) -> bool {
    if state.form.selected.is_empty() {
        return false;
    }
    let sel = state.form.selected.clone();
    let mut copies = Vec::new();
    let ok = state.edit_graph(|g| {
        copies = graph_edit::duplicate(g, &sel, DUPLICATE_OFFSET);
        Ok(())
    });
    if ok && !copies.is_empty() {
        let g = state.scene.fractal.to_graph();
        // The top-level copies: a copied body follows its Répéter.
        state.form.selected = copies
            .iter()
            .copied()
            .filter(|c| graph_edit::frame_of(&g, *c).is_none_or(|f| !copies.contains(&f)))
            .collect();
    }
    ok
}

/// Takes `id` out of its frame and sets it just right of the frame.
pub(crate) fn leave_frame(g: &mut Graph, id: NodeId) -> Result<(), syfrack_core::SceneError> {
    let right = graph_edit::frame_of(g, id)
        .and_then(|f| g.node(f))
        .map(|f| canvas::frame_rect(g, f).max.x);
    let owner = graph_edit::frame_of(g, id);
    graph_edit::move_out_of_frame(g, id);
    if let (Some(x), Some(n)) = (right, g.node_mut(id)) {
        n.pos[0] = snap(x + LEAVE_GAP);
    }
    restack(g, owner);
    Ok(())
}

/// Restacks the outermost frame holding `frame` (nested bodies follow), so
/// cards never overlap and their order on screen is the body order.
pub(super) fn restack(g: &mut Graph, frame: Option<NodeId>) {
    let Some(mut root) = frame else { return };
    let mut guard = 0;
    while let (Some(up), true) = (graph_edit::frame_of(g, root), guard < 16) {
        root = up;
        guard += 1;
    }
    canvas::restack_body(g, root);
}

/// End of a card move: a single Point → Point card dropped in a frame joins
/// its body at the height it was dropped, one dragged out of the frame
/// leaves it. Frames are taken as they were before the move.
pub(super) fn drop_in_frame(state: &mut AppState, origins: &[(NodeId, [f32; 2])]) -> bool {
    let [id] = state.form.selected[..] else {
        return false;
    };
    let g = state.scene.fractal.to_graph();
    let Some(n) = g.node(id) else {
        return false;
    };
    if !n.kind.is_point_to_point() {
        return false;
    }
    let mut before = g.clone();
    for (o, p) in origins {
        if let Some(n) = before.node_mut(*o) {
            n.pos = *p;
        }
    }
    let inside = with_bodies(&g, &[id]);
    let target = canvas::drop_target_frame_excluding(&before, canvas::card_rect(n), &inside);
    let current = graph_edit::frame_of(&g, id);
    let y = n.pos[1];
    // The body is restacked in the same step: no overlap, screen order =
    // body order (a card moved inside its own frame snaps back too).
    match (current, target) {
        (Some(c), Some(t)) if c == t => {
            let index = graph_edit::body_index_for(&g, c, y);
            state.edit_graph(|g| {
                graph_edit::move_into_frame(g, id, c, index)?;
                restack(g, Some(c));
                Ok(())
            })
        }
        (_, Some(t)) => {
            let index = graph_edit::body_index_for(&g, t, y);
            state.edit_graph(|g| {
                graph_edit::move_into_frame(g, id, t, index)?;
                restack(g, current);
                restack(g, Some(t));
                Ok(())
            })
        }
        (Some(c), None) => state.edit_graph(|g| {
            graph_edit::move_out_of_frame(g, id);
            restack(g, Some(c));
            Ok(())
        }),
        (None, None) => false,
    }
}
