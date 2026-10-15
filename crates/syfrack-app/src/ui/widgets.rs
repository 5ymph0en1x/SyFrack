//! The studio's controls. One bounded slider for every number, and a few
//! controls drawn for what they set: black-body temperature, colour ramp,
//! light direction, linear colour seen through the look.

use egui::{Align2, Color32, Painter, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};
use glam::Vec3;
use syfrack_core::{Bounds, Color, HdrImage, Look, Ramp};

use crate::ui::theme;

/// Height of one parameter row: three grid steps.
pub const ROW_H: f32 = 4.0 * theme::GRID;
/// Width of the label column, shared by every row so rails line up.
pub const LABEL_W: f32 = 12.0 * theme::GRID;
/// Width of the value + unit column at the right of a row.
pub const VALUE_W: f32 = 10.0 * theme::GRID;
/// Width reserved for the unit inside the value column.
const UNIT_W: f32 = 3.0 * theme::GRID;
/// Rail colour of the interesting range: a muted accent.
const INTERESTING: Color32 = Color32::from_rgb(0x56, 0x70, 0x73);

/// Wider labels in the workspace inspector; compact rows on the node canvas.
pub fn label_width(width: f32) -> f32 {
    if width >= 320.0 {
        144.0
    } else {
        LABEL_W
    }
}
/// Pointer travel, in points, past which a press on a card rail drags it.
const CLICK_TRAVEL: f32 = 3.0;
/// Number of flat swatches painted along a gradient.
const GRADIENT_STEPS: usize = 48;

