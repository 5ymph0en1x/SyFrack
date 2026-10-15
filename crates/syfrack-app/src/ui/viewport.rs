//! Mouse and keyboard binding of the central viewport. All camera maths lives
//! in `crate::input`; this file only translates egui events into scene edits.

use egui::{Key, PointerButton, Sense};
use glam::Vec3;
use syfrack_core::{Camera, CameraBasis, CameraMode, Light, Scene, MAX_LIGHTS};

use crate::gizmos;
use crate::input;
use crate::preview::Pick;
use crate::state::AppState;
use crate::ui::panels::new_area;
use crate::ui::{theme, Viewport};

/// Roll speed of the arrow keys, degrees per second.
const ROLL_DEG_PER_S: f32 = 45.0;
/// Points egui reports per wheel line on touchpads and pixel-delta devices.
const POINTS_PER_NOTCH: f32 = 50.0;
/// Lines per page for page-unit wheel events.
const LINES_PER_PAGE: f32 = 10.0;
/// Wheel or key navigation separated by less than this is one undo step.
const NAV_GESTURE_GAP_S: f64 = 0.3;
/// Padding and corner radius of the backdrop behind text over the image.
const OVERLAY_PAD: f32 = 4.0;
/// The studio background at 70 % opacity.
const OVERLAY_BACKDROP: egui::Color32 = egui::Color32::from_rgba_premultiplied(
    (0x1a * 179 / 255) as u8,
    (0x18 * 179 / 255) as u8,
    (0x16 * 179 / 255) as u8,
    179,
);
/// Minimum width of the pick menu, so each action fits on one line.
const MENU_W: f32 = 28.0 * theme::GRID;
/// How far in front of the picked surface « Placer une surfacique ici » puts the light.
const AREA_STANDOFF: f32 = 0.5;
/// Side of the square hit area of a point handle, in points (radius 11).
const HANDLE_HIT: f32 = 22.0;
/// Height of the hit strip of the fog line, in points.
const FOG_HIT: f32 = 12.0;
/// Drawn size of the sun handle (radius) and of the area centre (radius) and corners (side).
const SUN_HANDLE_R: f32 = 7.0;
const AREA_CENTRE_R: f32 = 6.0;
const AREA_CORNER: f32 = 8.0;
/// Sun angular size change per wheel notch over its handle.
const SUN_SIZE_PER_NOTCH: f32 = 1.1;
/// Bounds of the sun angular size in degrees (the scene's parameter bounds).
const SUN_SIZE_RANGE: (f32, f32) = (0.01, 90.0);

/// Zoom notches of one wheel event: a physical wheel click is one line and
/// therefore exactly one `ZOOM_PER_NOTCH` step.
pub fn wheel_notches(unit: egui::MouseWheelUnit, dy: f32) -> f32 {
    match unit {
        egui::MouseWheelUnit::Line => dy,
        egui::MouseWheelUnit::Point => dy / POINTS_PER_NOTCH,
        egui::MouseWheelUnit::Page => dy * LINES_PER_PAGE,
    }
}

/// The French readout of a pick, shown above the caption.
pub fn pick_label(point: Vec3, pick: &Pick) -> String {
    syfrack_i18n::tr_format!(
        "profondeur {:.3}  pas {}  ({:.2}, {:.2}, {:.2})",
        pick.depth,
        pick.steps as u32,
        point.x,
        point.y,
        point.z
    )
}

/// The frame an `output`-sized render covers over `view`. The camera field
/// of view is horizontal, so the output always spans the full width of the
/// preview and only its height follows the output aspect: a wider output
/// shows less above and below (bands), a taller one shows more (the frame
/// runs past the top and bottom, `taller` is then true).
pub struct GuideFrame {
    pub rect: egui::Rect,
    pub taller: bool,
}

