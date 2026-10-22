//! The « Forme » tab: the node canvas between the library and the inspector.
//! Geometry and hit testing live in `canvas.rs`, the editing operations in
//! `graph_edit.rs`; this module draws and turns events into selection, view
//! and scene changes. Every graph change goes through `AppState::edit_graph`
//! (validate or refuse), `edit` (card moves) or `set_param` (sliders).

use std::collections::HashMap;

use egui::epaint::CubicBezierShape;
use egui::{
    Align2, Color32, FontId, Id, Key, Modifiers, PointerButton, Pos2, Rect, RichText, Sense,
    Stroke, StrokeKind, Ui,
};
use syfrack_core::{Bounds, Fractal, Graph, Measure, Node, NodeId, NodeKind, Params, PortType};

use crate::canvas::{self, CanvasView, Hit};
use crate::graph_edit::{self, ALL_KINDS, CATEGORIES};
use crate::state::AppState;
use crate::ui::{panels, theme, viewport, widgets};

mod construction;
mod draw;
mod edits;
mod inline;
mod inspector;
mod menus;
mod radius;
mod shelf;
mod wires;

pub(crate) use edits::leave_frame;
use edits::*;
use inline::*;
pub(crate) use inline::{field_label, is_integer};
pub use inline::{inline_values, row_rail, InlineRow};
pub use menus::matching_kinds;
pub use shelf::{Library, LibraryDrag};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FormMode {
    #[default]
    Construction,
    Expert,
}
use wires::*;

/// Width of the library column.
const LIBRARY_W: f32 = 38.0 * theme::GRID;
/// Width of the inspector column.
const INSPECTOR_W: f32 = 42.0 * theme::GRID;
/// Width of the add and card menus.
const MENU_W: f32 = 28.0 * theme::GRID;
/// Height of the add menu's list before it scrolls.
const MENU_LIST_H: f32 = 40.0 * theme::GRID;
/// Zoom factor of one wheel notch.
const ZOOM_PER_NOTCH: f32 = 1.1;
/// Below this zoom the 8 px grid is too dense to draw.
const GRID_MIN_ZOOM: f32 = 0.75;
/// Every this many grid cells, a stronger line.
const GRID_MAJOR: i64 = 8;
/// Corner radius of cards and frames, in world units.
const CORNER: f32 = 4.0;
/// Frame fill: `PANEL` at 60 %.
const FRAME_FILL: Color32 = Color32::from_rgba_unmultiplied_const(0x19, 0x22, 0x29, 153);
/// Pointer travel, in screen points, before a press on a card becomes a move.
const DRAG_THRESHOLD: f32 = 3.0;
/// Life of the red flash of a refused wire, in seconds.
const FLASH_S: f64 = 0.3;
/// Offset of a duplicate, in world units.
const DUPLICATE_OFFSET: [f32; 2] = [4.0 * theme::GRID, 4.0 * theme::GRID];
/// Gap between a frame and a card taken out of it, in world units.
const LEAVE_GAP: f32 = 3.0 * theme::GRID;
/// The estimated enclosing radius is recomputed at most this often.
const RADIUS_PERIOD_S: f64 = 1.0;
/// Margin left above and left of a loaded form, in world units.
const LOAD_MARGIN: f32 = 5.0 * theme::GRID;

/// The preview as the app hands it to the tab: drawn under the inspector.
#[derive(Clone, Debug, Default)]
pub struct FormView {
    pub can_specialize: bool,
    pub specializing: bool,
    pub specialized: bool,
    pub exporting: bool,
    /// None while the preview waits for a complete form.
    pub texture: Option<egui::TextureId>,
    /// Size in pixels of the preview texture.
    pub size: (u32, u32),
    /// Stage and samples, « compilation… » while a pipeline compiles.
    pub caption: String,
}

/// The view that shows a freshly loaded graph from its top left card.
pub fn view_for(g: &Graph) -> CanvasView {
    let min = g
        .nodes
        .iter()
        .map(|n| canvas::card_rect(n).min)
        .reduce(|a, b| a.min(b))
        .unwrap_or(Pos2::ZERO);
    CanvasView {
        offset: egui::vec2(LOAD_MARGIN - min.x, LOAD_MARGIN - min.y),
        zoom: 1.0,
    }
}