/// One consistent exclusive-choice control: bounded hit area, selected
/// semantics for accessibility, and a persistent border/fill when active.
pub fn choice_button(ui: &mut Ui, selected: bool, label: impl Into<egui::WidgetText>) -> Response {
    let response = ui
        .add(
            egui::Button::new(label)
                .selected(selected)
                .corner_radius(4.0)
                .fill(if selected {
                    theme::SELECTED
                } else {
                    theme::FIELD
                })
                .stroke(Stroke::new(
                    1.0,
                    if selected {
                        theme::ACCENT
                    } else {
                        theme::SEPARATOR
                    },
                )),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if ui.is_enabled() && (response.hovered() || response.has_focus()) {
        ui.painter().rect_stroke(
            response.rect.shrink(0.5),
            4.0,
            Stroke::new(if response.has_focus() { 2.0 } else { 1.0 }, theme::ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    response
}

/// A full-width menu row keeps the selection obvious even with long labels.
pub fn menu_choice(ui: &mut Ui, selected: bool, label: &str) -> Response {
    let label = syfrack_i18n::text(label);
    ui.add_sized(
        [ui.available_width().max(180.0), 28.0],
        egui::Button::new("")
            .left_text(label)
            .selected(selected)
            .frame(selected),
    )
}

fn log_min(b: Bounds) -> f32 {
    if b.min > 0.0 {
        b.min
    } else {
        (b.max * 1e-3).max(1e-6)
    }
}

/// Value -> rail position in [0, 1]. Log rails floor a non-positive minimum
/// at 1e-3 of the maximum.
pub fn slider_map(v: f32, b: Bounds, log: bool) -> f32 {
    let t = if log {
        let lo = log_min(b).ln();
        let hi = b.max.ln();
        (v.max(log_min(b)).ln() - lo) / (hi - lo)
    } else {
        (v - b.min) / (b.max - b.min)
    };
    t.clamp(0.0, 1.0)
}

/// Rail position in [0, 1] -> value inside the bounds. The left end of a log
/// rail whose minimum is not positive is that minimum itself (usually 0).
pub fn slider_unmap(t: f32, b: Bounds, log: bool) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if log {
        let lo = log_min(b).ln();
        let hi = b.max.ln();
        let v = (lo + t * (hi - lo)).exp();
        if t == 0.0 && b.min <= 0.0 {
            b.min
        } else {
            v.clamp(b.min, b.max)
        }
    } else {
        b.min + t * (b.max - b.min)
    }
}

/// A value as the studio prints it: fixed decimals and the selected decimal separator,
/// never a negative zero.
pub fn format_value(v: f32, decimals: usize) -> String {
    let s = format!("{v:.decimals$}");
    let negative_zero = s.starts_with('-') && s[1..].chars().all(|c| c == '0' || c == '.');
    let s = if negative_zero { &s[1..] } else { &s[..] };
    match syfrack_i18n::language() {
        syfrack_i18n::Language::English => s.to_owned(),
        syfrack_i18n::Language::French => s.replace('.', ","),
    }
}

/// Parses a typed value; accepts a decimal comma or point.
pub fn parse_value(text: &str) -> Option<f32> {
    text.trim()
        .replace(',', ".")
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite())
}

/// 8-bit sRGB of a display-linear colour.
pub fn to_srgb32(c: [f32; 3]) -> Color32 {
    let e = |v: f32| (syfrack_color::encode::linear_to_srgb(v) * 255.0).round() as u8;
    Color32::from_rgb(e(c[0]), e(c[1]), e(c[2]))
}

/// The three columns of a parameter row: label, rail, value + unit.
struct Row {
    rect: Rect,
    rail: Rect,
    value: Rect,
}

impl Row {
    fn new(rect: Rect, rail_half_h: f32) -> Self {
        let cy = rect.center().y;
        let rail = Rect::from_min_max(
            Pos2::new(rect.left() + label_width(rect.width()), cy - rail_half_h),
            Pos2::new(rect.right() - VALUE_W - theme::GRID, cy + rail_half_h),
        );
        let value = Rect::from_min_max(
            Pos2::new(rect.right() - VALUE_W, rect.top()),
            rect.right_bottom(),
        );
        Self { rect, rail, value }
    }

    fn x_of(&self, t: f32) -> f32 {
        self.rail.left() + t * self.rail.width()
    }

    fn label(&self, ui: &Ui, painter: &Painter, label: &str) {
        painter.text(
            Pos2::new(self.rect.left(), self.rect.center().y),
            Align2::LEFT_CENTER,
            label,
            egui::TextStyle::Body.resolve(ui.style()),
            theme::TEXT,
        );
    }
}

/// Paints the tabular value right-aligned before its grey unit or, after a
/// double click on it, a text field: Enter or focus loss commits (clamped to
/// the bounds), Escape cancels.
fn value_cell(
    ui: &mut Ui,
    row: &Row,
    response: &mut Response,
    value: &mut f32,
    bounds: Bounds,
    decimals: usize,
    unit: &str,
) {
    let edit_id = response.id.with("edit");
    let number = Rect::from_min_max(
        row.value.min,
        Pos2::new(row.value.right() - UNIT_W, row.value.bottom()),
    );
    let starts = response.double_clicked()
        && response
            .interact_pointer_pos()
            .is_some_and(|p| row.value.contains(p));
    if starts {
        let text = format_value(*value, decimals);
        ui.data_mut(|d| d.insert_temp(edit_id, text));
    }
    let editing: Option<String> = ui.data(|d| d.get_temp(edit_id));
    match editing {
        Some(mut text) => {
            let mut child =
                ui.new_child(egui::UiBuilder::new().max_rect(number.shrink2(Vec2::new(0.0, 2.0))));
            let te = egui::TextEdit::singleline(&mut text)
                .font(egui::TextStyle::Monospace)
                .horizontal_align(egui::Align::RIGHT)
                .desired_width(number.width())
                .show(&mut child)
                .response;
            if starts {
                te.request_focus();
            }
            if te.lost_focus() {
                ui.data_mut(|d| d.remove::<String>(edit_id));
                let cancelled = ui.input(|i| i.key_pressed(egui::Key::Escape));
                if let (false, Some(v)) = (cancelled, parse_value(&text)) {
                    let v = v.clamp(bounds.min, bounds.max);
                    if v != *value {
                        *value = v;
                        response.mark_changed();
                    }
                }
            } else {
                ui.data_mut(|d| d.insert_temp(edit_id, text));
            }
        }
        None => {
            ui.painter_at(row.rect).text(
                Pos2::new(number.right(), number.center().y),
                Align2::RIGHT_CENTER,
                format_value(*value, decimals),
                egui::TextStyle::Monospace.resolve(ui.style()),
                theme::TEXT,
            );
        }
    }
    ui.painter_at(row.rect).text(
        Pos2::new(row.value.right(), row.value.center().y),
        Align2::RIGHT_CENTER,
        unit,
        egui::TextStyle::Small.resolve(ui.style()),
        theme::TEXT_DIM,
    );
}

/// Moves `value` under the pointer while a press that began on the rail lasts.
fn rail_drag(
    ui: &Ui,
    row: &Row,
    response: &mut Response,
    value: &mut f32,
    unmap: impl Fn(f32) -> f32,
) {
    if !(response.dragged() || response.clicked()) {
        return;
    }
    let grab = row.rail.expand2(Vec2::new(theme::GRID, ROW_H));
    // On the release frame of a click the press origin is gone: the click
    // position stands for it.
    let began_on_rail = ui
        .input(|i| i.pointer.press_origin())
        .or_else(|| response.interact_pointer_pos())
        .is_some_and(|p| grab.contains(p));
    if let (true, Some(pos)) = (began_on_rail, response.interact_pointer_pos()) {
        let t = ((pos.x - row.rail.left()) / row.rail.width()).clamp(0.0, 1.0);
        let v = unmap(t);
        if v != *value {
            *value = v;
            response.mark_changed();
        }
    }
}

/// Paints a horizontal gradient over `rect`, `color(t)` for t in [0, 1].
fn gradient(painter: &Painter, rect: Rect, color: impl Fn(f32) -> [f32; 3]) {
    for i in 0..GRADIENT_STEPS {
        let t0 = i as f32 / GRADIENT_STEPS as f32;
        let t1 = (i + 1) as f32 / GRADIENT_STEPS as f32;
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(rect.left() + t0 * rect.width(), rect.top()),
                Pos2::new(rect.left() + t1 * rect.width() + 0.5, rect.bottom()),
            ),
            0.0,
            to_srgb32(color((t0 + t1) * 0.5)),
        );
    }
}