pub fn frame_guide_rect(view: egui::Rect, output: (u32, u32)) -> GuideFrame {
    if output.0 == 0 || output.1 == 0 || view.width() <= 0.0 || view.height() <= 0.0 {
        return GuideFrame {
            rect: view,
            taller: false,
        };
    }
    let h = view.width() * output.1 as f32 / output.0 as f32;
    GuideFrame {
        rect: egui::Rect::from_center_size(view.center(), egui::vec2(view.width(), h)),
        taller: h > view.height() + 0.5,
    }
}

/// Dims what a render of `output` size leaves out and rules its frame; when
/// the frame is taller than the preview, marks that it runs past the edges.
fn draw_frame_guide(ui: &egui::Ui, rect: egui::Rect, output: (u32, u32)) {
    let guide = frame_guide_rect(rect, output);
    let frame = guide.rect;
    let painter = ui.painter_at(rect);
    let line = egui::Stroke::new(1.0, theme::TEXT_DIM);
    if guide.taller {
        // L-shaped marks in the four corners: the frame goes on beyond them.
        let tick = 2.0 * theme::GRID;
        for y in [rect.top(), rect.bottom()] {
            let out = if y == rect.top() { -1.0 } else { 1.0 };
            for x in [rect.left(), rect.right()] {
                let inward = if x == rect.left() { 1.0 } else { -1.0 };
                painter.line_segment([egui::pos2(x, y), egui::pos2(x + inward * tick, y)], line);
                painter.line_segment([egui::pos2(x, y), egui::pos2(x, y - out * tick)], line);
            }
        }
        overlay_text(
            ui,
            rect.left_bottom() + egui::vec2(theme::GRID, -4.0 * theme::GRID),
            syfrack_i18n::tr!("cadre plus haut que l'aperçu"),
        );
    } else if rect.top() < frame.top() - 0.5 {
        let dim = egui::Color32::from_black_alpha(110);
        painter.rect_filled(
            egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), frame.top())),
            0.0,
            dim,
        );
        painter.rect_filled(
            egui::Rect::from_min_max(egui::pos2(rect.left(), frame.bottom()), rect.max),
            0.0,
            dim,
        );
        painter.hline(rect.x_range(), frame.top(), line);
        painter.hline(rect.x_range(), frame.bottom(), line);
    }
}

/// What the viewport reports to the app after one frame.
#[derive(Default)]
pub struct ViewportOutput {
    /// Pick requested by a click, in [0, 1]^2 over the image.
    pub pick_request: Option<(f32, f32)>,
    /// Cursor position over the image, in [0, 1]^2, when it hovers it.
    pub hover_uv: Option<(f32, f32)>,
    /// The cursor has not moved this frame (safe moment for a depth readback).
    pub pointer_still: bool,
}

pub(crate) fn nav_id() -> egui::Id {
    egui::Id::new("viewport-nav")
}

/// True while a wheel or key navigation gesture is going on.
pub fn navigating(ui: &egui::Ui) -> bool {
    let now = ui.input(|i| i.time);
    ui.data(|d| d.get_temp::<f64>(nav_id()))
        .is_some_and(|t| now - t < NAV_GESTURE_GAP_S)
}

/// Applies a camera move: wheel and key moves open (or continue) a
/// navigation gesture so the whole gesture is one undo step.
fn move_camera(ui: &egui::Ui, state: &mut AppState, gesture: bool, f: impl FnOnce(&mut Camera)) {
    if gesture {
        let now = ui.input(|i| i.time);
        ui.data_mut(|d| d.insert_temp(nav_id(), now));
        state.begin_drag();
    }
    state.edit(|s| f(&mut s.camera));
}

/// The focus distance that puts `point` in focus, held inside the focus bounds
/// (a point behind the lens gets the nearest focus).
pub fn focus_here(scene: &Scene, point: Vec3) -> f32 {
    let d = input::focus_distance(&scene.camera.basis(), point);
    match crate::ui::panels::bounds_of(scene, "camera.focus_distance") {
        Some(b) => d.clamp(b.min, b.max),
        None => d,
    }
}