/// A wire being drawn: one end fixed on a port, the other on the pointer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wire {
    /// The fixed port: an output when `from_output`, else an input.
    pub end: (NodeId, u32),
    pub ty: PortType,
    pub from_output: bool,
    /// The input port this wire was lifted from: its link goes when the
    /// wire lands elsewhere.
    pub lifted: Option<(NodeId, u32)>,
}

enum Drag {
    /// Moving cards: where the press landed and each moved card's position
    /// then. `moving` turns on once the pointer has left the press point.
    Cards {
        start_world: Pos2,
        start_screen: Pos2,
        origins: Vec<(NodeId, [f32; 2])>,
        moving: bool,
    },
    Wire(Wire),
}

/// What a canvas menu offers.
#[derive(Clone, Debug, PartialEq)]
pub enum MenuKind {
    /// Add a node, linked to `wire` when the menu opened on its release.
    Add { query: String, wire: Option<Wire> },
    /// Act on a card (and the rest of the selection).
    Card(NodeId),
}

pub struct Menu {
    /// Where it opened, on screen and in the world.
    screen: Pos2,
    world: Pos2,
    pub kind: MenuKind,
    /// First frame: the search field takes the keyboard.
    fresh: bool,
}

/// What a menu click asks for, applied once the menu is drawn.
enum MenuAction {
    Add(NodeKind),
    Duplicate,
    SetEnabled(bool),
    Rename(NodeId),
    Delete,
    LeaveFrame(NodeId),
    /// Closes the menu; a lifted wire dropped on the void is removed.
    Close,
    /// Closes the menu and puts a lifted wire back.
    Cancel,
}

pub struct FormPanel {
    pub specialization_requested: bool,
    pub control_requested: bool,
    pub mode: FormMode,
    query: String,
    drag: Option<Drag>,
    /// Lasso corner where the press landed, in world units.
    lasso_from: Option<Pos2>,
    /// What lies under the pointer this frame.
    pub hover: Hit,
    /// Screen rectangle of the canvas at the last frame.
    pub rect: Rect,
    /// The open canvas menu.
    pub menu: Option<Menu>,
    /// Screen rectangle of the menu at the last frame.
    menu_rect: Rect,
    /// A refused wire: where (world) and when.
    pub flash: Option<(Pos2, f64)>,
    /// The inspector's name field: node and text being typed.
    name_buf: Option<(NodeId, String)>,
    /// « Renommer » asked the name field for the keyboard.
    focus_name: bool,
    /// Estimated enclosing radius, computed by a worker.
    radius: radius::RadiusEstimate,
    /// Duration of the last shader compilation, in ms.
    pub compile_ms: Option<f32>,
    /// The preview shows the form in clay (else with the scene's look).
    pub clay: bool,
    /// The preview image, set by the app each frame.
    pub view: FormView,
    pub library: Library,
    pub(crate) creation: super::creation::Editor,
    flat: bool,
}

impl Default for FormPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl FormPanel {
    pub fn dialogs(&mut self, ui: &mut Ui, state: &mut AppState) {
        self.library.dialogs(ui, state);
    }

    pub fn new() -> Self {
        Self {
            specialization_requested: false,
            control_requested: false,
            mode: FormMode::default(),
            query: String::new(),
            drag: None,
            lasso_from: None,
            hover: Hit::Empty,
            rect: Rect::NOTHING,
            menu: None,
            menu_rect: Rect::NOTHING,
            flash: None,
            name_buf: None,
            focus_name: false,
            radius: radius::RadiusEstimate::default(),
            compile_ms: None,
            clay: false,
            view: FormView::default(),
            library: Library::default(),
            creation: super::creation::Editor::default(),
            flat: false,
        }
    }

    /// A panel reading its library from `dir` (tests).
    pub fn with_library(dir: std::path::PathBuf) -> Self {
        Self {
            library: Library::with_dir(dir),
            ..Self::new()
        }
    }

    /// The wire being drawn, if any.
    pub fn wire(&self) -> Option<Wire> {
        match &self.drag {
            Some(Drag::Wire(w)) => Some(*w),
            _ => None,
        }
    }

    pub fn show(&mut self, ui: &mut Ui, state: &mut AppState) {
        self.flat = state.scene.dimension == syfrack_core::Dimension::TwoD;
        let mut editor = std::mem::take(&mut self.creation);
        super::creation::show(ui, state, self, &mut editor);
        self.creation = editor;
    }
    pub(crate) fn creation_forms(&mut self) -> Vec<crate::library::LibraryEntry> {
        self.library.refresh();
        self.library.entries.clone()
    }