/// The studio slider: label left; a rail that shows the interesting range
/// lighter and the reference value as a fine tick; tabular value and grey
/// unit right. Double click on the value types it; elsewhere it restores
/// the reference value.
pub struct ParamSlider<'a> {
    label: &'a str,
    unit: &'a str,
    bounds: Bounds,
    log: bool,
    default: Option<f32>,
    decimals: usize,
    topology: bool,
}

/// Marks a parameter whose change rebuilds the shader (an integer of the
/// form graph): « ⟳ » before its label.
pub const TOPOLOGY_MARK: &str = "⟳";

impl<'a> ParamSlider<'a> {
    pub fn new(label: &'a str, bounds: Bounds) -> Self {
        Self {
            label,
            unit: "",
            bounds,
            log: false,
            default: None,
            decimals: 2,
            topology: false,
        }
    }

    /// A topology parameter: « ⟳ » before the label, whole values only.
    pub fn topology(mut self) -> Self {
        self.topology = true;
        self.decimals = 0;
        self
    }

    pub fn unit(mut self, unit: &'a str) -> Self {
        self.unit = unit;
        self
    }

    pub fn log(mut self) -> Self {
        self.log = true;
        self
    }

    pub fn default_value(mut self, v: f32) -> Self {
        self.default = Some(v);
        self
    }

    pub fn decimals(mut self, d: usize) -> Self {
        self.decimals = d;
        self
    }

