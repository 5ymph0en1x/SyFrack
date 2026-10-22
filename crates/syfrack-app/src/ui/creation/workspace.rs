//! Presentation and direct manipulation of the unified creation graph.
use super::*;
use egui::{Align2, Key, PointerButton, RichText, StrokeKind};

fn type_name(t: SocketType) -> &'static str {
    match t {
        SocketType::Point => "Point 3D",
        SocketType::Distance => "Distance",
        SocketType::Scalar => "Valeur",
        SocketType::Vector => "Vecteur / couleur",
        SocketType::Complex => "Complexe",
        SocketType::Texture => "Surface",
        SocketType::State => "État",
    }
}
fn family(kind: &CreationKind) -> (&'static str, Color32) {
    match kind {
        CreationKind::Output => ("SORTIE", theme::ACCENT),
        CreationKind::Form(_) => ("GÉOMÉTRIE", color(SocketType::Distance)),
        CreationKind::Simulation(_) | CreationKind::SampleState { .. } => {
            ("SIMULATION", color(SocketType::State))
        }
        CreationKind::Palette(_) | CreationKind::Mix => ("COULEUR", color(SocketType::Texture)),
        CreationKind::Pattern { .. } | CreationKind::Texture(_) | CreationKind::Flame(_) => {
            ("SOURCE", color(SocketType::Complex))
        }
        _ => ("CHAMP", color(SocketType::Scalar)),
    }
}
fn socket_name(g: &CreationGraph, s: Socket) -> String {
    g.node(s.node)
        .map(|n| {
            let ports = if s.output {
                n.kind.output_labels()
            } else {
                n.kind.input_labels()
            };
            format!(
                "{} · {}",
                label(&n.name),
                label(ports.get(s.port as usize).copied().unwrap_or("?"))
            )
        })
        .unwrap_or_default()
}

fn library_button(
    ui: &mut egui::Ui,
    name: &str,
    kind: &CreationKind,
    context: Option<SocketType>,
) -> egui::Response {
    let (inputs, outputs) = workflow::library_signature(kind, context);
    let padding = ui.spacing().button_padding;
    // Reserve room inside the button so long names cannot cover the dots.
    ui.spacing_mut().button_padding.x =
        18.0 + inputs.len().max(outputs.len()).saturating_sub(1) as f32 * 9.0;
    let response = ui.add_sized(
        Vec2::new(ui.available_width(), 28.0),
        egui::Button::new(label(name)).truncate(),
    );
    ui.spacing_mut().button_padding = padding;
    for (types, output) in [(&inputs, false), (&outputs, true)] {
        for (i, t) in types.iter().enumerate() {
            let x = if output {
                response.rect.right() - 10.0 - (types.len() - 1 - i) as f32 * 9.0
            } else {
                response.rect.left() + 10.0 + i as f32 * 9.0
            };
            ui.painter()
                .circle_filled(Pos2::new(x, response.rect.center().y), 3.5, color(*t));
        }
    }
    let describe = |types: &[SocketType]| {
        if types.is_empty() {
            "—".to_owned()
        } else {
            types
                .iter()
                .map(|t| label(type_name(*t)))
                .collect::<Vec<_>>()
                .join(" / ")
        }
    };
    response.on_hover_text(format!(
        "{}\nIN : {}\nOUT : {}\n{}",
        label(name),
        describe(&inputs),
        describe(&outputs),
        label("Ajouter et raccorder à la prise choisie, ou prolonger l’étape sélectionnée.")
    ))
}
pub(super) fn toolbar(
    ui: &mut egui::Ui,
    state: &mut AppState,
    g: &mut CreationGraph,
    form: &FormPanel,
    editor: &mut Editor,
) {
    ui.horizontal_wrapped(|ui| {
        ui.strong(label("Graphe de création"));
        ui.separator();
        if ui.selectable_label(!editor.library_hidden, label("Bibliothèque")).clicked() { editor.library_hidden = !editor.library_hidden; }
        if ui.button(label("Tout cadrer")).on_hover_text(label("F · Cadrer tous les nœuds")).clicked() { fit(state, g, form.rect.size()); }
        if ui.button(label("Organiser")).on_hover_text(label("Répartir les étapes et démêler les branches sans modifier les traitements.")).clicked() { layout(g); fit(state, g, form.rect.size()); }
        ui.add_enabled_ui(!state.form.selected.is_empty(), |ui| {
            if ui.button(label("Cadrer la sélection")).clicked() { fit_selection(state, g, form.rect.size()); }
            ui.toggle_value(&mut editor.focus_branch, label("Isoler le parcours"))
                .on_hover_text(label("Atténuer les autres branches pour suivre le traitement sélectionné jusqu’à la Scène."));
        });
        ui.menu_button(label("Composer"), |ui| {
            ui.small(label("Ajouter une branche complète et modifiable"));
            for (recipe,name) in [(0,"Motif et palette"),(1,"Interférences colorées"),(2,"Flamme : couleur et densité")] {
                if ui.button(label(name)).on_hover_text(label("Relie la nouvelle branche à la Scène. Les autres nœuds sont conservés ; Ctrl+Z annule." )).clicked() {
                    match workflow::compose(g,recipe) {
                        Ok(ids) => { state.form.selected = ids; fit_selection(state,g,form.rect.size()); editor.message = None; },
                        Err(e) => editor.message = Some(e.to_string()),
                    }
                    ui.close();
                }
            }
        });
        ui.menu_button(label("Actions"), |ui| {
            if ui.add_enabled(!state.form.selected.is_empty(), egui::Button::new(label("Dupliquer"))).clicked() { duplicate(g, &mut state.form.selected); ui.close(); }
            if ui.add_enabled(!state.form.selected.is_empty(), egui::Button::new(label("Retirer en raccordant"))).on_hover_text(label("Maj+Suppr · Raccorder les voisins des traitements à une entrée et une sortie compatibles.")).clicked() { workflow::dissolve(g, &state.form.selected); state.form.selected.clear(); ui.close(); }
            if ui.add_enabled(!state.form.selected.is_empty(), egui::Button::new(label("Supprimer"))).clicked() { remove(g, &mut state.form.selected); ui.close(); }
            ui.separator();
            if ui.add_enabled(editor.task.is_none() && editor.thumbnail_task.is_none(), egui::Button::new(label("Figer les sources GPU"))).clicked() {
                if let Some(runtime) = super::super::phenomena::shared_runtime(ui.ctx()) {
                    let source = state.scene.clone();
                    let (tx,rx) = mpsc::channel();
                    editor.task = Some(Arc::new(Mutex::new(rx)));
                    editor.cancel = Arc::new(AtomicBool::new(false));
                    let control = syfrack_render::phenomena::PreparationControl { paused: runtime.paused, cancelled: editor.cancel.clone() };
                    std::thread::spawn(move || { let result = runtime.renderer.freeze_creation_sources_controlled(&source,&control).map(|frozen|(source,frozen)).map_err(|e|format!("{e:#}")); let _ = tx.send(result); });
                } else { editor.message = Some(label("Le moteur GPU doit être disponible pour ce calcul.")); }
                ui.close();
            }
        });
        if editor.task.is_some() || editor.thumbnail_task.is_some() {
            ui.spinner();
            if ui.button(label("Annuler")).clicked() { editor.cancel.store(true, Ordering::Relaxed); }
        }
    });
    ui.horizontal_wrapped(|ui| {
        let draft = state.scene.creation_draft.is_some();
        ui.colored_label(
            if draft {
                color(SocketType::Scalar)
            } else {
                theme::ACCENT
            },
            label(if draft {
                "Brouillon · dernier rendu valide conservé"
            } else {
                "Scène à jour"
            }),
        );
        ui.separator();
        ui.small(format!(
            "{} / {} {} · {} {}",
            active_nodes(g).len(),
            g.nodes.len(),
            label("nœuds actifs"),
            g.links.len(),
            label("liaisons")
        ));
        if let Some(s) = editor.wire {
            ui.separator();
            ui.colored_label(
                theme::ACCENT,
                format!("{} {}", label("Relier :"), socket_name(g, s)),
            );
            if ui.small_button(label("Annuler")).clicked() {
                editor.wire = None;
            }
        }
    });
    if let Some(message) = editor.message.clone() {
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(theme::ERROR, message);
            if ui.small_button("×").clicked() {
                editor.message = None;
            }
        });
    }
}