/// Text drawn over the image, bottom-left anchored, on a soft dark backdrop
/// so it reads on light images too.
pub fn overlay_text(ui: &egui::Ui, anchor: egui::Pos2, text: &str) {
    let painter = ui.painter();
    let galley = painter.layout_no_wrap(
        text.to_string(),
        egui::FontId::monospace(12.0),
        theme::TEXT_DIM,
    );
    let rect = egui::Align2::LEFT_BOTTOM.anchor_size(anchor, galley.size());
    painter.rect_filled(rect.expand(OVERLAY_PAD), OVERLAY_PAD, OVERLAY_BACKDROP);
    painter.galley(rect.min, galley, theme::TEXT_DIM);
}

/// The point the sun compass is centred on: the orbit point, or the centre of
/// the focus plane when flying.
fn sun_anchor(camera: &Camera, basis: &CameraBasis) -> Vec3 {
    match camera.mode {
        CameraMode::Orbit { point, .. } => point,
        CameraMode::Free { .. } => basis.origin + basis.forward * basis.focus_distance,
    }
}

/// Handle colour: quiet at rest, accent while hovered or dragged.
fn handle_color(r: &egui::Response) -> egui::Color32 {
    if r.hovered() || r.dragged() {
        theme::ACCENT
    } else {
        theme::TEXT_DIM
    }
}

fn point_handle(
    ui: &egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    at: egui::Pos2,
    side: f32,
) -> egui::Response {
    ui.interact(
        egui::Rect::from_center_size(at, egui::vec2(side, side)),
        ui.id().with(id),
        Sense::drag(),
    )
}

pub(crate) fn wheel_notches_of(ui: &egui::Ui) -> f32 {
    ui.input(|i| {
        i.events
            .iter()
            .map(|e| match e {
                egui::Event::MouseWheel { unit, delta, .. } => wheel_notches(*unit, delta.y),
                _ => 0.0,
            })
            .sum()
    })
}

