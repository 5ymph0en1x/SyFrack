//! The six scene panels, in the order of their visual impact. Each one reads
//! the scene and writes back only through `AppState::edit` / `set_param`.

pub mod atmosphere;
pub mod camera;
pub mod fractal;
pub mod lights;
pub mod look;
pub mod material;

use std::borrow::Cow;
use std::sync::OnceLock;

use egui::{Align, Align2, Layout, Pos2, Sense, Ui, Vec2};
use glam::Vec3;
use syfrack_core::{
    Bounds, Color, Grain, Graph, Light, ParamInfo, Params, Scene, Tonemap, Vignette,
};

use crate::state::AppState;
use crate::ui::theme;
use crate::ui::widgets::{label_width, linear_color, temperature_slider, ParamSlider, ROW_H};

/// Mid-grey target set when automatic exposure is switched on.
pub const AUTO_EXPOSURE_TARGET: f32 = 0.18;

/// The light added by « + Soleil ».
pub fn new_sun() -> Light {
    Light::Sun {
        direction: Vec3::new(0.3, 0.8, 0.4).normalize(),
        color: Color::Kelvin(5500.0),
        intensity: 4.0,
        angular_size: 1.0,
    }
}

/// An area light at `position` emitting along `normal`.
pub fn new_area(position: Vec3, normal: Vec3) -> Light {
    Light::Area {
        position,
        normal: normal.normalize_or(Vec3::NEG_Y),
        size: [1.0, 1.0],
        color: Color::Kelvin(9000.0),
        intensity: 6.0,
    }
}

/// The vignette set when « Vignettage » is switched on.
pub fn new_vignette() -> Vignette {
    Vignette {
        radius: 0.75,
        softness: 0.9,
        intensity: 0.25,
        elliptical: true,
    }
}

/// The grain set when « Grain » is switched on.
pub fn new_grain() -> Grain {
    Grain {
        amount: 0.03,
        size: 1.0,
        seed: 11,
    }
}

/// The curve set when « Reinhard » is chosen.
pub fn reinhard() -> Tonemap {
    Tonemap::Reinhard { white: 4.0 }
}

/// Path segment naming a light's kind, as the parameter paths spell it.
pub fn light_kind(light: &Light) -> &'static str {
    match light {
        Light::Sun { .. } => "sun",
        Light::Area { .. } => "area",
        Light::Projector { .. } => "area",
        Light::Tube { .. } => "tube",
        Light::Environment { .. } => "environment",
        Light::Hdri { .. } => "hdri",
    }
}

/// The scenes the reference ticks are read from: `Scene::example()`,
/// completed by the templates of the add buttons and toggles so every
/// optional member has a reference too.
fn reference_scenes() -> [Scene; 2] {
    let mut a = Scene::example();
    a.lights.push(new_area(Vec3::ZERO, Vec3::NEG_Y));
    a.look.auto_exposure = Some(AUTO_EXPOSURE_TARGET);
    a.look.vignette = Some(new_vignette());
    a.look.grain = Some(new_grain());
    a.material.texture = Some(syfrack_core::FractalTexture::default());
    a.material.surface = Some(syfrack_core::SurfaceVariation::default());
    a.material.texture.as_mut().unwrap().fan = Some(syfrack_core::FanMapping::default());
    a.material.texture.as_mut().unwrap().mask = Some(syfrack_core::TextureMask::default());
    a.atmosphere.seascape = Some(syfrack_core::Seascape::default());
    a.atmosphere.seascape.as_mut().unwrap().surface = Some(syfrack_core::OceanSurface::default());
    let mut b = a.clone();
    b.look.tonemap = reinhard();
    [a, b]
}

/// What the reference scenes offer.
struct Reference {
    /// Kind of each reference light, by index.
    lights: Vec<&'static str>,
    params: Vec<ParamInfo>,
}