pub(super) fn library(
    ui: &mut egui::Ui,
    state: &mut AppState,
    g: &mut CreationGraph,
    form: &mut FormPanel,
    editor: &mut Editor,
) {
    ui.heading(label("Bibliothèque"));
    ui.horizontal(|ui| {
        if ui
            .selectable_label(!editor.library_add, label("Parcours"))
            .clicked()
        {
            editor.library_add = false;
            editor.query.clear();
        }
        if ui
            .selectable_label(editor.library_add, label("Ajouter"))
            .clicked()
        {
            editor.library_add = true;
            editor.query.clear();
        }
    });
    let search = ui.add(
        egui::TextEdit::singleline(&mut editor.query)
            .desired_width(f32::INFINITY)
            .hint_text(label("Rechercher…")),
    );
    if ui
        .ctx()
        .data_mut(|d| d.remove_temp::<bool>(egui::Id::new("creation-search-focus")))
        .unwrap_or(false)
    {
        search.request_focus();
    }
    if let Some(s) = editor.wire.filter(|_| editor.library_add) {
        ui.colored_label(
            theme::ACCENT,
            label("Nœuds compatibles avec la prise choisie"),
        );
        ui.small(socket_name(g, s));
    } else {
        ui.small(label("Sélectionnez une étape pour prolonger sa chaîne."));
    }
    ui.add_space(6.0);
    let query = editor.query.to_lowercase();
    if !editor.library_add {
        let active = active_nodes(g);
        ui.small(label(
            "Cliquez une étape pour la sélectionner et la cadrer.",
        ));
        egui::ScrollArea::vertical()
            .id_salt("creation-outline")
            .show(ui, |ui| {
                for id in g
                    .order()
                    .unwrap_or_else(|_| g.nodes.iter().map(|n| n.id).collect())
                {
                    let Some(n) = g.node(id) else {
                        continue;
                    };
                    if !query.is_empty()
                        && !label(&n.name).to_lowercase().contains(&query)
                        && !label(&n.kind.label()).to_lowercase().contains(&query)
                    {
                        continue;
                    }
                    let (_, c) = family(&n.kind);
                    let response = ui.add(
                        egui::Button::new(RichText::new(label(&n.name)).color(
                            if active.contains(&id) {
                                c
                            } else {
                                theme::TEXT_DIM
                            },
                        ))
                        .selected(state.form.selected.contains(&id))
                        .min_size(Vec2::new(ui.available_width(), 30.0))
                        .truncate(),
                    );
                    if response
                        .on_hover_text(format!("{} · {}", label(&n.name), label(&n.kind.label())))
                        .clicked()
                    {
                        state.form.selected = vec![id];
                        fit_selection(state, g, form.rect.size());
                    }
                }
            });
        return;
    }
    if let Some(s) = editor.wire {
        workflow::candidates(g, s, &g.types().unwrap_or_default(), editor);
    }
    let context = editor
        .wire
        .or_else(|| {
            state
                .form
                .selected
                .last()
                .map(|id| Socket::new(*id, 0, true))
        })
        .and_then(|s| socket_type(g, s, &g.types().unwrap_or_default()));
    let (legend, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 16.0), Sense::hover());
    ui.painter().text(
        legend.left_center() + Vec2::new(6.0, 0.0),
        Align2::LEFT_CENTER,
        "IN",
        FontId::proportional(10.0),
        theme::TEXT_DIM,
    );
    ui.painter().text(
        legend.right_center() - Vec2::new(6.0, 0.0),
        Align2::RIGHT_CENTER,
        "OUT",
        FontId::proportional(10.0),
        theme::TEXT_DIM,
    );
    egui::ScrollArea::vertical()
        .id_salt("creation-nodes")
        .show(ui, |ui| {
            let mut found = false;
            for (category, items) in catalogue() {
                let filtered: Vec<_> = items
                    .into_iter()
                    .filter(|(name, kind)| {
                        (query.is_empty()
                            || name.to_lowercase().contains(&query)
                            || label(name).to_lowercase().contains(&query)
                            || label(category).to_lowercase().contains(&query))
                            && editor.wire.is_none_or(|s| {
                                *editor
                                    .addable
                                    .entry((s, category.to_string(), name.clone()))
                                    .or_insert_with(|| workflow::attachment(g, kind, s).is_some())
                            })
                    })
                    .collect();
                if filtered.is_empty() {
                    continue;
                }
                found = true;
                egui::CollapsingHeader::new(RichText::new(label(category)).color(theme::TEXT_DIM))
                    .default_open(true)
                    .show(ui, |ui| {
                        for (name, kind) in filtered {
                            let response = library_button(ui, &name, &kind, context);
                            if response.clicked() {
                                let anchor = editor.wire.take();
                                add_connected(g, kind, state, form.rect, anchor);
                                fit_neighborhood(state, g, form.rect.size());
                                editor.message = None;
                            }
                        }
                    });
            }
            if !found {
                ui.label(label("Aucun nœud compatible avec cette recherche."));
            }
            ui.separator();
            egui::CollapsingHeader::new(label("Formes fractales")).show(ui, |ui| {
                for entry in form.creation_forms() {
                    if ui.button(entry.localized_name()).clicked() {
                        append_form(g, &entry.graph, state);
                        editor.framed = false;
                    }
                }
            });
            egui::CollapsingHeader::new(label("Modules naturels")).show(ui, |ui| {
                for (name, title) in presets::PRESETS {
                    if ui.button(syfrack_i18n::text(title)).clicked() {
                        match presets::scene(name) {
                            Ok(s) => {
                                append_fields(g, &CreationGraph::from_scene(&s));
                                editor.framed = false;
                            }
                            Err(e) => editor.message = Some(e.to_string()),
                        }
                    }
                }
                ui.small(label(
                    "Les modules s’ajoutent au graphe. Reliez leurs sorties pour les combiner.",
                ));
            });
        });
}