/// Draws the sun compass, the area-light handles and the fog line over the
/// image and applies their drags. They only show while the pointer is over the
/// image or a handle is held. Returns true while a handle is being dragged
/// (the camera then ignores the drag).
fn show_gizmos(ui: &egui::Ui, state: &mut AppState, rect: egui::Rect, aspect: f32) -> bool {
    let basis = state.scene.camera.basis();
    let size = (rect.width(), rect.height());
    let to_screen = |(u, v): (f32, f32)| rect.min + egui::vec2(u * rect.width(), v * rect.height());
    let project = |p: Vec3| input::project(&basis, aspect, p).map(to_screen);
    let painter = ui.painter_at(rect);
    let line = egui::Stroke::new(1.0, theme::TEXT_DIM);
    let near = ui.rect_contains_pointer(rect);
    let mut busy = false;
    let mut label: Option<(egui::Pos2, String)> = None;

    // Fog first, so the point handles (created later) win where they overlap.
    let (fog_density, fog_height) = (
        state.scene.atmosphere.fog_density,
        state.scene.atmosphere.fog_height,
    );
    if fog_density > 0.0 {
        if let Some((a, b)) = gizmos::fog_horizon(&basis, aspect, fog_height) {
            let (a, b) = (to_screen(a), to_screen(b));
            if (b.x - a.x).abs() > 1e-3 {
                let y_at = |x: f32| a.y + (b.y - a.y) / (b.x - a.x) * (x - a.x);
                let mid = y_at(rect.center().x);
                if (rect.top()..=rect.bottom()).contains(&mid) {
                    let strip = egui::Rect::from_center_size(
                        egui::pos2(rect.center().x, mid),
                        egui::vec2(rect.width(), FOG_HIT),
                    );
                    let r = ui.interact(strip, ui.id().with("fog"), Sense::drag());
                    busy |= r.dragged();
                    if r.dragged() {
                        let h = gizmos::drag_fog_height(fog_height, &basis, r.drag_delta().y, size);
                        if h != fog_height {
                            state.set_param("atmosphere.fog_height", h);
                        }
                    }
                    if near || r.dragged() {
                        let ends = [
                            egui::pos2(rect.left(), y_at(rect.left())),
                            egui::pos2(rect.right(), y_at(rect.right())),
                        ];
                        let color = if r.hovered() || r.dragged() {
                            theme::ACCENT
                        } else {
                            theme::ACCENT.gamma_multiply(0.5)
                        };
                        painter.extend(egui::Shape::dashed_line(
                            &ends,
                            egui::Stroke::new(1.0, color),
                            6.0,
                            4.0,
                        ));
                    }
                    if r.hovered() || r.dragged() {
                        label = Some((
                            egui::pos2(rect.left() + 3.0 * theme::GRID, mid - 8.0),
                            syfrack_i18n::tr_format!(
                                "brouillard  y = {:.2}",
                                state.scene.atmosphere.fog_height
                            ),
                        ));
                    }
                }
            }
        }
    }

    let lights = state.scene.lights.clone();
    let anchor = sun_anchor(&state.scene.camera, &basis);
    for (i, light) in lights.iter().enumerate() {
        match light {
            Light::Sun {
                direction,
                angular_size,
                ..
            } => {
                let Some(g) = gizmos::sun_gizmo(&basis, aspect, anchor, *direction, size) else {
                    continue;
                };
                let (c, h) = (to_screen(g.centre), to_screen(g.handle));
                let marker = g.edge_marker.map(to_screen);
                let grab = if g.in_front { h } else { marker.unwrap_or(h) };
                let r = point_handle(ui, ("sun", i), grab, HANDLE_HIT);
                busy |= r.dragged();
                let d = r.drag_delta();
                if r.dragged() && d != egui::Vec2::ZERO {
                    let new_dir = gizmos::drag_sun(*direction, &basis, d.x, d.y, size);
                    state.edit(|s| {
                        if let Light::Sun { direction, .. } = &mut s.lights[i] {
                            *direction = new_dir;
                        }
                    });
                }
                let notches = if r.hovered() {
                    wheel_notches_of(ui)
                } else {
                    0.0
                };
                if notches != 0.0 {
                    let deg = (angular_size * SUN_SIZE_PER_NOTCH.powf(notches))
                        .clamp(SUN_SIZE_RANGE.0, SUN_SIZE_RANGE.1);
                    state.set_param(&format!("lights[{i}].sun.angular_size"), deg);
                }
                if near || r.dragged() {
                    painter.circle_stroke(
                        c,
                        g.radius_px,
                        egui::Stroke::new(1.0, theme::TEXT_DIM.gamma_multiply(0.6)),
                    );
                    painter.line_segment([c, h], line);
                    if g.in_front {
                        painter.circle_filled(h, SUN_HANDLE_R, handle_color(&r));
                    } else {
                        painter.circle_filled(h, SUN_HANDLE_R, theme::TEXT_DIM);
                        if let Some(m) = marker {
                            let dir = (m - c).normalized();
                            let side = egui::vec2(-dir.y, dir.x);
                            painter.add(egui::Shape::convex_polygon(
                                vec![
                                    m + dir * SUN_HANDLE_R,
                                    m - dir * SUN_HANDLE_R * 0.5 + side * SUN_HANDLE_R,
                                    m - dir * SUN_HANDLE_R * 0.5 - side * SUN_HANDLE_R,
                                ],
                                theme::ACCENT,
                                egui::Stroke::NONE,
                            ));
                        }
                    }
                }
                if r.hovered() || r.dragged() {
                    let elevation = direction.y.clamp(-1.0, 1.0).asin().to_degrees();
                    label = Some((
                        grab + egui::vec2(14.0, -8.0),
                        syfrack_i18n::tr_format!("soleil  élévation {elevation:.0}°  taille {angular_size:.2}° (molette)", angular_size = angular_size, elevation = elevation),
                    ));
                }
            }
            Light::Area {
                position,
                size: side,
                ..
            }
            | Light::Projector {
                position,
                size: side,
                ..
            } => {
                let Some(corners) = gizmos::area_corners(light) else {
                    continue;
                };
                let pts: Option<Vec<egui::Pos2>> = corners.iter().map(|c| project(*c)).collect();
                let (Some(pts), Some(centre)) = (pts, project(*position)) else {
                    continue;
                };
                // Corners first: the centre handle is created last and so wins
                // where a small rectangle makes the boxes overlap.
                let mut corner_responses = Vec::with_capacity(4);
                let mut active = false;
                for (k, p) in pts.iter().enumerate() {
                    let rk = point_handle(ui, ("area-corner", i, k), *p, HANDLE_HIT * 0.8);
                    busy |= rk.dragged();
                    let d = rk.drag_delta();
                    if rk.dragged() && d != egui::Vec2::ZERO {
                        if let Some(resized) = gizmos::resize_area(light, k, &basis, d.x, d.y, size)
                        {
                            state.edit(|s| s.lights[i] = resized);
                        }
                    }
                    active |= rk.hovered() || rk.dragged();
                    corner_responses.push(rk);
                }
                let rc = point_handle(ui, ("area", i), centre, HANDLE_HIT);
                busy |= rc.dragged();
                let d = rc.drag_delta();
                if rc.dragged() && d != egui::Vec2::ZERO {
                    if let Some(moved) =
                        gizmos::move_area_in_screen_plane(light, &basis, d.x, d.y, size)
                    {
                        state.edit(|s| s.lights[i] = moved);
                    }
                }
                active |= rc.hovered() || rc.dragged();
                if near || busy {
                    painter.add(egui::Shape::closed_line(pts.clone(), line));
                    painter.circle_filled(centre, AREA_CENTRE_R, handle_color(&rc));
                    for (p, rk) in pts.iter().zip(&corner_responses) {
                        let square =
                            egui::Rect::from_center_size(*p, egui::vec2(AREA_CORNER, AREA_CORNER));
                        painter.rect_stroke(
                            square,
                            0.0,
                            egui::Stroke::new(1.0, handle_color(rk)),
                            egui::StrokeKind::Middle,
                        );
                    }
                }
                if active {
                    label = Some((
                        centre + egui::vec2(14.0, -8.0),
                        syfrack_i18n::tr_format!("surfacique  {:.2} × {:.2}", side[0], side[1]),
                    ));
                }
            }
            Light::Tube {
                start, end, radius, ..
            } => {
                if let (Some(a), Some(b)) = (project(*start), project(*end)) {
                    if near || busy {
                        painter.line_segment([a, b], line);
                    }
                    for (endpoint, world, pixel) in [(0, *start, a), (1, *end, b)] {
                        let response =
                            point_handle(ui, ("tube-end", i, endpoint), pixel, HANDLE_HIT);
                        busy |= response.dragged();
                        let delta = response.drag_delta();
                        if response.dragged() && delta != egui::Vec2::ZERO {
                            let moved =
                                world + gizmos::screen_drag(&basis, world, delta.x, delta.y, size);
                            state.edit(|s| {
                                if let Light::Tube { start, end, .. } = &mut s.lights[i] {
                                    if endpoint == 0 {
                                        *start = moved;
                                    } else {
                                        *end = moved;
                                    }
                                }
                            });
                        }
                        if near || response.hovered() {
                            painter.circle_filled(pixel, AREA_CENTRE_R, handle_color(&response));
                        }
                        if response.hovered() {
                            label = Some((
                                pixel + egui::vec2(14.0, -8.0),
                                syfrack_i18n::tr_format!(
                                    "néon  rayon {radius:.3} m",
                                    radius = radius
                                ),
                            ));
                        }
                    }
                }
            }
            Light::Environment { .. } | Light::Hdri { .. } => {}
        }
    }
    if let Some((at, text)) = label {
        overlay_text(ui, at, &text);
    }
    busy
}