    pub fn show(self, ui: &mut Ui, value: &mut f32) -> Response {
        let (rect, mut response) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), ROW_H),
            Sense::click_and_drag(),
        );
        let row = Row::new(rect, 1.5);
        let (b, log, whole) = (self.bounds, self.log, self.topology);
        rail_drag(ui, &row, &mut response, value, |t| {
            let v = slider_unmap(t, b, log);
            if whole {
                v.round()
            } else {
                v
            }
        });
        if response.double_clicked() {
            let on_value = response
                .interact_pointer_pos()
                .is_some_and(|p| row.value.contains(p));
            if let (false, Some(d)) = (on_value, self.default) {
                if d != *value {
                    *value = d;
                    response.mark_changed();
                }
            }
        }
        value_cell(ui, &row, &mut response, value, b, self.decimals, self.unit);

        let painter = ui.painter_at(rect);
        if self.topology {
            row.label(ui, &painter, &format!("{TOPOLOGY_MARK} {}", self.label));
        } else {
            row.label(ui, &painter, self.label);
        }
        painter.rect_filled(row.rail, 1.5, theme::SEPARATOR);
        for &(lo, hi) in b.interesting {
            let a = row.x_of(slider_map(lo, b, log));
            let z = row.x_of(slider_map(hi, b, log)).max(a + 2.0);
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(a, row.rail.top()),
                    Pos2::new(z, row.rail.bottom()),
                ),
                1.5,
                INTERESTING,
            );
        }
        if let Some(d) = self.default {
            let x = row.x_of(slider_map(d, b, log));
            painter.line_segment(
                [
                    Pos2::new(x, row.rail.top() - 4.0),
                    Pos2::new(x, row.rail.bottom() + 4.0),
                ],
                Stroke::new(1.0, theme::TEXT_DIM),
            );
        }
        let at = Pos2::new(row.x_of(slider_map(*value, b, log)), row.rail.center().y);
        let active = response.dragged() || response.hovered();
        painter.circle_filled(at, if active { 6.0 } else { 5.0 }, theme::ACCENT);
        response.on_hover_text(syfrack_i18n::tr_format!(
            "{} · {} … {}\nGlisser le curseur ; double-clic sur la valeur pour saisir au clavier.",
            self.label,
            format_value(b.min, self.decimals),
            format_value(b.max, self.decimals)
        ))
    }
}

/// The rail of one inline parameter inside a node card, drawn at the canvas
/// zoom `z`: interesting range, reference tick and knob. With an `id` it is
/// live: a press on the knob, or a drag past 3 px, moves the value under the
/// pointer (whole values when `whole`); a plain click writes nothing; a
/// double click restores `default`. Returns true on change.
#[allow(clippy::too_many_arguments)]
pub fn card_rail(
    ui: &Ui,
    id: Option<egui::Id>,
    rail: Rect,
    value: &mut f32,
    bounds: Bounds,
    log: bool,
    whole: bool,
    default: Option<f32>,
    z: f32,
) -> bool {
    let grab = rail.expand2(Vec2::new(4.0 * z, 6.0 * z));
    let response = id.map(|id| ui.interact(grab, id, Sense::click_and_drag()));
    let mut changed = false;
    let x_at = |v: f32| rail.left() + slider_map(v, bounds, log) * rail.width();
    if let Some(r) = &response {
        // A plain click writes nothing: the value follows the pointer once
        // the press started on the knob or travelled past 3 px.
        let armed_id = r.id.with("armed");
        let mut armed = ui.data(|d| d.get_temp::<bool>(armed_id)).unwrap_or(false);
        if r.is_pointer_button_down_on() && !armed {
            let origin = ui.input(|i| i.pointer.press_origin());
            let pos = r.interact_pointer_pos();
            let knob = Pos2::new(x_at(*value), rail.center().y);
            armed = match (origin, pos) {
                (Some(o), Some(p)) => {
                    (o - knob).length() <= 6.0 * z || (p - o).length() > CLICK_TRAVEL
                }
                _ => false,
            };
            if armed {
                ui.data_mut(|d| d.insert_temp(armed_id, true));
            }
        }
        if !r.is_pointer_button_down_on() && !r.dragged() {
            ui.data_mut(|d| d.remove::<bool>(armed_id));
        }
        if armed && (r.dragged() || r.is_pointer_button_down_on() || r.drag_stopped()) {
            if let Some(pos) = r.interact_pointer_pos() {
                let t = ((pos.x - rail.left()) / rail.width().max(1.0)).clamp(0.0, 1.0);
                let mut v = slider_unmap(t, bounds, log);
                if whole {
                    v = v.round();
                }
                changed |= v != *value;
                *value = v;
            }
        }
        if let (true, Some(d)) = (r.double_clicked(), default) {
            changed |= d != *value;
            *value = d;
        }
    }
    let painter = ui.painter_at(grab);
    let x_of = x_at;
    painter.rect_filled(rail, 1.0, theme::SEPARATOR);
    for &(lo, hi) in bounds.interesting {
        let a = x_of(lo);
        let b = x_of(hi).max(a + 2.0);
        painter.rect_filled(
            Rect::from_min_max(Pos2::new(a, rail.top()), Pos2::new(b, rail.bottom())),
            1.0,
            INTERESTING,
        );
    }
    if let Some(d) = default {
        let x = x_of(d);
        painter.line_segment(
            [
                Pos2::new(x, rail.top() - 2.0 * z),
                Pos2::new(x, rail.bottom() + 2.0 * z),
            ],
            Stroke::new(1.0, theme::TEXT_DIM),
        );
    }
    let active = response.is_some_and(|r| r.dragged() || r.hovered());
    painter.circle_filled(
        Pos2::new(x_of(*value), rail.center().y),
        (if active { 4.5 } else { 3.5 }) * z,
        theme::ACCENT,
    );
    changed
}