pub(super) fn connections(
    ui: &mut egui::Ui,
    state: &mut AppState,
    g: &mut CreationGraph,
    id: u32,
    editor: &mut Editor,
) {
    let Some(n) = g.node(id).cloned() else {
        return;
    };
    let types = g.types().unwrap_or_default();
    let active = active_nodes(g).contains(&id);
    ui.heading(label(&n.kind.label()));
    if let Some(node) = g.nodes.iter_mut().find(|n| n.id == id) {
        ui.text_edit_singleline(&mut node.name);
    }
    ui.colored_label(
        if active {
            theme::ACCENT
        } else {
            theme::TEXT_DIM
        },
        label(if active {
            "Contribue à l’image"
        } else {
            "Hors du parcours de rendu"
        }),
    );
    ui.small(description(&n.kind));
    egui::CollapsingHeader::new(label("Connexions"))
        .default_open(n.kind.input_labels().len() <= 3)
        .show(ui, |ui| {
            for (i, name) in n.kind.input_labels().iter().enumerate() {
                let target = Socket::new(id, i as u32, false);
                let current = g.input(id, i as u32);
                ui.colored_label(
                    color(socket_type(g, target, &types).unwrap_or(SocketType::Scalar)),
                    label(name),
                );
                let candidates = workflow::candidates(g, target, &types, editor);
                let mut chosen = None;
                egui::ComboBox::from_id_salt(("creation-connection", id, i))
                    .width(ui.available_width() - 36.0)
                    .selected_text(
                        current
                            .map(|(a, b)| socket_name(g, Socket::new(a, b, true)))
                            .unwrap_or_else(|| label("Non reliée")),
                    )
                    .show_ui(ui, |ui| {
                        for source in candidates {
                            if ui
                                .selectable_label(
                                    current == Some(source.end()),
                                    socket_name(g, source),
                                )
                                .clicked()
                            {
                                chosen = Some(source);
                            }
                        }
                    });
                ui.horizontal(|ui| {
                    if ui.small_button(label("Choisir dans le graphe")).clicked() {
                        editor.wire = Some(target);
                        editor.library_add = true;
                        editor.library_hidden = false;
                    }
                    if ui
                        .add_enabled(
                            current.is_some(),
                            egui::Button::new(label("Débrancher")).small(),
                        )
                        .clicked()
                    {
                        g.links.retain(|l| l.to != target.end());
                    }
                });
                if let Some(source) = chosen {
                    match workflow::connect(g, source, target) {
                        Ok(()) => editor.message = None,
                        Err(e) => editor.message = Some(e.to_string()),
                    }
                }
                ui.add_space(3.0);
            }
            for (i, name) in n.kind.output_labels().iter().enumerate() {
                let source = Socket::new(id, i as u32, true);
                ui.horizontal(|ui| {
                    ui.colored_label(
                        color(
                            types
                                .get(&source.end())
                                .copied()
                                .unwrap_or(SocketType::Scalar),
                        ),
                        format!("{} →", label(name)),
                    );
                    if ui.small_button(label("Relier")).clicked() {
                        editor.wire = Some(source);
                        editor.library_add = true;
                        editor.library_hidden = false;
                    }
                });
                for link in g.links.iter().filter(|l| l.from == source.end()) {
                    if ui
                        .small_button(socket_name(g, Socket::new(link.to.0, link.to.1, false)))
                        .clicked()
                    {
                        state.form.selected = vec![link.to.0];
                    }
                }
            }
        });
    ui.separator();
}