/// The menu of a right click on the image, acting on the last pick.
fn pick_menu(ui: &mut egui::Ui, state: &mut AppState, pick: Option<&(Vec3, Pick)>) {
    // One line per action.
    ui.set_min_width(MENU_W);
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
    let Some((point, pick)) = pick else {
        ui.label(
            egui::RichText::new(syfrack_i18n::tr!("aucune surface ici")).color(theme::TEXT_DIM),
        );
        return;
    };
    let basis = state.scene.camera.basis();
    if ui
        .button(syfrack_i18n::tr!("Mettre au point ici"))
        .clicked()
    {
        let focus = focus_here(&state.scene, *point);
        state.set_param("camera.focus_distance", focus);
        ui.close();
    }
    if ui
        .button(syfrack_i18n::tr!("Orbiter autour d'ici"))
        .clicked()
    {
        let roll = match state.scene.camera.mode {
            CameraMode::Orbit { roll, .. } | CameraMode::Free { roll, .. } => roll,
        };
        let mode = input::orbit_around(&basis, *point, roll);
        state.edit(|s| s.camera.mode = mode);
        ui.close();
    }
    let areas: Vec<(usize, Light)> = state
        .scene
        .lights
        .iter()
        .enumerate()
        .filter(|(_, l)| matches!(l, Light::Area { .. } | Light::Projector { .. }))
        .map(|(i, l)| (i, l.clone()))
        .collect();
    for (n, (i, light)) in areas.iter().enumerate() {
        let label = if areas.len() > 1 {
            syfrack_i18n::tr_format!("Viser la surfacique {} ici", n + 1)
        } else {
            syfrack_i18n::tr!("Viser la surfacique ici").to_string()
        };
        if ui.button(label).clicked() {
            if let Some(aimed) = gizmos::aim_area_at(light, *point) {
                state.edit(|s| s.lights[*i] = aimed);
            }
            ui.close();
        }
    }
    let room = state.scene.lights.len() < MAX_LIGHTS;
    if ui
        .add_enabled(
            room,
            egui::Button::new(syfrack_i18n::tr!("Placer une surfacique ici")),
        )
        .clicked()
    {
        let n = Vec3::from(pick.normal).normalize_or(Vec3::Y);
        let light = new_area(*point + n * AREA_STANDOFF, -n);
        state.edit(|s| s.lights.push(light));
        ui.close();
    }
}