/// The references, built once.
fn reference() -> &'static Reference {
    static REFERENCE: OnceLock<Reference> = OnceLock::new();
    REFERENCE.get_or_init(|| {
        let scenes = reference_scenes();
        Reference {
            lights: scenes[0].lights.iter().map(light_kind).collect(),
            params: scenes
                .iter()
                .flat_map(|s| s.params())
                .filter(|p| !p.path.starts_with("fractal."))
                .collect(),
        }
    })
}

/// The `fractal.*` parameters of every bundled graph, by shader fingerprint.
fn reference_form_fingerprint(form: &Graph) -> u64 {
    // The reference tick describes the form, irrespective of the specialised
    // Power6/8 shader family selected by its current scalar value.
    let mut reference = form.clone();
    for node in &mut reference.nodes {
        if let syfrack_core::NodeKind::Power { power } = &mut node.kind {
            *power = 8.0;
        }
    }
    syfrack_render::graph_wgsl::fingerprint(&reference)
}

fn bundled_form_params() -> &'static [(u64, Vec<ParamInfo>)] {
    static TABLE: OnceLock<Vec<(u64, Vec<ParamInfo>)>> = OnceLock::new();
    TABLE.get_or_init(|| {
        syfrack_core::scene::graph::bundled::BUNDLED
            .iter()
            .filter_map(|(name, _)| Graph::bundled(name))
            .map(|mut g| {
                let fp = reference_form_fingerprint(&g);
                let mut params = Vec::new();
                g.visit_params("fractal", &mut |path, v, bounds| {
                    params.push(ParamInfo {
                        path: path.to_string(),
                        value: *v,
                        bounds,
                    });
                });
                (fp, params)
            })
            .collect()
    })
}

/// Rewrites `lights[i].<kind>...` to the index of the first light of that
/// kind in `lights`; other paths are returned unchanged.
fn light_path<'a>(lights: &[&str], path: &'a str) -> Option<Cow<'a, str>> {
    let Some(rest) = path.strip_prefix("lights[") else {
        return Some(Cow::Borrowed(path));
    };
    let (_, rest) = rest.split_once(']')?;
    let kind = rest.trim_start_matches('.').split('.').next()?;
    let i = lights.iter().position(|&k| k == kind)?;
    Some(Cow::Owned(format!("lights[{i}]{rest}")))
}

/// The value of `path` in the example scene: the fine tick on a rail.
/// Lights are matched by kind, not by index. A `fractal.*` path takes its
/// tick from the bundled graph of the same topology as `form`, if any.
pub fn reference_value(form: &Graph, path: &str) -> Option<f32> {
    reference_info(form, path).map(|p| p.value)
}

/// The parameter at `path` in the reference scenes, lights matched by kind.
fn reference_info(form: &Graph, path: &str) -> Option<ParamInfo> {
    if path.starts_with("fractal.") {
        let fp = reference_form_fingerprint(form);
        return bundled_form_params()
            .iter()
            .find(|(f, _)| *f == fp)
            .and_then(|(_, params)| params.iter().find(|p| p.path == path).cloned());
    }
    let r = reference();
    let path = light_path(&r.lights, path)?;
    r.params.iter().find(|p| p.path == path).cloned()
}

/// One bounded parameter, drawn by path: bounds from the scene's parameter
/// list, reference tick from the example scene.
pub struct Param<'a> {
    path: &'a str,
    label: &'a str,
    unit: &'a str,
    log: bool,
    decimals: usize,
    reference: bool,
    enabled: bool,
    topology: bool,
    default: Option<f32>,
}

pub fn param<'a>(path: &'a str, label: &'a str) -> Param<'a> {
    Param {
        path,
        label,
        unit: "",
        log: false,
        decimals: 2,
        reference: true,
        enabled: true,
        topology: false,
        default: None,
    }
}

impl<'a> Param<'a> {
    pub fn unit(mut self, unit: &'a str) -> Self {
        self.unit = unit;
        self
    }

    /// Logarithmic rail: distances, intensities, densities, bailout.
    pub fn log(mut self) -> Self {
        self.log = true;
        self
    }