fn description(kind: &CreationKind) -> String {
    label(match kind {
        CreationKind::Output => "La Scène réunit les branches. Les entrées non utilisées sont facultatives selon la composition 2D ou 3D.",
        CreationKind::Palette(_) => "Transforme une valeur en couleur. Reliez un motif à Valeur, puis la sortie à Couleur sur la Scène.",
        CreationKind::Flame(_) => "Les orbites produisent couleur et densité. Leur finesse dépend aussi de la résolution et du budget de points de la source.",
        CreationKind::Simulation(_) => "Fait évoluer un état. La résolution du domaine et le pas de temps déterminent les détails disponibles.",
        CreationKind::Pattern { .. } | CreationKind::Texture(_) => "Évalue un motif fractal aux coordonnées de Position. Valeur peut piloter une palette, une densité ou un relief.",
        CreationKind::Form(_) => "Transforme la géométrie. Les prises Point 3D et Distance restent distinctes pour préserver le calcul de la forme.",
        _ => "Combine ou transforme les champs. Les prises compatibles sont mises en évidence pendant un raccordement.",
    })
}

pub(super) fn canvas(
    ui: &mut egui::Ui,
    state: &mut AppState,
    g: &mut CreationGraph,
    editor: &mut Editor,
    rect: Rect,
) {
    let size_changed = editor.canvas_size != Vec2::ZERO
        && ((editor.canvas_size.x - rect.width()).abs() > 16.0
            || (editor.canvas_size.y - rect.height()).abs() > 96.0);
    if editor.framed && size_changed {
        if state.form.selected.len() == 1 {
            fit_selection(state, g, rect.size());
        } else {
            fit(state, g, rect.size());
        }
    }
    editor.canvas_size = rect.size();
    if !editor.framed {
        fit(state, g, rect.size());
        if g.nodes.len() > 20 && state.form.view.zoom < 0.45 {
            if state.form.selected.is_empty() {
                if let Some(n) = g
                    .nodes
                    .iter()
                    .find(|n| matches!(n.kind, CreationKind::Output))
                {
                    state.form.selected = vec![n.id];
                }
            }
            fit_neighborhood(state, g, rect.size());
        }
        editor.framed = true;
    }
    if editor.wire.is_some_and(|s| g.node(s.node).is_none()) {
        editor.wire = None;
    }
    let background = ui.interact(
        rect,
        egui::Id::new("creation-canvas"),
        Sense::click_and_drag(),
    );
    let pan = ui.input(|i| i.key_down(Key::Space));
    if background.dragged_by(PointerButton::Middle) || (background.dragged() && pan) {
        state.form.view.offset += ui.input(|i| i.pointer.delta()) / state.form.view.zoom;
    }
    if let Some(pointer) = ui
        .input(|i| i.pointer.hover_pos())
        .filter(|p| rect.contains(*p))
    {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            let before = state.form.view.to_world(rect.min, pointer);
            state.form.view.zoom =
                (state.form.view.zoom * (scroll * 0.0015).exp()).clamp(0.001, 3.0);
            let after = state.form.view.to_world(rect.min, pointer);
            state.form.view.offset += after - before;
        }
    }
    let view = state.form.view;
    let screen = |p| view.to_screen(rect.min, p);
    let painter = ui.painter_at(rect);
    let step = (32.0 * view.zoom).max(16.0);
    let origin = screen(Pos2::ZERO);
    let mut x = rect.left() + (origin.x - rect.left()).rem_euclid(step);
    while x < rect.right() {
        let mut y = rect.top() + (origin.y - rect.top()).rem_euclid(step);
        while y < rect.bottom() {
            painter.circle_filled(Pos2::new(x, y), 0.7, theme::SEPARATOR.gamma_multiply(0.6));
            y += step;
        }
        x += step;
    }
    let types = g.types().unwrap_or_default();
    let targets: BTreeSet<_> = editor
        .wire
        .map(|s| workflow::candidates(g, s, &types, editor))
        .unwrap_or_default()
        .into_iter()
        .collect();
    let active = active_nodes(g);
    let branch = workflow::branch(g, &state.form.selected);
    let focused = editor.focus_branch && !state.form.selected.is_empty();
    let bodies: BTreeSet<_> = g
        .nodes
        .iter()
        .filter_map(|n| match &n.kind {
            CreationKind::Form(NodeKind::Iterate { body, .. }) => Some(body.iter().copied()),
            _ => None,
        })
        .flatten()
        .collect();
    for n in &g.nodes {
        if let CreationKind::Form(NodeKind::Iterate { body, .. }) = &n.kind {
            let bounds = body
                .iter()
                .filter_map(|id| g.node(*id))
                .map(|n| {
                    Rect::from_min_size(
                        screen(Pos2::from(n.pos)),
                        Vec2::new(WIDTH, height(n)) * view.zoom,
                    )
                })
                .reduce(Rect::union);
            if let Some(bounds) = bounds {
                let group = bounds.expand(18.0 * view.zoom);
                painter.rect_filled(
                    group,
                    8.0,
                    Color32::from_rgba_unmultiplied(110, 82, 150, 15),
                );
                painter.rect_stroke(
                    group,
                    8.0,
                    Stroke::new(1.0, color(SocketType::State).gamma_multiply(0.5)),
                    StrokeKind::Inside,
                );
                painter.text(
                    group.left_top() - Vec2::new(0.0, 6.0),
                    Align2::LEFT_BOTTOM,
                    format!("{} · {}", label(&n.name), label("Étapes de la boucle")),
                    FontId::proportional(12.0 * view.zoom),
                    theme::TEXT_DIM,
                );
            }
        }
    }
    for l in &g.links {
        if let (Some(a), Some(b)) = (g.node(l.from.0), g.node(l.to.0)) {
            let c = color(types.get(&l.from).copied().unwrap_or(SocketType::Scalar));
            let highlighted =
                branch.contains(&a.id) && branch.contains(&b.id) && !state.form.selected.is_empty();
            let alpha = if focused && !highlighted {
                0.12
            } else if active.contains(&b.id) || highlighted {
                0.85
            } else {
                0.3
            };
            let points = crate::canvas::bezier(
                screen(port(a, l.from.1, true)),
                screen(port(b, l.to.1, false)),
            );
            if highlighted {
                painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
                    points,
                    false,
                    Color32::TRANSPARENT,
                    Stroke::new(6.0, c.gamma_multiply(0.12)),
                ));
            }
            painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
                points,
                false,
                Color32::TRANSPARENT,
                Stroke::new(if highlighted { 2.6 } else { 1.6 }, c.gamma_multiply(alpha)),
            ));
        }
    }
    if let (Some(s), Some(pointer)) = (editor.wire, ui.input(|i| i.pointer.hover_pos())) {
        if let Some(n) = g.node(s.node) {
            let fixed = screen(port(n, s.port, s.output));
            let (a, b) = if s.output {
                (fixed, pointer)
            } else {
                (pointer, fixed)
            };
            let c = color(socket_type(g, s, &types).unwrap_or(SocketType::Scalar));
            painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
                crate::canvas::bezier(a, b),
                false,
                Color32::TRANSPARENT,
                Stroke::new(2.2, c),
            ));
        }
    }
    let mut landed = false;
    for n in g.nodes.clone() {
        let card = Rect::from_min_size(
            screen(Pos2::from(n.pos)),
            Vec2::new(WIDTH, height(&n)) * view.zoom,
        );
        if !card.intersects(rect) {
            continue;
        }
        let selected = state.form.selected.contains(&n.id);
        let dim = focused && !branch.contains(&n.id);
        let (category, accent) = family(&n.kind);
        let text = if dim {
            theme::TEXT_DIM.gamma_multiply(0.45)
        } else {
            theme::TEXT
        };
        painter.rect_filled(
            card.translate(Vec2::new(0.0, 3.0)),
            8.0,
            Color32::from_black_alpha(65),
        );
        painter.rect_filled(
            card,
            8.0,
            if selected {
                Color32::from_rgb(31, 48, 53)
            } else {
                theme::PANEL
            },
        );
        painter.rect_stroke(
            card,
            8.0,
            Stroke::new(
                if selected { 1.8 } else { 1.0 },
                if selected { accent } else { theme::SEPARATOR },
            ),
            StrokeKind::Inside,
        );
        painter.vline(
            card.left() + 2.0,
            card.top() + 10.0..=card.bottom() - 10.0,
            Stroke::new(2.0, accent.gamma_multiply(if dim { 0.2 } else { 0.8 })),
        );
        let title = Rect::from_min_size(card.min, Vec2::new(card.width(), HEADER * view.zoom));
        let title_painter = painter.with_clip_rect(
            title
                .shrink2(Vec2::new(12.0 * view.zoom, 0.0))
                .intersect(rect),
        );
        title_painter.text(
            title.left_top() + Vec2::new(14.0, 12.0) * view.zoom,
            Align2::LEFT_TOP,
            label(category),
            FontId::proportional(9.0 * view.zoom),
            accent.gamma_multiply(if dim { 0.3 } else { 1.0 }),
        );
        title_painter.text(
            title.left_top() + Vec2::new(14.0, 28.0) * view.zoom,
            Align2::LEFT_TOP,
            label(&n.name),
            FontId::proportional(14.0 * view.zoom),
            text,
        );
        painter.hline(
            card.x_range(),
            title.bottom(),
            Stroke::new(1.0, theme::SEPARATOR.gamma_multiply(0.6)),
        );
        let response = ui.interact(
            if pan { Rect::NOTHING } else { title },
            egui::Id::new(("creation-card", n.id)),
            Sense::click_and_drag(),
        );
        if response.clicked() || response.drag_started() {
            let extend = ui.input(|i| i.modifiers.ctrl || i.modifiers.shift);
            if !extend && !selected {
                state.form.selected.clear();
            }
            if !selected {
                state.form.selected.push(n.id);
            } else if extend && response.clicked() {
                state.form.selected.retain(|id| *id != n.id);
            }
        }
        if response.double_clicked() {
            state.form.selected = vec![n.id];
            fit_selection(state, g, rect.size());
        }
        response.clone().on_hover_text(label(&n.name));
        if response.drag_started() {
            state.begin_drag();
            let mut moving: BTreeSet<_> = state.form.selected.iter().copied().collect();
            loop {
                let before = moving.len();
                for n in &g.nodes {
                    if moving.contains(&n.id) {
                        if let CreationKind::Form(NodeKind::Iterate { body, .. }) = &n.kind {
                            moving.extend(body);
                        }
                    }
                }
                if before == moving.len() {
                    break;
                }
            }
            editor.drag = ui.input(|i| i.pointer.interact_pos()).map(|p| {
                (
                    p,
                    g.nodes
                        .iter()
                        .filter(|n| moving.contains(&n.id))
                        .map(|n| (n.id, n.pos))
                        .collect(),
                )
            });
        }
        if response.dragged() {
            if let (Some((start, origins)), Some(p)) =
                (&editor.drag, ui.input(|i| i.pointer.interact_pos()))
            {
                let delta = (p - *start) / view.zoom;
                for (id, pos) in origins {
                    if let Some(n) = g.nodes.iter_mut().find(|n| n.id == *id) {
                        n.pos = [pos[0] + delta.x, pos[1] + delta.y];
                    }
                }
            }
        }
        response.context_menu(|ui| {
            if ui.button(label("Cadrer la sélection")).clicked() {
                state.form.selected = vec![n.id];
                fit_selection(state, g, rect.size());
                ui.close();
            }
            if ui.button(label("Dupliquer")).clicked() {
                state.form.selected = vec![n.id];
                duplicate(g, &mut state.form.selected);
                ui.close();
            }
            if !matches!(n.kind, CreationKind::Output)
                && ui.button(label("Retirer en raccordant")).clicked()
            {
                workflow::dissolve(g, &[n.id]);
                state.form.selected.clear();
                ui.close();
            }
        });
        if !bodies.contains(&n.id) {
            for output in [false, true] {
                let names = if output {
                    n.kind.output_labels()
                } else {
                    n.kind.input_labels()
                };
                for (i, name) in names.iter().enumerate() {
                    let s = Socket::new(n.id, i as u32, output);
                    let p = screen(port(&n, i as u32, output));
                    let ty = socket_type(g, s, &types).unwrap_or(SocketType::Scalar);
                    let compatible = editor
                        .wire
                        .is_none_or(|fixed| fixed == s || targets.contains(&s));
                    let c = color(ty).gamma_multiply(if !compatible || dim { 0.22 } else { 1.0 });
                    let hit = Rect::from_min_max(
                        Pos2::new(card.left() - 10.0, p.y - ROW * view.zoom * 0.5),
                        Pos2::new(card.right() + 10.0, p.y + ROW * view.zoom * 0.5),
                    )
                    .intersect(rect);
                    let response = ui.interact(
                        if pan { Rect::NOTHING } else { hit },
                        egui::Id::new(("creation-socket", n.id, i, output)),
                        Sense::click_and_drag(),
                    );
                    if response.hovered() && compatible {
                        painter.rect_filled(hit.shrink(1.0), 3.0, c.gamma_multiply(0.1));
                    }
                    let wired = output || g.input(n.id, i as u32).is_some();
                    painter.circle_filled(p, 6.0 * view.zoom, theme::BG);
                    if wired {
                        painter.circle_filled(p, 4.2 * view.zoom, c);
                    } else {
                        painter.circle_stroke(p, 4.2 * view.zoom, Stroke::new(1.5, c));
                    }
                    if editor.wire.is_some() && compatible {
                        painter.circle_stroke(p, (8.0 * view.zoom).max(5.0), Stroke::new(1.0, c));
                    }
                    let sign = if output { -1.0 } else { 1.0 };
                    painter.text(
                        p + Vec2::new(sign * 14.0 * view.zoom, 0.0),
                        if output {
                            Align2::RIGHT_CENTER
                        } else {
                            Align2::LEFT_CENTER
                        },
                        label(name),
                        FontId::proportional(12.0 * view.zoom),
                        if compatible {
                            text
                        } else {
                            theme::TEXT_DIM.gamma_multiply(0.4)
                        },
                    );
                    if response.secondary_clicked() && !output {
                        g.links.retain(|l| l.to != s.end());
                    }
                    let release = response.hovered()
                        && ui.input(|i| i.pointer.button_released(PointerButton::Primary));
                    if (response.clicked() || release) && !landed {
                        if let Some(fixed) = editor.wire {
                            if fixed != s && fixed.output != s.output {
                                match workflow::connect(g,fixed,s) { Ok(()) => { editor.wire = None; editor.message = None; }, Err(_) => editor.message = Some(label("Prises incompatibles ou boucle : choisissez une prise mise en évidence.")) }
                                landed = true;
                            } else if response.clicked() {
                                editor.wire = Some(s);
                            }
                        } else if response.clicked() {
                            editor.wire = Some(s);
                            landed = true;
                        }
                    }
                    if response.clicked() || response.drag_started() {
                        editor.library_add = true;
                    }
                    if response.drag_started() {
                        editor.wire = Some(s);
                    }
                    response.on_hover_text(format!(
                        "{} · {}\n{}",
                        label(name),
                        label(type_name(ty)),
                        label(if output {
                            "Cliquez ou glissez vers une entrée. Échap annule."
                        } else {
                            "Cliquez pour choisir une source. Clic droit pour débrancher."
                        })
                    ));
                    #[cfg(test)]
                    ui.data_mut(|d| {
                        d.insert_temp(
                            egui::Id::new(("creation-socket-test", n.id, i, output)),
                            hit,
                        )
                    });
                }
            }
        }
        let status = if !n.enabled {
            "Désactivé"
        } else if bodies.contains(&n.id) {
            "Étape de boucle"
        } else if active.contains(&n.id) {
            "Dans la composition"
        } else {
            "Non relié à la Scène"
        };
        painter.text(
            card.left_bottom() + Vec2::new(14.0, -13.0) * view.zoom,
            Align2::LEFT_CENTER,
            label(status),
            FontId::proportional(10.0 * view.zoom),
            if active.contains(&n.id) && n.enabled {
                accent.gamma_multiply(0.7)
            } else {
                theme::TEXT_DIM.gamma_multiply(0.6)
            },
        );
        #[cfg(test)]
        ui.data_mut(|d| d.insert_temp(egui::Id::new(("creation-node", n.id)), card));
    }
    if background.clicked() && !pan {
        state.form.selected.clear();
    }
    background.context_menu(|ui| {
        if ui.button(label("Ajouter un nœud")).clicked() {
            editor.library_hidden = false;
            editor.library_add = true;
            ui.ctx()
                .data_mut(|d| d.insert_temp(egui::Id::new("creation-search-focus"), true));
            ui.close();
        }
        if ui.button(label("Tout cadrer")).clicked() {
            fit(state, g, rect.size());
            ui.close();
        }
        if ui.button(label("Organiser")).clicked() {
            layout(g);
            fit(state, g, rect.size());
            ui.close();
        }
    });
    if ui.input(|i| i.key_pressed(Key::Escape)) {
        editor.wire = None;
        editor.message = None;
    }
    if !ui.ctx().egui_wants_keyboard_input() {
        if ui.input(|i| i.key_pressed(Key::Delete)) {
            if ui.input(|i| i.modifiers.shift) {
                workflow::dissolve(g, &state.form.selected);
                state.form.selected.clear();
            } else {
                remove(g, &mut state.form.selected);
            }
        }
        if ui.input(|i| i.modifiers.command && i.key_pressed(Key::A)) {
            state.form.selected = g.nodes.iter().map(|n| n.id).collect();
        }
        if ui.input(|i| i.modifiers.command && i.key_pressed(Key::D)) {
            duplicate(g, &mut state.form.selected);
        }
        if ui.input(|i| i.key_pressed(Key::F)) {
            if ui.input(|i| i.modifiers.shift) {
                fit_selection(state, g, rect.size());
            } else {
                fit(state, g, rect.size());
            }
        }
        if ui.input(|i| i.modifiers.shift && !i.modifiers.command && i.key_pressed(Key::A)) {
            editor.library_hidden = false;
            editor.library_add = true;
            ui.ctx()
                .data_mut(|d| d.insert_temp(egui::Id::new("creation-search-focus"), true));
        }
    }
    // A compact, readable legend stays in screen space at every zoom level.
    let help = Rect::from_min_size(
        rect.left_bottom() + Vec2::new(12.0, -36.0),
        Vec2::new((rect.width() - 24.0).max(0.0), 26.0),
    );
    painter.rect_filled(help, 5.0, theme::BG.gamma_multiply(0.92));
    painter.text(
        help.left_center() + Vec2::new(8.0, 0.0),
        Align2::LEFT_CENTER,
        if rect.width() < 600.0 {
            format!("{:.0}% · F : {}", view.zoom * 100.0, label("Tout cadrer"))
        } else {
            label("Molette : zoom · Espace + glisser : déplacer · F : tout cadrer")
        },
        FontId::proportional(11.0),
        theme::TEXT_DIM,
    );
}