    /// Compatibility view for embedders of the geometry-only editor. Studio
    /// uses `show`, the unified canvas. Edits still adapt to the canonical DAG.
    pub fn show_geometry_compatibility(&mut self, ui: &mut Ui, state: &mut AppState) {
        egui::Panel::top("legacy-geometry-mode").show(ui, |ui| {
            ui.horizontal(|ui| {
                for (mode, label) in [
                    (FormMode::Construction, "Construction"),
                    (FormMode::Expert, syfrack_i18n::tr!("Graphe expert")),
                ] {
                    if widgets::choice_button(ui, self.mode == mode, label).clicked() {
                        self.mode = mode;
                    }
                }
            });
        });
        let mut g = state.scene.fractal.to_graph();
        let before = state.scene.fractal.clone();
        let undo = state.can_undo();
        self.library.compact = self.mode == FormMode::Construction;
        egui::Panel::left("legacy-geometry-library")
            .resizable(false)
            .exact_size(if self.mode == FormMode::Expert {
                LIBRARY_W
            } else {
                224.0
            })
            .show(ui, |ui| self.library.show(ui, state));
        let mut edited = state.scene.fractal != before || state.can_undo() != undo;
        egui::Panel::right("legacy-geometry-inspector")
            .resizable(false)
            .exact_size(INSPECTOR_W)
            .show(ui, |ui| {
                egui::Panel::bottom("legacy-geometry-preview").show(ui, |ui| self.preview(ui));
                edited |= self.inspector(ui, state, &g);
            });
        if edited {
            g = state.scene.fractal.to_graph();
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(theme::BG))
            .show(ui, |ui| match self.mode {
                FormMode::Construction => self.construction(ui, state, &g),
                FormMode::Expert => self.canvas(ui, state, &mut g),
            });
    }

    /// The preview under the inspector: the « argile / look réel » switch,
    /// the image letterboxed in a 16:9 box, its caption.
    pub(crate) fn preview(&mut self, ui: &mut Ui) {
        self.preview_controls(ui);
        self.preview_image(ui);
    }

    /// Shared with the workshop status bar: requesting native compilation is
    /// explicit in both views, and disabled while export owns the GPU.
    pub fn preview_controls(&mut self, ui: &mut Ui) {
        if self.view.can_specialize {
            ui.horizontal(|ui| {
                if self.view.specialized {
                    if ui.add_enabled(!self.view.exporting,egui::Button::new(syfrack_i18n::tr!("Revenir au contrôle GPU"))).clicked() {
                        self.control_requested=true;
                    }
                } else {
                    if ui.add_enabled(!self.view.exporting && !self.view.specializing,egui::Button::new(syfrack_i18n::tr!("Compiler l’aperçu spécialisé")))
                        .on_hover_text(syfrack_i18n::tr!("Affiner l’éclairage et les matières. Le contrôle géométrique reste disponible pendant la compilation.")).clicked() {
                        self.specialization_requested=true;
                    }
                    if self.view.specializing { ui.spinner(); }
                }
            });
        }
    }

