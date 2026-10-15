//! The add menu and the card menu.

use super::*;

impl FormPanel {
    /// A right click: the card menu on a card or frame (selecting it), the
    /// add menu elsewhere.
    pub(super) fn open_menu_at(
        &mut self,
        state: &mut AppState,
        g: &Graph,
        screen: Pos2,
        world: Pos2,
    ) {
        let kind = match canvas::hit_test(g, world) {
            Hit::Card(id) | Hit::Frame(id) => {
                if !state.form.selected.contains(&id) {
                    state.form.selected = vec![id];
                }
                MenuKind::Card(id)
            }
            _ => MenuKind::Add {
                query: String::new(),
                wire: None,
            },
        };
        self.drag = None;
        self.lasso_from = None;
        self.menu = Some(Menu {
            screen,
            world,
            kind,
            fresh: true,
        });
    }

    /// Closes the menu. A wire lifted off its port and dropped on the empty
    /// canvas is removed when no node took it.
    pub(super) fn close_menu(&mut self, state: &mut AppState) -> bool {
        let Some(menu) = self.menu.take() else {
            return false;
        };
        self.menu_rect = Rect::NOTHING;
        match menu.kind {
            MenuKind::Add {
                wire: Some(Wire {
                    lifted: Some(l), ..
                }),
                ..
            } => state.edit_graph(|g| {
                graph_edit::disconnect(g, l);
                Ok(())
            }),
            _ => false,
        }
    }

    pub(super) fn menu_ui(&mut self, ui: &Ui, state: &mut AppState, g: &Graph) {
        let Some(menu) = &mut self.menu else {
            return;
        };
        let mut action = None;
        let fresh = std::mem::replace(&mut menu.fresh, false);
        let area = egui::Area::new(Id::new("form-menu"))
            .order(egui::Order::Foreground)
            .layout(egui::Layout::top_down_justified(egui::Align::Min))
            .fixed_pos(menu.screen)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(MENU_W);
                    match &mut menu.kind {
                        MenuKind::Add { query, wire } => {
                            action = add_menu(ui, query, wire.as_ref(), fresh);
                        }
                        MenuKind::Card(id) => action = card_menu(ui, g, *id),
                    }
                    if ui.input(|i| i.key_pressed(Key::Escape)) {
                        action = Some(MenuAction::Cancel);
                    }
                });
            });
        self.menu_rect = area.response.rect;
        let Some(action) = action else {
            return;
        };
        let menu = self.menu.take().expect("drawn above");
        self.menu_rect = Rect::NOTHING;
        match action {
            MenuAction::Add(kind) => {
                let wire = match &menu.kind {
                    MenuKind::Add { wire, .. } => *wire,
                    MenuKind::Card(_) => None,
                };
                add_node_at(state, kind, menu.world, wire);
            }
            MenuAction::Duplicate => {
                duplicate_selection(state);
            }
            MenuAction::SetEnabled(on) => {
                let sel = state.form.selected.clone();
                state.edit_graph(|g| {
                    graph_edit::set_enabled(g, &sel, on);
                    Ok(())
                });
            }
            MenuAction::Rename(id) => {
                state.form.selected = vec![id];
                self.name_buf = None;
                self.focus_name = true;
            }
            MenuAction::Delete => {
                delete_selection(state);
            }
            MenuAction::LeaveFrame(id) => {
                state.edit_graph(|g| leave_frame(g, id));
            }
            MenuAction::Close => {
                self.menu = Some(menu);
                self.close_menu(state);
            }
            // Escape: a lifted wire goes back where it was.
            MenuAction::Cancel => {}
        }
    }
}

/// The add menu: a search field, then the matching nodes by category (only
/// those that can take `wire`). Enter picks the first match.
pub(super) fn add_menu(
    ui: &mut Ui,
    query: &mut String,
    wire: Option<&Wire>,
    fresh: bool,
) -> Option<MenuAction> {
    let mut action = None;
    let field = ui.add(
        egui::TextEdit::singleline(query)
            .hint_text(syfrack_i18n::tr!("Rechercher un nœud…"))
            .desired_width(f32::INFINITY),
    );
    if fresh {
        field.request_focus();
    }
    let matches = matching_kinds(query, wire);
    if field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
        if let Some(k) = matches.first() {
            action = Some(MenuAction::Add((*k).clone()));
        }
    }
    ui.add_space(theme::GRID / 2.0);
    egui::ScrollArea::vertical()
        .max_height(MENU_LIST_H)
        .show(ui, |ui| {
            for (c, title) in CATEGORIES.iter().enumerate() {
                let in_cat: Vec<&&NodeKind> = matches
                    .iter()
                    .filter(|k| graph_edit::category(k) == c)
                    .collect();
                if in_cat.is_empty() {
                    continue;
                }
                ui.label(
                    RichText::new(syfrack_i18n::text(title))
                        .small()
                        .color(theme::TEXT_DIM),
                );
                for k in in_cat {
                    if ui.button(k.label()).clicked() {
                        action = Some(MenuAction::Add((**k).clone()));
                    }
                }
                ui.add_space(theme::GRID / 2.0);
            }
            if matches.is_empty() {
                ui.label(RichText::new(syfrack_i18n::tr!("Aucun nœud")).color(theme::TEXT_DIM));
            }
        });
    action
}

/// The nodes whose label contains `query` (any case) and that have a port
/// for `wire`.
pub fn matching_kinds(query: &str, wire: Option<&Wire>) -> Vec<&'static NodeKind> {
    let q = query.trim().to_lowercase();
    ALL_KINDS
        .iter()
        .filter(|k| k.label().to_lowercase().contains(&q))
        .filter(|k| match wire {
            Some(w) if w.from_output => graph_edit::input_port_of(k, w.ty).is_some(),
            Some(w) => graph_edit::output_port_of(k, w.ty).is_some(),
            None => true,
        })
        .collect()
}

/// The card menu: duplicate, enable or disable, rename, delete, and leave
/// the frame for a card inside one.
pub(super) fn card_menu(ui: &mut Ui, g: &Graph, id: NodeId) -> Option<MenuAction> {
    let Some(n) = g.node(id) else {
        return Some(MenuAction::Close);
    };
    let io = matches!(n.kind, NodeKind::Input | NodeKind::Output);
    let mut action = None;
    let item = |ui: &mut Ui, label: &str, shortcut: &str, enabled: bool| {
        ui.add_enabled(
            enabled,
            egui::Button::new(label).shortcut_text(RichText::new(shortcut).color(theme::TEXT_DIM)),
        )
        .clicked()
    };
    if item(ui, syfrack_i18n::tr!("Dupliquer"), "Ctrl+D", !io) {
        action = Some(MenuAction::Duplicate);
    }
    let toggle = if n.enabled {
        syfrack_i18n::tr!("Désactiver")
    } else {
        syfrack_i18n::tr!("Activer")
    };
    if item(ui, toggle, "", !io) {
        action = Some(MenuAction::SetEnabled(!n.enabled));
    }
    if item(ui, syfrack_i18n::tr!("Renommer"), "", true) {
        action = Some(MenuAction::Rename(id));
    }
    if graph_edit::frame_of(g, id).is_some()
        && item(ui, syfrack_i18n::tr!("Sortir du cadre"), "", true)
    {
        action = Some(MenuAction::LeaveFrame(id));
    }
    if item(
        ui,
        syfrack_i18n::tr!("Supprimer"),
        syfrack_i18n::tr!("Suppr"),
        !io,
    ) {
        action = Some(MenuAction::Delete);
    }
    action
}