    pub fn decimals(mut self, d: usize) -> Self {
        self.decimals = d;
        self
    }

    /// No reference tick: positions and directions have no meaningful one.
    pub fn no_reference(mut self) -> Self {
        self.reference = false;
        self
    }

    /// A form-graph integer whose change rebuilds the shader: « ⟳ » before
    /// the label, whole values.
    pub fn topology(mut self) -> Self {
        self.topology = true;
        self
    }

    /// The reference tick when the example scenes have none (form nodes).
    pub fn default_value(mut self, v: Option<f32>) -> Self {
        self.default = v;
        self
    }

    /// Shown but inactive when false. A member absent from the scene is then
    /// drawn greyed at its reference value, so the row does not vanish.
    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }

    pub fn show(self, ui: &mut Ui, state: &mut AppState) {
        let form = &state.scene.fractal.to_graph();
        let info = match find_param(&state.scene, self.path) {
            Some(info) => info,
            None if !self.enabled => match reference_info(form, self.path) {
                Some(info) => info,
                None => return,
            },
            None => return,
        };
        let mut slider = ParamSlider::new(self.label, info.bounds)
            .unit(self.unit)
            .decimals(self.decimals);
        if self.log {
            slider = slider.log();
        }
        if self.topology {
            slider = slider.topology();
        }
        if let Some(d) = self
            .reference
            .then(|| reference_value(form, self.path).or(self.default))
            .flatten()
        {
            slider = slider.default_value(d);
        }
        let mut v = info.value;
        let r = ui
            .add_enabled_ui(self.enabled, |ui| slider.show(ui, &mut v))
            .inner;
        #[cfg(test)]
        ui.data_mut(|d| d.insert_temp(egui::Id::new(("param-row", self.path)), r.rect));
        if self.enabled && r.changed() {
            state.set_param(self.path, v);
        }
    }
}

fn find_param(scene: &Scene, path: &str) -> Option<ParamInfo> {
    scene.params().into_iter().find(|p| p.path == path)
}

/// The range of the channels of the RGB parameter at `prefix`.
pub fn rgb_bounds(scene: &Scene, prefix: &str) -> Bounds {
    bounds_of(scene, &format!("{prefix}.r")).unwrap_or(Bounds::new(0.0, 1.0, &[]))
}

/// The valid range of a parameter of `scene`.
pub fn bounds_of(scene: &Scene, path: &str) -> Option<Bounds> {
    find_param(scene, path).map(|p| p.bounds)
}

/// A Kelvin parameter on the black-body rail, bounds from the scene.
pub fn temperature(ui: &mut Ui, state: &mut AppState, path: &str, label: &str) {
    let Some(info) = find_param(&state.scene, path) else {
        return;
    };
    let mut k = info.value;
    if temperature_slider(ui, label, &mut k, info.bounds).changed() {
        state.set_param(path, k);
    }
}

/// A light colour: the black-body rail for Kelvin, the developed swatch and
/// picker for linear RGB. `prefix` is the colour's parameter path.
pub fn color_row(
    ui: &mut Ui,
    state: &mut AppState,
    prefix: &str,
    label: &str,
    color: &Color,
    set: impl FnOnce(&mut Scene, Color),
) {
    match color {
        Color::Kelvin(_) => temperature(ui, state, &format!("{prefix}.kelvin"), label),
        Color::Linear(c) => {
            let mut c = *c;
            let bounds = rgb_bounds(&state.scene, prefix);
            if linear_color(ui, label, &mut c, bounds, &state.scene.look) {
                state.edit(|s| set(s, Color::Linear(c)));
            }
        }
    }
}