    fn preview_image(&mut self, ui: &mut Ui) {
        let current = usize::from(!self.clay);
        // The fixed geometry view has one neutral look. Offer the look switch
        // after explicit specialization, when it actually changes the image.
        if !self.flat && (!self.view.can_specialize || self.view.specialized) {
            if let Some(i) = panels::choice(
                ui,
                syfrack_i18n::tr!("Aperçu"),
                &["argile", syfrack_i18n::tr!("look réel")],
                current,
            ) {
                self.clay = i == 0;
            }
        }
        ui.add_space(theme::GRID / 2.0);
        let w = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(egui::vec2(w, w * 9.0 / 16.0), Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, theme::BG);
        let (iw, ih) = (
            self.view.size.0.max(1) as f32,
            self.view.size.1.max(1) as f32,
        );
        match self.view.texture {
            Some(tex) => {
                let s = (rect.width() / iw).min(rect.height() / ih);
                let img = Rect::from_center_size(rect.center(), egui::vec2(iw * s, ih * s));
                let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                painter.image(tex, img, uv, Color32::WHITE);
            }
            None => {
                painter.text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    &self.view.caption,
                    FontId::proportional(13.0),
                    theme::TEXT_DIM,
                );
            }
        }
        ui.add_space(theme::GRID / 2.0);
        ui.label(
            RichText::new(&self.view.caption)
                .monospace()
                .color(theme::TEXT_DIM),
        );
    }

    /// Wheel zoom, middle-button pan, menus, selection, card moves, wires,
    /// the lasso and the canvas shortcuts. Returns true when the scene changed.
    fn input(
        &mut self,
        ui: &Ui,
        state: &mut AppState,
        g: &Graph,
        resp: &egui::Response,
        rect: Rect,
        view: &mut CanvasView,
    ) -> bool {
        let pointer = ui.input(|i| i.pointer.latest_pos());
        let world = pointer.map(|p| view.to_world(rect.min, p));
        let mut edited = false;
        // Hover follows the pointer over the whole canvas, inline rails
        // included (they only come alive on the hovered card).
        let over_menu = pointer.is_some_and(|p| self.menu_rect.contains(p));
        let in_canvas = pointer.is_some_and(|p| rect.contains(p)) && !over_menu;
        self.hover = match world {
            Some(w) if in_canvas || self.drag.is_some() => canvas::hit_test(g, w),
            _ => Hit::Empty,
        };

        if resp.hovered() {
            let notches = viewport::wheel_notches_of(ui);
            if let (true, Some(p)) = (notches != 0.0, pointer) {
                view.zoom_at(rect.min, p, ZOOM_PER_NOTCH.powf(notches));
            }
        }
        if resp.dragged_by(PointerButton::Middle) {
            view.offset += resp.drag_delta() / view.zoom;
        }

        let (pressed, down, released, shift, now) = ui.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.primary_released(),
                i.modifiers.shift,
                i.time,
            )
        });

        // A press outside an open menu closes it and does nothing else.
        let any_press = ui.input(|i| i.pointer.any_pressed());
        if self.menu.is_some() && any_press && !over_menu {
            edited |= self.close_menu(state);
        } else if pressed && resp.hovered() {
            if let (Some(p), Some(w)) = (pointer, world) {
                self.press(state, g, w, p, shift);
            }
        }
        if resp.secondary_clicked() {
            if let (Some(p), Some(w)) = (pointer, world) {
                self.open_menu_at(state, g, p, w);
            }
        }

        // The release frame applies its own last delta too: `coalesce_gestures`
        // keeps it in the gesture, so a whole move is one undo step.
        if let (
            Some(Drag::Cards {
                start_world,
                start_screen,
                origins,
                moving,
            }),
            true,
            Some(p),
        ) = (&mut self.drag, down || released, pointer)
        {
            *moving |= (p - *start_screen).length() > DRAG_THRESHOLD;
            if *moving {
                // Snapping the delta keeps off-grid cards off-grid but moves
                // them by whole grid steps.
                let d = view.to_world(rect.min, p) - *start_world;
                let (dx, dy) = (snap(d.x), snap(d.y));
                let moved: Vec<(NodeId, [f32; 2])> = origins
                    .iter()
                    .map(|(id, o)| (*id, [o[0] + dx, o[1] + dy]))
                    .collect();
                let changed = moved
                    .iter()
                    .any(|(id, p)| g.node(*id).is_some_and(|n| n.pos != *p));
                if changed {
                    state.edit(|s| {
                        let g = s.fractal.graph_mut();
                        for (id, p) in &moved {
                            if let Some(n) = g.node_mut(*id) {
                                n.pos = *p;
                            }
                        }
                    });
                    edited = true;
                }
            }
        }
        if released || (!down && (self.drag.is_some() || self.lasso_from.is_some())) {
            if let (Some(from), Some(w)) = (self.lasso_from, world) {
                let r = Rect::from_two_pos(from, w);
                let picked = canvas::lasso_select(g, r);
                let sel = &mut state.form.selected;
                if !shift {
                    sel.clear();
                }
                for id in picked {
                    if !sel.contains(&id) {
                        sel.push(id);
                    }
                }
            }
            match self.drag.take() {
                Some(Drag::Cards {
                    origins,
                    moving: true,
                    ..
                }) => edited |= drop_in_frame(state, &origins),
                Some(Drag::Wire(w)) => {
                    if let (Some(p), Some(wp)) = (pointer, world) {
                        edited |= self.finish_wire(state, w, p, wp, now);
                    }
                }
                _ => {}
            }
            self.lasso_from = None;
        }

        edited |= self.shortcuts(ui, state, pointer.is_some_and(|p| rect.contains(p)));
        edited
    }

    /// Delete, Ctrl-D, Ctrl-A and Escape while the pointer is over the canvas
    /// and no text field has the keyboard.
    fn shortcuts(&mut self, ui: &Ui, state: &mut AppState, hovered: bool) -> bool {
        // Escape always drops a wire being drawn (a lifted one goes back).
        if matches!(self.drag, Some(Drag::Wire(_)))
            && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.drag = None;
            return false;
        }
        if !hovered || ui.ctx().egui_wants_keyboard_input() {
            return false;
        }
        let (delete, duplicate, all, escape) = ui.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::Delete),
                i.consume_key(Modifiers::COMMAND, Key::D),
                i.consume_key(Modifiers::COMMAND, Key::A),
                i.consume_key(Modifiers::NONE, Key::Escape),
            )
        });
        let mut edited = false;
        if delete {
            edited |= delete_selection(state);
        }
        if duplicate {
            edited |= duplicate_selection(state);
        }
        if all {
            state.form.selected = state
                .scene
                .fractal
                .to_graph()
                .nodes
                .iter()
                .map(|n| n.id)
                .collect();
        }
        if escape {
            if matches!(self.drag, Some(Drag::Wire(_))) {
                self.drag = None;
            } else if self.menu.is_some() {
                self.menu = None;
            } else {
                state.form.selected.clear();
            }
        }
        edited
    }

    /// A primary press on the canvas: select and start moving a card or a
    /// frame, start a wire on a port, or a lasso on the empty canvas.
    fn press(&mut self, state: &mut AppState, g: &Graph, world: Pos2, screen: Pos2, shift: bool) {
        match canvas::hit_test(g, world) {
            Hit::Card(id) | Hit::Frame(id) => {
                let sel = &mut state.form.selected;
                if shift {
                    if !sel.contains(&id) {
                        sel.push(id);
                    }
                } else if !sel.contains(&id) {
                    *sel = vec![id];
                }
                // Only a graph has positions to write.
                if !matches!(state.scene.fractal, Fractal::Graph(_)) {
                    return;
                }
                // A moved Répéter carries its body along.
                let ids = with_bodies(g, sel);
                let origins = ids
                    .iter()
                    .filter_map(|id| g.node(*id).map(|n| (*id, n.pos)))
                    .collect();
                self.drag = Some(Drag::Cards {
                    start_world: world,
                    start_screen: screen,
                    origins,
                    moving: false,
                });
            }
            Hit::Empty | Hit::Wire(_) => {
                if !shift {
                    state.form.selected.clear();
                }
                self.lasso_from = Some(world);
            }
            Hit::OutPort(id, i) => {
                if let Some(ty) = g.node(id).and_then(|n| n.kind.outputs().get(i).copied()) {
                    self.drag = Some(Drag::Wire(Wire {
                        end: (id, i as u32),
                        ty,
                        from_output: true,
                        lifted: None,
                    }));
                }
            }
            Hit::InPort(id, i) => {
                let Some(ty) = g.node(id).and_then(|n| n.kind.inputs().get(i).copied()) else {
                    return;
                };
                // A press on a linked input lifts its wire off, still held
                // by its origin.
                self.drag = Some(Drag::Wire(match g.input_of(id, i as u32) {
                    Some(from) => Wire {
                        end: from,
                        ty,
                        from_output: true,
                        lifted: Some((id, i as u32)),
                    },
                    None => Wire {
                        end: (id, i as u32),
                        ty,
                        from_output: false,
                        lifted: None,
                    },
                }));
            }
        }
    }
}