/// Draws the preview image filling the central area and turns navigation
/// into camera edits. Orbit: left drag rotates, middle drag pans, wheel zooms.
/// Free: right drag looks, Z/Q/S/D (logical keys, so W/A on QWERTY) move, E/C rise and sink.
/// Arrows roll. Keys are ignored with a modifier held or while a text field has focus.
/// A right click opens the pick menu.
pub fn show(ui: &mut egui::Ui, state: &mut AppState, view: &mut Viewport) {
    let mut out = ViewportOutput::default();
    let avail = ui.available_size();
    let sized = view.texture.filter(|_| view.size.0 > 0 && view.size.1 > 0);
    let Some(id) = sized else {
        // No image: say why when the app knows (« forme incomplète »).
        let text = if view.caption.is_empty() {
            syfrack_i18n::tr!("aperçu")
        } else {
            view.caption.as_str()
        };
        ui.centered_and_justified(|ui| {
            ui.label(egui::RichText::new(text).color(theme::TEXT_DIM));
        });
        view.output = out;
        return;
    };

    let aspect = view.size.0 as f32 / view.size.1 as f32;
    let (mut w, mut h) = (avail.x, avail.x / aspect);
    if h > avail.y {
        (w, h) = (avail.y * aspect, avail.y);
    }
    let area = ui.max_rect();
    let rect = egui::Rect::from_center_size(area.center(), egui::vec2(w, h));
    let response = ui.allocate_rect(rect, Sense::click_and_drag());
    egui::Image::new((id, rect.size())).paint_at(ui, rect);
    if let Some(output) = view.frame_guide {
        draw_frame_guide(ui, rect, output);
    }

    if state.scene.dimension == syfrack_core::Dimension::TwoD {
        navigate_canvas(ui, state, &response, rect, aspect);
        overlay_text(
            ui,
            rect.left_bottom() + egui::vec2(theme::GRID, -2.0 * theme::GRID),
            syfrack_i18n::tr!("2D · glisser pour déplacer · molette pour zoomer"),
        );
        view.output = out;
        return;
    }

    let gizmo_busy = state.show_gizmos && show_gizmos(ui, state, rect, aspect);

    let dt = ui.input(|i| i.stable_dt).min(0.1);
    let drag = response.drag_delta();
    let moved = drag != egui::Vec2::ZERO && !gizmo_busy;
    let is_orbit = matches!(state.scene.camera.mode, CameraMode::Orbit { .. });
    if moved && is_orbit {
        if response.dragged_by(PointerButton::Primary) {
            move_camera(ui, state, false, |c| input::orbit_drag(c, drag.x, drag.y));
        }
        if response.dragged_by(PointerButton::Middle) {
            move_camera(ui, state, false, |c| {
                input::orbit_pan(c, drag.x, drag.y, rect.width())
            });
        }
    } else if moved && response.dragged_by(PointerButton::Secondary) {
        move_camera(ui, state, false, |c| input::free_look(c, drag.x, drag.y));
    }

    if response.hovered() {
        let notches: f32 = ui.input(|i| {
            i.events
                .iter()
                .map(|e| match e {
                    egui::Event::MouseWheel { unit, delta, .. } => wheel_notches(*unit, delta.y),
                    _ => 0.0,
                })
                .sum()
        });
        if notches != 0.0 && is_orbit {
            move_camera(ui, state, true, |c| input::orbit_zoom(c, notches));
        }
        let keys_ok = !ui.ctx().egui_wants_keyboard_input() && !ui.input(|i| i.modifiers.any());
        let mut local = Vec3::ZERO;
        let mut roll = 0.0;
        if keys_ok {
            ui.input(|i| {
                if i.key_down(Key::Z) || i.key_down(Key::W) {
                    local.z += 1.0;
                }
                if i.key_down(Key::S) {
                    local.z -= 1.0;
                }
                if i.key_down(Key::D) {
                    local.x += 1.0;
                }
                if i.key_down(Key::Q) || i.key_down(Key::A) {
                    local.x -= 1.0;
                }
                if i.key_down(Key::E) {
                    local.y += 1.0;
                }
                if i.key_down(Key::C) {
                    local.y -= 1.0;
                }
                if i.key_down(Key::ArrowLeft) {
                    roll -= ROLL_DEG_PER_S * dt;
                }
                if i.key_down(Key::ArrowRight) {
                    roll += ROLL_DEG_PER_S * dt;
                }
            });
        }
        if roll != 0.0 {
            move_camera(ui, state, true, |c| input::roll(c, roll));
        }
        if local != Vec3::ZERO && !is_orbit {
            let hint = view.depth_hint;
            move_camera(ui, state, true, |c| input::free_move(c, local, hint, dt));
        }
        if let Some(pos) = response.hover_pos() {
            let uv = (
                ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0),
                ((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0),
            );
            out.hover_uv = Some(uv);
            if response.clicked_by(PointerButton::Primary) || response.secondary_clicked() {
                out.pick_request = Some(uv);
            }
            out.pointer_still = ui.input(|i| i.pointer.velocity().length() < 1.0);
        }
    }

    if let Some((point, pick)) = &view.pick {
        overlay_text(
            ui,
            rect.left_bottom() + egui::vec2(theme::GRID, -4.0 * theme::GRID - 2.0),
            &pick_label(*point, pick),
        );
    }
    response.context_menu(|ui| pick_menu(ui, state, view.pick.as_ref()));
    view.output = out;
}