/// A row with the label in the slider label column and `add` laid out left
/// to right where the rail would be, so every control lines up.
pub fn labeled(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    let width = label_width(ui.available_width());
    ui.horizontal(|ui| {
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(width - ui.spacing().item_spacing.x, ROW_H),
            Sense::hover(),
        );
        ui.painter_at(rect).text(
            Pos2::new(rect.left(), rect.center().y),
            Align2::LEFT_CENTER,
            label,
            egui::TextStyle::Body.resolve(ui.style()),
            theme::TEXT,
        );
        response.on_hover_text(label);
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), ROW_H),
            Layout::left_to_right(Align::Center),
            add,
        );
    });
}

/// A checkbox in a labelled row. Returns the new value when it was toggled.
pub fn toggle(ui: &mut Ui, label: &str, on: bool) -> Option<bool> {
    let mut v = on;
    let r = ui.checkbox(&mut v, label);
    #[cfg(test)]
    ui.data_mut(|d| d.insert_temp(egui::Id::new(("option-toggle", label)), r.rect));
    r.changed().then_some(v)
}

/// Two or more exclusive choices in a labelled row. Returns the clicked one
/// when it differs from `current`.
pub fn choice(ui: &mut Ui, label: &str, options: &[&str], current: usize) -> Option<usize> {
    let translated: Vec<&str> = options.iter().map(|s| syfrack_i18n::text(s)).collect();
    let options = translated.as_slice();
    let mut chosen = None;
    labeled(ui, label, |ui| {
        let font = egui::TextStyle::Button.resolve(ui.style());
        let needed: f32 = options
            .iter()
            .map(|text| {
                ui.fonts_mut(|f| {
                    f.layout_no_wrap((*text).into(), font.clone(), theme::TEXT)
                        .size()
                        .x
                }) + 2.0 * ui.spacing().button_padding.x
                    + ui.spacing().item_spacing.x
            })
            .sum();
        if needed > ui.available_width() {
            egui::ComboBox::from_id_salt(("exclusive-choice", label))
                .width(ui.available_width())
                .selected_text(options.get(current).copied().unwrap_or("—"))
                .show_ui(ui, |ui| {
                    for (i, name) in options.iter().enumerate() {
                        if crate::ui::widgets::menu_choice(ui, i == current, name).clicked()
                            && i != current
                        {
                            chosen = Some(i);
                        }
                    }
                });
            return;
        }
        for (i, name) in options.iter().enumerate() {
            let selected = i == current;
            let text = egui::RichText::new(*name).color(if selected {
                theme::ACCENT
            } else {
                theme::TEXT_DIM
            });
            let r = crate::ui::widgets::choice_button(ui, selected, text);
            #[cfg(test)]
            ui.data_mut(|d| d.insert_temp(egui::Id::new(("choice", label, *name)), r.rect));
            if r.clicked() && !selected {
                chosen = Some(i);
            }
        }
    });
    chosen
}

/// A thin rule between two groups of a panel.
pub fn rule(ui: &mut Ui) {
    ui.add_space(theme::GRID / 2.0);
    ui.separator();
    ui.add_space(theme::GRID / 2.0);
}

/// The fold holding the rarely touched settings of a group, closed by default.
pub fn advanced(ui: &mut Ui, salt: &str, body: impl FnOnce(&mut Ui)) {
    egui::CollapsingHeader::new(
        egui::RichText::new(syfrack_i18n::tr!("Avancé")).color(theme::TEXT_DIM),
    )
    .id_salt(("advanced", salt))
    .default_open(false)
    .show(ui, body);
}

fn section(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    egui::CollapsingHeader::new(egui::RichText::new(title).heading())
        .default_open(true)
        .show(ui, body);
    ui.add_space(theme::GRID);
}

pub fn column(ui: &mut Ui, body: impl FnOnce(&mut Ui)) {
    ui.spacing_mut().indent = theme::GRID;
    ui.spacing_mut().item_spacing.y = 6.0;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Keep the unit column clear of the overlaid scroll bar.
            egui::Frame::NONE
                .inner_margin(egui::Margin {
                    right: (1.5 * theme::GRID) as i8,
                    ..Default::default()
                })
                .show(ui, body);
        });
}