/// A Kelvin slider whose rail is the black-body gradient itself.
pub fn temperature_slider(ui: &mut Ui, label: &str, kelvin: &mut f32, bounds: Bounds) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), ROW_H),
        Sense::click_and_drag(),
    );
    let row = Row::new(rect, 4.0);
    rail_drag(ui, &row, &mut response, kelvin, |t| {
        slider_unmap(t, bounds, true).round()
    });
    value_cell(ui, &row, &mut response, kelvin, bounds, 0, "K");

    let painter = ui.painter_at(rect);
    row.label(ui, &painter, label);
    gradient(&painter, row.rail, |t| {
        Color::Kelvin(slider_unmap(t, bounds, true)).to_linear()
    });
    let at = Pos2::new(
        row.x_of(slider_map(*kelvin, bounds, true)),
        row.rail.center().y,
    );
    let r = if response.dragged() || response.hovered() {
        6.0
    } else {
        5.0
    };
    painter.circle_stroke(at, r, Stroke::new(2.0, theme::TEXT));
    response
}

/// Editable palette. Horizontal dragging places stops; vertical dragging adjusts brightness.
pub fn ramp_editor(ui: &mut Ui, ramp: &mut Ramp) -> bool {
    use syfrack_core::{PaletteInterpolation, MAX_PALETTE_STOPS};
    let mut changed = false;
    let mut base = ramp.clone();
    base.frequency = 1.0;
    base.phase = 0.0;
    base.gamma = 1.0;
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), ROW_H),
        Sense::click_and_drag(),
    );
    let band = rect.shrink2(Vec2::new(theme::GRID, 6.0));
    let count = ramp.stops.len();
    let positions: Vec<f32> = (0..count).map(|i| ramp.position(i)).collect();
    let drag_id = response.id.with("stop");
    let dragging: Option<usize> = ui.data(|d| d.get_temp(drag_id));
    let near_stop = |x: f32| {
        positions
            .iter()
            .enumerate()
            .map(|(i, t)| (i, (x - (band.left() + t * band.width())).abs()))
            .filter(|(_, d)| *d <= 7.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    };
    if let Some(pos) = response.interact_pointer_pos() {
        if response.drag_started() {
            if let Some(i) = near_stop(pos.x) {
                ui.data_mut(|d| d.insert_temp(drag_id, i));
            }
        }
        if let (true, Some(i)) = (response.dragged(), dragging.filter(|i| *i < count)) {
            if pos.y < rect.top() - 16.0 || pos.y > rect.bottom() + 16.0 {
                if count > 2 {
                    ramp.stops.remove(i);
                    if !ramp.positions.is_empty() {
                        ramp.positions.remove(i);
                    }
                    ui.data_mut(|d| d.remove::<usize>(drag_id));
                    changed = true;
                }
            } else {
                let delta = ui.input(|i| i.pointer.delta());
                if delta.x != 0.0 {
                    if ramp.positions.is_empty() {
                        ramp.positions = positions.clone();
                    }
                    let lo = if i == 0 {
                        0.0
                    } else {
                        ramp.positions[i - 1] + 0.001
                    };
                    let hi = if i + 1 == count {
                        if ramp.cyclic {
                            0.999
                        } else {
                            1.0
                        }
                    } else {
                        ramp.positions[i + 1] - 0.001
                    };
                    if lo <= hi {
                        ramp.positions[i] = ((pos.x - band.left()) / band.width()).clamp(lo, hi);
                        changed = true;
                    }
                }
                if delta.y != 0.0 {
                    let k = (1.0 - delta.y * 0.01).clamp(0.5, 1.5);
                    for c in &mut ramp.stops[i] {
                        *c = (*c * k).clamp(0.0, 1.0);
                    }
                    changed = true;
                }
            }
        }
        if response.drag_stopped() {
            ui.data_mut(|d| d.remove::<usize>(drag_id));
        }
        if response.clicked() && count < MAX_PALETTE_STOPS && near_stop(pos.x).is_none() {
            let t = ((pos.x - band.left()) / band.width())
                .clamp(0.0, if ramp.cyclic { 0.999 } else { 1.0 });
            let index = positions.partition_point(|p| *p < t);
            ramp.positions = positions;
            ramp.positions.insert(index, t);
            ramp.stops.insert(index, base.sample(t));
            changed = true;
        }
    }
    let painter = ui.painter_at(rect);
    gradient(&painter, band, |t| base.sample(t));
    for (i, stop) in ramp.stops.iter().enumerate() {
        let at = Pos2::new(
            band.left() + ramp.position(i) * band.width(),
            band.center().y,
        );
        painter.circle_filled(at, 5.0, to_srgb32(*stop));
        painter.circle_stroke(
            at,
            5.0,
            Stroke::new(
                1.5,
                if dragging == Some(i) {
                    theme::ACCENT
                } else {
                    theme::TEXT
                },
            ),
        );
    }
    ui.horizontal_wrapped(|ui| {
        for (i, c) in ramp.stops.iter_mut().enumerate() {
            ui.push_id(("ramp-stop", i), |ui| {
                changed |= ui.color_edit_button_rgb(c).changed();
            });
        }
    });
    ui.small(syfrack_i18n::tr!("Cliquer pour ajouter une couleur · glisser pour la placer · sortir de la bande pour la retirer."));
    if !ramp.positions.is_empty()
        && ui
            .small_button(syfrack_i18n::tr!("Espacer régulièrement"))
            .clicked()
    {
        ramp.positions.clear();
        changed = true;
    }
    ui.horizontal(|ui| {
        if ui
            .checkbox(&mut ramp.cyclic, syfrack_i18n::tr!("Palette cyclique"))
            .changed()
        {
            // Keep a nonzero closing interval when enabling cycling on explicit positions.
            if ramp.cyclic && ramp.positions.last().is_some_and(|p| *p >= 1.0) {
                let factor = (ramp.stops.len() - 1) as f32 / ramp.stops.len() as f32;
                for p in &mut ramp.positions {
                    *p *= factor;
                }
            }
            changed = true;
        }
        let smooth = ramp.interpolation == PaletteInterpolation::Smooth;
        let mut selected = smooth;
        if ui
            .checkbox(&mut selected, syfrack_i18n::tr!("Transitions douces"))
            .changed()
        {
            ramp.interpolation = if selected {
                PaletteInterpolation::Smooth
            } else {
                PaletteInterpolation::Linear
            };
            changed = true;
        }
    });
    for (label, value, range) in [
        (
            syfrack_i18n::tr!("Fréquence"),
            &mut ramp.frequency,
            0.01..=256.0,
        ),
        ("Phase", &mut ramp.phase, -256.0..=256.0),
        ("Gamma", &mut ramp.gamma, 0.05..=20.0),
    ] {
        ui.horizontal(|ui| {
            ui.label(label);
            changed |= ui
                .add(egui::DragValue::new(value).range(range).speed(0.01))
                .changed();
        });
    }
    // Show the actual repeated palette separately from the editable base cycle.
    let (preview, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), theme::GRID), Sense::hover());
    gradient(&ui.painter_at(preview), preview, |t| ramp.sample(t));
    changed
}