fn fit_selection(state: &mut AppState, g: &CreationGraph, size: Vec2) {
    let mut selected = g.clone();
    selected
        .nodes
        .retain(|n| state.form.selected.contains(&n.id));
    fit(state, &selected, size);
}

fn fit_neighborhood(state: &mut AppState, g: &CreationGraph, size: Vec2) {
    let mut nearby: BTreeSet<_> = state.form.selected.iter().copied().collect();
    for l in &g.links {
        if state.form.selected.contains(&l.to.0) {
            nearby.insert(l.from.0);
        }
        if state.form.selected.contains(&l.from.0) {
            nearby.insert(l.to.0);
        }
    }
    let mut local = g.clone();
    local.nodes.retain(|n| nearby.contains(&n.id));
    fit(state, &local, size);
    if state.form.view.zoom < 0.45 && !state.form.selected.is_empty() {
        fit_selection(state, g, size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Modifiers, RawInput};

    struct Harness {
        ctx: egui::Context,
        state: AppState,
        form: FormPanel,
        time: f64,
        source: u32,
        target: u32,
    }
    impl Harness {
        fn new() -> Self {
            let ctx = egui::Context::default();
            theme::apply(&ctx);
            let mut state = AppState::new(Scene::example_2d(), None);
            let mut g = graph(&state);
            // Put the target earlier in drawing order than its source.
            let target = g.add(
                CreationKind::Field(FieldOp::Power { exponent: 2.0 }),
                [0.0, 440.0],
            );
            let source = g.add(
                CreationKind::Field(FieldOp::Scalar { value: 0.5 }),
                [0.0, 260.0],
            );
            let vector = g.add(
                CreationKind::Field(FieldOp::Vector {
                    value: [1.0; 3].into(),
                }),
                [0.0, 600.0],
            );
            assert!(g.node(vector).is_some());
            commit(&mut state, g).unwrap();
            let mut h = Self {
                ctx,
                state,
                form: FormPanel::new(),
                time: 0.0,
                source,
                target,
            };
            h.frame(vec![]);
            h.frame(vec![]);
            h
        }
        fn frame(&mut self, events: Vec<Event>) {
            self.time += 1.0 / 60.0;
            let mut output = self.ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 1000.0))),
                    time: Some(self.time),
                    events,
                    ..Default::default()
                },
                |ui| self.form.show(ui, &mut self.state),
            );
            output.textures_delta.clear();
        }
        fn socket(&self, id: u32, output: bool) -> Pos2 {
            self.ctx
                .data(|d| {
                    d.get_temp::<Rect>(egui::Id::new(("creation-socket-test", id, 0usize, output)))
                })
                .unwrap()
                .center()
        }
        fn pointer(&mut self, p: Pos2, pressed: bool) {
            self.frame(vec![
                Event::PointerMoved(p),
                Event::PointerButton {
                    pos: p,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ]);
        }
        fn click(&mut self, p: Pos2) {
            self.pointer(p, true);
            self.pointer(p, false);
        }
    }

    #[test]
    fn click_to_connect_works_in_both_directions_and_escape_cancels() {
        for reverse in [false, true] {
            let mut h = Harness::new();
            let (a, b) = if reverse {
                (h.socket(h.target, false), h.socket(h.source, true))
            } else {
                (h.socket(h.source, true), h.socket(h.target, false))
            };
            h.click(a);
            assert!(h.form.creation.wire.is_some());
            h.frame(vec![]);
            h.click(b);
            assert_eq!(
                graph(&h.state).input(h.target, 0),
                Some((h.source, 0)),
                "reverse={reverse}"
            );
            assert!(h.form.creation.wire.is_none());
            h.click(a);
            h.frame(vec![Event::Key {
                key: Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }]);
            assert!(h.form.creation.wire.is_none());
        }
    }

    #[test]
    fn a_drag_can_land_on_a_target_drawn_before_the_source() {
        let mut h = Harness::new();
        let a = h.socket(h.source, true);
        let b = h.socket(h.target, false);
        h.pointer(a, true);
        h.frame(vec![Event::PointerMoved(a + Vec2::new(16.0, 0.0))]);
        assert!(h.form.creation.wire.is_some());
        h.frame(vec![Event::PointerMoved(b)]);
        h.pointer(b, false);
        assert_eq!(graph(&h.state).input(h.target, 0), Some((h.source, 0)));
        assert!(h.form.creation.wire.is_none());
    }

    #[test]
    fn a_refused_wire_preserves_the_graph_and_can_be_retargeted() {
        let mut h = Harness::new();
        let vector = graph(&h.state)
            .nodes
            .iter()
            .find(|n| matches!(n.kind, CreationKind::Field(FieldOp::Vector { .. })))
            .unwrap()
            .id;
        let before = h.state.scene.clone();
        h.click(h.socket(vector, true));
        h.click(h.socket(h.target, false));
        assert_eq!(h.state.scene, before);
        assert!(h.form.creation.wire.is_some());
        assert!(h.form.creation.message.is_some());
        h.click(h.socket(h.source, true));
        h.click(h.socket(h.target, false));
        assert_eq!(graph(&h.state).input(h.target, 0), Some((h.source, 0)));
        assert!(h.form.creation.message.is_none());
    }

    #[test]
    fn compact_framing_uses_the_actual_canvas_size() {
        let mut h = Harness::new();
        let g = graph(&h.state);
        fit(&mut h.state, &g, Vec2::new(310.0, 470.0));
        for n in &g.nodes {
            let card = Rect::from_min_size(
                h.state.form.view.to_screen(Pos2::ZERO, Pos2::from(n.pos)),
                Vec2::new(WIDTH, height(n)) * h.state.form.view.zoom,
            );
            assert!(Rect::from_min_size(Pos2::ZERO, Vec2::new(310.0, 470.0)).contains_rect(card));
        }
    }

    #[test]
    fn inspecting_a_colored_recipe_preserves_exact_source_values_and_history() {
        let mut h = Harness::new();
        let mut g = graph(&h.state);
        let added = workflow::compose(&mut g, 0).unwrap();
        commit(&mut h.state, g).unwrap();
        h.state.form.selected = vec![*added.last().unwrap()];
        let before = h.state.scene.clone();
        h.state.take_preview_dirty();
        for _ in 0..4 {
            h.frame(vec![]);
        }
        assert_eq!(h.state.scene, before);
        assert!(!h.state.take_preview_dirty());
        h.state.undo();
        assert_ne!(h.state.scene, before, "idle frames must not add undo steps");
    }

    #[test]
    fn frame_all_covers_a_long_graph_at_the_node_limit() {
        let mut h = Harness::new();
        let mut g = CreationGraph::default();
        for i in 0..syfrack_core::scene::creation::MAX_CREATION_NODES {
            g.add(
                CreationKind::Field(FieldOp::Scalar { value: 0.5 }),
                [i as f32 * (WIDTH + 88.0), 0.0],
            );
        }
        let size = Vec2::new(310.0, 470.0);
        fit(&mut h.state, &g, size);
        let canvas = Rect::from_min_size(Pos2::ZERO, size);
        for n in &g.nodes {
            let card = Rect::from_min_size(
                h.state.form.view.to_screen(Pos2::ZERO, Pos2::from(n.pos)),
                Vec2::new(WIDTH, height(n)) * h.state.form.view.zoom,
            );
            assert!(canvas.contains_rect(card));
        }
    }
}