fn navigate_canvas(
    ui: &egui::Ui,
    state: &mut AppState,
    response: &egui::Response,
    rect: egui::Rect,
    aspect: f32,
) {
    let drag = response.drag_delta();
    if (response.dragged_by(PointerButton::Primary) || response.dragged_by(PointerButton::Middle))
        && drag != egui::Vec2::ZERO
    {
        let mut canvas = state.scene.canvas.clone();
        let a = canvas.point([0.5, 0.5], aspect);
        let b = canvas.point(
            [0.5 + drag.x / rect.width(), 0.5 + drag.y / rect.height()],
            aspect,
        );
        let shift = a - b;
        canvas.center[0] = (canvas.center[0] + shift.x).clamp(-100.0, 100.0);
        canvas.center[1] = (canvas.center[1] + shift.y).clamp(-100.0, 100.0);
        state.edit(|s| s.canvas = canvas);
    }
    if response.hovered() {
        let notches: f32 = ui.input(|i| {
            i.events
                .iter()
                .map(|e| match e {
                    egui::Event::MouseWheel { unit, delta, .. } => wheel_notches(*unit, delta.y),
                    _ => 0.0,
                })
                .sum()
        });
        if notches != 0.0 {
            let mut canvas = state.scene.canvas.clone();
            let pos = response.hover_pos().unwrap_or(rect.center());
            let uv = [
                (pos.x - rect.left()) / rect.width(),
                (pos.y - rect.top()) / rect.height(),
            ];
            let anchor = canvas.point(uv, aspect);
            canvas.zoom =
                (canvas.zoom * 1.15_f32.powf(notches.clamp(-20.0, 20.0))).clamp(0.01, 10000.0);
            let shift = anchor - canvas.point(uv, aspect);
            canvas.center[0] = (canvas.center[0] + shift.x).clamp(-100.0, 100.0);
            canvas.center[1] = (canvas.center[1] + shift.y).clamp(-100.0, 100.0);
            let now = ui.input(|i| i.time);
            ui.data_mut(|d| d.insert_temp(nav_id(), now));
            state.begin_drag();
            state.edit(|s| s.canvas = canvas);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::MouseWheelUnit::{Line, Page, Point};

    #[test]
    fn a_wide_output_in_a_squarer_viewport_leaves_horizontal_bands() {
        let view = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(800.0, 600.0));
        let g = frame_guide_rect(view, (1920, 1080));
        assert_eq!(g.rect.width(), 800.0, "the output spans the full width");
        assert_eq!(g.rect.height(), 450.0);
        assert_eq!(g.rect.top() - view.top(), 75.0, "top band");
        assert_eq!(view.bottom() - g.rect.bottom(), 75.0, "bottom band");
        assert_eq!(g.rect.center(), view.center());
        assert!(!g.taller);
    }

    #[test]
    fn a_tall_output_spans_the_width_and_runs_past_the_top_and_bottom() {
        let view = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1600.0, 900.0));
        let g = frame_guide_rect(view, (1000, 1000));
        assert_eq!(
            g.rect.width(),
            1600.0,
            "still the full width, no side bands"
        );
        assert_eq!(g.rect.height(), 1600.0);
        assert_eq!(view.top() - g.rect.top(), 350.0, "past the top");
        assert_eq!(g.rect.bottom() - view.bottom(), 350.0, "past the bottom");
        assert!(g.taller);
    }

    #[test]
    fn the_viewport_own_aspect_leaves_no_band() {
        let view = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1600.0, 900.0));
        let g = frame_guide_rect(view, (3840, 2160));
        assert_eq!(g.rect, view);
        assert!(!g.taller);
        assert_eq!(
            frame_guide_rect(view, (0, 10)).rect,
            view,
            "degenerate output"
        );
    }

    #[test]
    fn focus_here_stays_inside_the_focus_bounds() {
        let scene = syfrack_core::Scene::example();
        let b = scene.camera.basis();
        assert!((focus_here(&scene, b.origin + b.forward * 3.0) - 3.0).abs() < 1e-4);
        let behind = focus_here(&scene, b.origin - b.forward);
        let far = focus_here(&scene, b.origin + b.forward * 1e6);
        let bounds = crate::ui::panels::bounds_of(&scene, "camera.focus_distance").unwrap();
        assert_eq!((behind, far), (bounds.min, bounds.max));
    }

    #[test]
    fn one_wheel_click_is_one_notch() {
        assert_eq!(wheel_notches(Line, 1.0), 1.0);
        assert_eq!(wheel_notches(Line, -4.0), -4.0);
        assert!((wheel_notches(Point, 50.0) - 1.0).abs() < 1e-6);
        assert_eq!(wheel_notches(Page, 1.0), 10.0);
    }
}