/// Azimuth (from +z toward +x) and elevation of a direction, in degrees.
pub fn direction_angles(dir: Vec3) -> (f32, f32) {
    let d = dir.normalize_or(Vec3::Y);
    (
        d.x.atan2(d.z).to_degrees(),
        d.y.clamp(-1.0, 1.0).asin().to_degrees(),
    )
}

/// A small sphere: the direction is a dot on it, bright on the front
/// hemisphere; dragging rotates the direction.
pub fn direction_compass(ui: &mut Ui, dir: &mut Vec3, size: f32) -> bool {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::drag());
    let painter = ui.painter_at(rect);
    let c = rect.center();
    let r = size * 0.45;
    painter.circle_filled(c, r, theme::SEPARATOR);
    painter.circle_stroke(c, r, Stroke::new(1.0, theme::TEXT_DIM));
    painter.line_segment(
        [Pos2::new(c.x - r, c.y), Pos2::new(c.x + r, c.y)],
        Stroke::new(1.0, theme::BG),
    );
    let d = dir.normalize_or(Vec3::Y);
    // Screen: x right, y up, z toward the viewer (front hemisphere bright).
    let p = Pos2::new(c.x + d.x * r, c.y - d.y * r);
    let col = if d.z >= 0.0 {
        theme::ACCENT
    } else {
        theme::TEXT_DIM
    };
    painter.line_segment([c, p], Stroke::new(1.5, col));
    painter.circle_filled(p, 4.0, col);
    if !response.dragged() {
        return false;
    }
    let delta = response.drag_delta();
    let yaw = glam::Quat::from_rotation_y(delta.x * 0.01);
    let pitch = glam::Quat::from_rotation_x(delta.y * 0.01);
    let nd = (pitch * (yaw * d)).normalize();
    if nd == d {
        return false;
    }
    *dir = nd;
    true
}