/// `ids` and the bodies of the Répéter among them, recursively.
fn with_bodies(g: &Graph, ids: &[NodeId]) -> Vec<NodeId> {
    let mut out: Vec<NodeId> = Vec::new();
    let mut stack: Vec<NodeId> = ids.to_vec();
    while let Some(id) = stack.pop() {
        if out.contains(&id) {
            continue;
        }
        out.push(id);
        stack.extend_from_slice(g.body_ids_of(id));
    }
    out
}

/// Positions snap to the 8 px grid.
fn snap(v: f32) -> f32 {
    (v / theme::GRID).round() * theme::GRID
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Vec2;

    #[test]
    fn the_add_menu_filters_by_name_and_by_the_wire_it_carries() {
        let all = matching_kinds("", None);
        assert_eq!(all.len(), ALL_KINDS.len());
        let mirror = matching_kinds("MIR", None);
        assert_eq!(mirror.len(), 1);
        assert_eq!(mirror[0].label(), "Mirror");
        let dist_out = Wire {
            end: (7, 0),
            ty: PortType::Distance,
            from_output: true,
            lifted: None,
        };
        let takes_distance = matching_kinds("", Some(&dist_out));
        assert!(takes_distance
            .iter()
            .all(|k| k.inputs().contains(&PortType::Distance)));
        let point_in = Wire {
            end: (1, 0),
            ty: PortType::Point,
            from_output: false,
            lifted: None,
        };
        let gives_point = matching_kinds("", Some(&point_in));
        assert!(gives_point.iter().all(|k| k.is_point_to_point()));
    }

    #[test]
    fn inline_rows_keep_three_fields_and_group_vec3_components() {
        let mut g = Graph::new_minimal();
        graph_edit::add_node(
            &mut g,
            NodeKind::Mirror {
                normal: glam::Vec3::X,
                offset: 0.5,
            },
            [0.0; 2],
        );
        let rows = inline_values(&mut g);
        let m = &rows[&3];
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].field, "normal");
        assert_eq!(m[0].values, vec![1.0, 0.0, 0.0]);
        assert_eq!(m[1].values, vec![0.5]);
        let rails: Vec<Rect> = (0..3)
            .map(|c| {
                row_rail(
                    Rect::from_min_size(Pos2::ZERO, Vec2::new(192.0, 80.0)),
                    0,
                    c,
                    3,
                    1.0,
                )
            })
            .collect();
        assert!(rails[0].max.x < rails[1].min.x && rails[1].max.x < rails[2].min.x);
        assert!(rails[2].max.x <= 192.0);
    }

    #[test]
    fn unwired_active_inputs_are_red_and_named_in_the_inspector() {
        let mut g = Graph::new_minimal();
        g.links.retain(|l| l.to.0 != 2); // Sortie unwired
        let u = graph_edit::add_node(&mut g, NodeKind::Union, [0.0; 2]); // dangling
        assert_eq!(inspector::port_notes(&g, 2), vec!["Unconnected port"]);
        assert!(
            inspector::port_notes(&g, u).is_empty(),
            "dangling: no fault"
        );
        graph_edit::connect(&mut g, (u, 0), (2, 0)).unwrap();
        graph_edit::connect(&mut g, (1, 0), (u, 1)).unwrap();
        assert!(
            inspector::port_notes(&g, 2).is_empty(),
            "rewired: mark gone"
        );
        assert_eq!(
            inspector::port_notes(&g, u),
            vec!["Port 1 is not connected"]
        );
        let (fill, ring) = draw::port_style(PortType::Distance, false, true);
        assert_eq!(fill, theme::ERROR);
        assert!(ring.is_some_and(|s| s.color == theme::ERROR));
        assert_eq!(
            draw::port_style(PortType::Distance, false, false).0,
            canvas::WIRE_DIST
        );
    }

    #[test]
    fn a_loaded_form_is_framed_from_its_top_left_card() {
        let mut g = Graph::new_minimal();
        for n in &mut g.nodes {
            n.pos[0] -= 200.0;
            n.pos[1] += 64.0;
        }
        let view = view_for(&g);
        let top_left = g
            .nodes
            .iter()
            .map(|n| canvas::card_rect(n).min)
            .reduce(|a, b| a.min(b))
            .unwrap();
        let screen = view.to_screen(Pos2::ZERO, top_left);
        assert_eq!(screen, Pos2::new(LOAD_MARGIN, LOAD_MARGIN));
        assert_eq!(view.zoom, 1.0);
    }

    #[test]
    fn the_topology_mark_is_in_the_font() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        ctx.run_ui(Default::default(), |_| {})
            .textures_delta
            .clear();
        let mark = widgets::TOPOLOGY_MARK.chars().next().unwrap();
        assert!(ctx.fonts_mut(|f| f.has_glyph(&FontId::proportional(11.0), mark)));
    }
}