/// Left column: what is seen (the form) and from where (camera).
/// `library` names the form when it matches one of its entries.
pub fn left(ui: &mut Ui, state: &mut AppState, library: &[crate::library::LibraryEntry]) {
    column(ui, |ui| {
        section(ui, syfrack_i18n::tr!("Forme"), |ui| {
            fractal::show(ui, state, library)
        });
        section(ui, syfrack_i18n::tr!("Caméra"), |ui| {
            camera::show(ui, state)
        });
    });
}

/// Right column: light, surface, air, then the development of the image.
pub fn right(ui: &mut Ui, state: &mut AppState) {
    column(ui, |ui| {
        section(ui, syfrack_i18n::tr!("Lumières"), |ui| {
            lights::show(ui, state)
        });
        section(ui, syfrack_i18n::tr!("Matière"), |ui| {
            material::show(ui, state)
        });
        section(ui, syfrack_i18n::tr!("Atmosphère"), |ui| {
            atmosphere::show(ui, state)
        });
        section(ui, "Look", |ui| look::show(ui, state));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use syfrack_core::{NodeKind, Scene};

    fn sphere() -> Graph {
        Graph::bundled("sphere").unwrap()
    }

    #[test]
    fn reference_values_come_from_the_example_scenes() {
        let sphere = &sphere();
        let example = Scene::example();
        assert_eq!(
            reference_value(sphere, "camera.focal_mm"),
            Some(example.camera.focal_mm)
        );
        assert_eq!(reference_value(sphere, "look.exposure_ev"), Some(0.0));
        assert_eq!(reference_value(sphere, "no.such.path"), None);
    }

    #[test]
    fn form_ticks_come_from_the_bundled_graph_of_the_same_topology() {
        // A Mandelbulb with another power and safety: same topology, the
        // ticks are the bundled Mandelbulb's values.
        let mut bulb = Graph::bundled("mandelbulb").unwrap();
        bulb.safety = 0.4;
        for n in &mut bulb.nodes {
            if let NodeKind::Power { power } = &mut n.kind {
                *power = 5.0;
            }
        }
        assert_eq!(reference_value(&bulb, "fractal.nodes[3].power"), Some(8.0));
        assert_eq!(reference_value(&bulb, "fractal.safety"), Some(0.7));
        assert_eq!(reference_value(&sphere(), "fractal.safety"), Some(1.0));
        assert_eq!(reference_value(&sphere(), "fractal.nodes[3].power"), None);
        // A topology no bundled graph has: no tick.
        bulb.nodes[2].enabled = false;
        assert_eq!(reference_value(&bulb, "fractal.safety"), None);
    }

    #[test]
    fn light_references_follow_the_light_kind_not_its_index() {
        let k = &sphere();
        assert_eq!(reference_value(k, "lights[3].sun.intensity"), Some(6.0));
        assert_eq!(
            reference_value(k, "lights[0].environment.intensity"),
            Some(0.7)
        );
        let Light::Area { intensity, .. } = new_area(Vec3::ZERO, Vec3::NEG_Y) else {
            panic!("new_area is an area light");
        };
        assert_eq!(
            reference_value(k, "lights[1].area.intensity"),
            Some(intensity),
            "a kind absent from the example uses the template of its add button"
        );
    }

    #[test]
    fn optional_look_members_use_the_templates_of_their_toggles() {
        let k = &sphere();
        assert_eq!(
            reference_value(k, "look.vignette.intensity"),
            Some(new_vignette().intensity)
        );
        assert_eq!(
            reference_value(k, "look.grain.amount"),
            Some(new_grain().amount)
        );
        assert_eq!(
            reference_value(k, "look.auto_exposure"),
            Some(AUTO_EXPOSURE_TARGET)
        );
        let Tonemap::Reinhard { white } = reinhard() else {
            panic!("reinhard() is Reinhard");
        };
        assert_eq!(
            reference_value(k, "look.tonemap.reinhard.white"),
            Some(white)
        );
    }
}