/// How the scene-linear colour `c` looks after `look`: exposure, balance,
/// grade, curve and saturation. Auto exposure, vignette and grain are
/// image-wide effects and are left out.
pub fn developed(c: [f32; 3], look: &Look) -> [f32; 3] {
    let mut look = look.clone();
    look.auto_exposure = None;
    look.vignette = None;
    look.grain = None;
    let mut img = HdrImage::new(1, 1);
    img.set(0, 0, [c[0], c[1], c[2], 1.0]);
    let d = syfrack_color::develop(&img, &look).get(0, 0);
    [d[0], d[1], d[2]]
}

/// Each channel held inside the bounds of its parameter.
pub fn clamp_rgb(c: [f32; 3], b: Bounds) -> [f32; 3] {
    c.map(|v| v.clamp(b.min, b.max))
}

/// A linear colour row: label, a wide swatch of the colour as the current
/// look develops it, and the picker button holding the raw linear value;
/// picked channels are held inside `bounds`.
pub fn linear_color(
    ui: &mut Ui,
    label: &str,
    c: &mut [f32; 3],
    bounds: Bounds,
    look: &Look,
) -> bool {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::hover());
    let row = Row::new(rect, 6.0);
    let painter = ui.painter_at(rect);
    row.label(ui, &painter, label);
    painter.rect_filled(row.rail, 2.0, to_srgb32(developed(*c, look)));
    let cell = Rect::from_min_max(
        Pos2::new(row.value.right() - UNIT_W, rect.top() + 4.0),
        Pos2::new(row.value.right(), rect.bottom() - 4.0),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(cell)
            .id_salt(("linear-color", label)),
    );
    child.spacing_mut().interact_size = cell.size();
    let mut rgb = *c;
    let mut changed = false;
    if bounds.max > 1.0 {
        let r = child
            .add_sized(cell.size(), egui::Button::new("…"))
            .on_hover_text(syfrack_i18n::tr!("Couleur HDR · valeurs RVB linéaires"));
        egui::Popup::from_toggle_button_response(&r).show(|ui| {
            ui.set_width(260.0);
            ui.label(egui::RichText::new(label).strong());
            for (i, name) in [
                syfrack_i18n::tr!("Rouge"),
                syfrack_i18n::tr!("Vert"),
                syfrack_i18n::tr!("Bleu"),
            ]
            .iter()
            .enumerate()
            {
                changed |= ui
                    .add(
                        egui::Slider::new(&mut rgb[i], bounds.min..=bounds.max)
                            .text(*name)
                            .fixed_decimals(3),
                    )
                    .changed();
            }
            ui.small(syfrack_i18n::tr!(
                "Valeurs linéaires ; au-delà de 1, la couleur émet davantage de lumière."
            ));
        });
    } else {
        changed = child
            .color_edit_button_rgb(&mut rgb)
            .on_hover_text(syfrack_i18n::tr!("Valeur RVB linéaire"))
            .changed();
    }
    if changed {
        *c = clamp_rgb(rgb, bounds);
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use syfrack_core::Bounds;

    #[test]
    fn linear_and_log_mappings_roundtrip_and_clamp() {
        let lin = Bounds::new(-10.0, 10.0, &[]);
        for v in [-10.0, -3.3, 0.0, 7.5, 10.0] {
            assert!((slider_unmap(slider_map(v, lin, false), lin, false) - v).abs() < 1e-4);
        }
        assert_eq!(slider_map(0.0, lin, false), 0.5);
        let log = Bounds::new(0.001, 1000.0, &[]);
        assert!(
            (slider_map(1.0, log, true) - 0.5).abs() < 1e-6,
            "1.0 is the geometric middle of 0.001..1000"
        );
        for v in [0.001, 0.02, 1.0, 55.0, 1000.0] {
            assert!((slider_unmap(slider_map(v, log, true), log, true) / v - 1.0).abs() < 1e-4);
        }
        assert_eq!(slider_unmap(-0.5, lin, false), -10.0);
        assert_eq!(slider_unmap(1.5, log, true), 1000.0);
        let zero_min = Bounds::new(0.0, 100.0, &[]);
        assert!(
            slider_map(0.0, zero_min, true).is_finite(),
            "log rails floor the minimum at 1e-3 of the max"
        );
    }

    #[test]
    fn direction_angles_follow_the_orbit_convention() {
        let close =
            |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-4 && (a.1 - b.1).abs() < 1e-4;
        assert!(
            close(direction_angles(Vec3::Z), (0.0, 0.0)),
            "+z: azimuth 0, elevation 0"
        );
        assert!(
            close(direction_angles(Vec3::X), (90.0, 0.0)),
            "+x: azimuth 90"
        );
        assert!(
            (direction_angles(Vec3::Y).1 - 90.0).abs() < 1e-4,
            "+y: elevation 90"
        );
        assert!(
            close(direction_angles(Vec3::new(0.0, 0.0, -3.0)), (180.0, 0.0)),
            "length does not matter"
        );
    }

    #[test]
    fn developed_is_the_cpu_develop_of_one_pixel_without_image_wide_effects() {
        let mut look = Look::neutral();
        look.exposure_ev = 0.7;
        look.temperature = 4800.0;
        look.saturation = 1.2;
        let c = [0.6, 0.35, 0.2];
        let mut img = HdrImage::new(1, 1);
        img.set(0, 0, [c[0], c[1], c[2], 1.0]);
        let want = syfrack_color::develop(&img, &look).get(0, 0);
        let mut with_image_wide = look.clone();
        with_image_wide.auto_exposure = Some(0.18);
        with_image_wide.vignette = Some(syfrack_core::Vignette {
            radius: 0.1,
            softness: 0.1,
            intensity: 1.0,
            elliptical: false,
        });
        with_image_wide.grain = Some(syfrack_core::Grain {
            amount: 0.5,
            size: 1.0,
            seed: 3,
        });
        for l in [&look, &with_image_wide] {
            assert_eq!(developed(c, l), [want[0], want[1], want[2]]);
        }
    }

    #[test]
    fn picked_colours_are_clamped_to_the_parameter_bounds() {
        let albedo = Bounds::new(0.0, 0.95, &[]);
        assert_eq!(clamp_rgb([1.2, -0.1, 0.5], albedo), [0.95, 0.0, 0.5]);
    }

    #[test]
    fn values_are_written_and_read_with_a_french_decimal_comma() {
        syfrack_i18n::with_language(syfrack_i18n::Language::French, || {
            assert_eq!(format_value(0.025, 3), "0,025");
            assert_eq!(format_value(35.0, 0), "35");
            assert_eq!(format_value(-1.5, 2), "-1,50");
            assert_eq!(format_value(-0.001, 2), "0,00", "no negative zero");
        });
        syfrack_i18n::with_language(syfrack_i18n::Language::English, || {
            assert_eq!(format_value(0.025, 3), "0.025");
            assert_eq!(format_value(-1.5, 2), "-1.50");
            assert_eq!(format_value(-0.001, 2), "0.00");
        });
        assert_eq!(parse_value("0,5"), Some(0.5));
        assert_eq!(parse_value(" 4.25 "), Some(4.25));
        assert_eq!(parse_value("abc"), None);
        assert_eq!(parse_value("inf"), None, "only finite values");
    }
}
