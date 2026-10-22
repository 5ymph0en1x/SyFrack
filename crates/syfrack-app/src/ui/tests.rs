//! Harness tests of the studio UI: whole frames driven by synthetic input.

use super::*;
use egui::{Event, Id, Pos2, RawInput, Rect};
use syfrack_core::Scene;

const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(1600.0, 900.0));

struct Harness {
    ctx: egui::Context,
    state: AppState,
    time: f64,
    screen: Rect,
    view_available: (u32, u32),
    /// What a frame draws: the whole studio unless a test narrows it.
    body: fn(&mut egui::Ui, &mut AppState, &mut Viewport, &mut GalleryPanel, &mut FormPanel),
    gallery: GalleryPanel,
    form: FormPanel,
    language: syfrack_i18n::Language,
    labels: Vec<String>,
}

impl Harness {
    fn new() -> Self {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let state = AppState::new(Scene::example_for("mandelbox").unwrap(), None);
        let mut h = Self {
            ctx,
            state,
            time: 0.0,
            screen: SCREEN,
            view_available: (0, 0),
            body: draw,
            gallery: GalleryPanel::with_dir("Z:/no/such/dir"),
            form: FormPanel::new(),
            language: syfrack_i18n::Language::French,
            labels: Vec::new(),
        };
        h.frame(vec![]);
        h.frame(vec![]);
        h
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.frame_with(events, Modifiers::NONE);
    }

    fn frame_with(&mut self, events: Vec<Event>, modifiers: Modifiers) {
        self.time += 1.0 / 60.0;
        let mut all = vec![Event::ModifiersChanged(modifiers)];
        all.extend(events);
        let raw = RawInput {
            screen_rect: Some(self.screen),
            time: Some(self.time),
            events: all,
            ..Default::default()
        };
        let mut view = Viewport {
            texture: None,
            size: (0, 0),
            caption: String::new(),
            available_px: (0, 0),
            depth_hint: 1.0,
            pick: None,
            frame_guide: None,
            output: Default::default(),
        };
        let state = &mut self.state;
        let body = self.body;
        let gallery = &mut self.gallery;
        let form = &mut self.form;
        let mut out = self.ctx.run_ui(raw, |ui| {
            syfrack_i18n::with_language(self.language, || body(ui, state, &mut view, gallery, form))
        });
        self.labels.clear();
        fn collect(shape: &egui::epaint::Shape, out: &mut Vec<String>) {
            match shape {
                egui::epaint::Shape::Text(text) => out.push(text.galley.text().to_owned()),
                egui::epaint::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
                _ => {}
            }
        }
        for shape in &out.shapes {
            collect(&shape.shape, &mut self.labels);
        }
        // epaint panics if texture deltas are dropped unapplied.
        out.textures_delta.clear();
        self.view_available = view.available_px;
    }

    fn button(&mut self, pos: Pos2, pressed: bool) {
        self.frame(vec![
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            },
        ]);
    }

    fn drag(&mut self, from: Pos2, to: Pos2) {
        self.frame(vec![Event::PointerMoved(from)]);
        self.button(from, true);
        for k in 1..=6 {
            let p = from + (to - from) * (k as f32 / 6.0);
            self.frame(vec![Event::PointerMoved(p)]);
        }
        self.button(to, false);
        self.frame(vec![]);
    }

    fn key(&mut self, key: Key, event_mods: Modifiers, state_mods: Modifiers) {
        let ev = |pressed| Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers: event_mods,
        };
        self.frame_with(vec![ev(true)], state_mods);
        self.frame_with(vec![ev(false)], state_mods);
    }
}

#[test]
fn both_workshops_switch_language_without_changing_the_scene_or_history() {
    use syfrack_i18n::Language::{English, French};
    let mut h = Harness::new();
    for dimension in [
        syfrack_core::Dimension::ThreeD,
        syfrack_core::Dimension::TwoD,
    ] {
        h.state.switch_dimension(dimension);
        let before = h.state.scene.clone();
        let shader = syfrack_render::graph_wgsl::compile(before.fractal.graph())
            .unwrap()
            .wgsl;
        let undo = h.state.can_undo();
        for (language, file, render) in [
            (English, "File", "Render / Export"),
            (French, "Fichier", "Rendre / Exporter"),
        ] {
            h.language = language;
            h.frame(vec![]);
            h.frame(vec![]);
            assert!(
                h.labels.iter().any(|s| s == file),
                "{language:?}: {:?}",
                h.labels
            );
            assert!(h.labels.iter().any(|s| s == render), "{language:?}");
            assert_eq!(h.state.scene, before);
            assert_eq!(h.state.can_undo(), undo);
            syfrack_i18n::with_language(language, || {
                assert_eq!(
                    syfrack_render::graph_wgsl::compile(h.state.scene.fractal.graph())
                        .unwrap()
                        .wgsl,
                    shader
                );
            });
        }
    }
}

/// Windows: egui-winit sets both `ctrl` and `command` while Ctrl is held.
const CTRL: Modifiers = Modifiers {
    alt: false,
    ctrl: true,
    shift: false,
    mac_cmd: false,
    command: true,
};

#[test]
fn interface_switch_is_reversible_and_preserves_document_and_redo() {
    use crate::interface::InterfaceMode;
    let mut h = Harness::new();
    assert_eq!(h.state.interface, InterfaceMode::Advanced);
    h.state.path = Some("scenes/same-project.ron".into());
    h.state.set_param("look.exposure_ev", 1.0);
    h.state.set_param("look.saturation", 0.5);
    h.state.undo();
    h.state.form.selected = vec![2];
    let before = h.state.scene.clone();
    let path = h.state.path.clone();
    let modified = h.state.is_modified();
    h.state.take_preview_dirty();
    for mode in [
        InterfaceMode::Simple,
        InterfaceMode::Advanced,
        InterfaceMode::Simple,
    ] {
        let at = control(&h, Id::new(("interface-mode", mode))).center();
        click(&mut h, at);
        assert_eq!(h.state.interface, mode);
        assert_eq!(h.state.scene, before);
        assert_eq!(h.state.path, path);
        assert_eq!(h.state.form.selected, vec![2]);
        assert_eq!(h.state.is_modified(), modified);
        assert!(h.state.can_redo());
        assert!(!h.state.take_preview_dirty());
    }
    assert!(h.state.redo());
    assert_eq!(h.state.scene.look.saturation, 0.5);
}

#[test]
fn both_interfaces_keep_navigation_and_export_visible_in_both_languages() {
    use crate::interface::InterfaceMode;
    let mut h = Harness::new();
    h.state.path = Some(format!("scenes/{}.ron", "long-title-".repeat(40)).into());
    let before = h.state.scene.clone();
    for language in syfrack_i18n::Language::ALL {
        h.language = language;
        for mode in InterfaceMode::ALL {
            h.state.set_interface(mode);
            for width in [700.0, 800.0, 1000.0, 1280.0, 1600.0, 1920.0] {
                h.screen = Rect::from_min_size(Pos2::ZERO, egui::vec2(width, 900.0));
                h.frame(vec![]);
                h.frame(vec![]);
                for id in [
                    Id::new("render-open-button"),
                    Id::new("display-menu"),
                    Id::new(("interface-mode", InterfaceMode::Simple)),
                    Id::new(("interface-mode", InterfaceMode::Advanced)),
                    Id::new(("dimension", "2D")),
                    Id::new(("dimension", "3D")),
                ] {
                    let rect = control(&h, id);
                    assert!(
                        h.screen.contains_rect(rect),
                        "{language:?}/{mode:?}/{width}: {rect:?}"
                    );
                }
                let render = control(&h, Id::new("render-open-button"));
                assert!(!render.intersects(control(&h, Id::new("document-title"))));
                assert!(h.view_available.0 >= 200 && h.view_available.1 >= 400);
            }
        }
    }
    assert_eq!(h.state.scene, before);
    assert!(!h.state.can_undo());
}

#[test]
fn simple_pattern_and_palette_edit_the_connected_graph_and_undo() {
    use crate::interface::InterfaceMode;
    use syfrack_core::CreationKind;
    let mut h = Harness::new();
    assert!(h.state.load_scene(Scene::example_2d(), None));
    h.state.set_interface(InterfaceMode::Simple);
    h.frame(vec![]);
    h.frame(vec![]);
    let before = h.state.scene.clone();
    let at = control(&h, Id::new(("simple-family", "Mandelbrot"))).center();
    click(&mut h, at);
    let graph = h.state.scene.creation.as_ref().unwrap();
    assert_eq!(graph.links, before.creation.as_ref().unwrap().links);
    assert!(graph
        .nodes
        .iter()
        .any(|n| matches!(&n.kind, CreationKind::Texture(t)
        if t.planar.as_ref().is_some_and(|p| p.family == syfrack_core::PlanarFamily::Mandelbrot))));
    assert_eq!(h.state.scene, h.state.scene.render_scene().unwrap());
    assert!(h.state.take_preview_dirty());
    assert!(h.state.undo());
    assert_eq!(h.state.scene, before);
    h.state.inspector = crate::state::Inspector::Material;
    h.frame(vec![]);
    h.frame(vec![]);
    let at = control(&h, Id::new(("simple-palette", 2usize))).center();
    click(&mut h, at);
    assert_ne!(h.state.scene.creation, before.creation);
    assert_eq!(
        h.state.scene.creation.as_ref().unwrap().links,
        before.creation.as_ref().unwrap().links
    );
    assert_eq!(h.state.scene, h.state.scene.render_scene().unwrap());
    let coloured = h.state.scene.clone();
    let palette_id = coloured
        .creation
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find(|n| matches!(n.kind, CreationKind::Palette(_)))
        .unwrap()
        .id;
    let frequency = format!("creation.nodes[{palette_id}].palette.frequency");
    let row = control(&h, Id::new(("creative-param", frequency.as_str())));
    let left = row.left() + widgets::label_width(row.width());
    let right = row.right() - widgets::VALUE_W - theme::GRID;
    h.drag(
        egui::pos2(left + (right - left) * 0.2, row.center().y),
        egui::pos2(left + (right - left) * 0.8, row.center().y),
    );
    assert_ne!(h.state.scene.creation, coloured.creation);
    assert_eq!(h.state.scene, h.state.scene.render_scene().unwrap());
    h.state.set_interface(InterfaceMode::Advanced);
    assert!(h.state.undo());
    assert_eq!(h.state.scene, coloured);
    assert!(h.state.undo());
    assert_eq!(h.state.scene, before);
}

#[test]
fn simple_volume_slider_updates_the_authoritative_form() {
    use crate::interface::InterfaceMode;
    let mut h = Harness::new();
    h.state.set_interface(InterfaceMode::Simple);
    h.frame(vec![]);
    h.frame(vec![]);
    let before = h.state.scene.clone();
    let path = "creation.nodes[2].limit";
    let row = control(&h, Id::new(("creative-param", path)));
    let left = row.left() + widgets::label_width(row.width());
    let right = row.right() - widgets::VALUE_W - theme::GRID;
    h.drag(
        egui::pos2(left + (right - left) * 0.3, row.center().y),
        egui::pos2(left + (right - left) * 0.9, row.center().y),
    );
    assert_ne!(h.state.scene.creation, before.creation);
    assert_eq!(h.state.scene, h.state.scene.render_scene().unwrap());
    assert!(h.state.undo());
    assert_eq!(h.state.scene, before);
}

#[test]
fn simple_can_start_a_2d_pattern_from_a_3d_scene_without_losing_geometry() {
    use crate::interface::InterfaceMode;
    let mut h = Harness::new();
    h.state.set_interface(InterfaceMode::Simple);
    h.frame(vec![]);
    h.frame(vec![]);
    let before = h.state.scene.clone();
    let at = control(&h, Id::new(("dimension", "2D"))).center();
    click(&mut h, at);
    assert_eq!(h.state.scene.dimension, syfrack_core::Dimension::TwoD);
    assert_ne!(h.state.scene.creation, before.creation);
    assert_eq!(h.state.scene.fractal, before.fractal);
    assert!(h.state.scene.creation_draft.is_none());
    assert_eq!(h.state.scene, h.state.scene.render_scene().unwrap());
    assert!(h.state.undo());
    assert_eq!(h.state.scene, before);
}

#[test]
fn simple_preserves_unfinished_graphs_and_offers_the_advanced_editor() {
    use crate::interface::InterfaceMode;
    let mut h = Harness::new();
    let mut draft = h.state.scene.creation.clone().unwrap();
    draft.links.clear();
    h.state.scene.creation_draft = Some(draft);
    h.state.set_interface(InterfaceMode::Simple);
    let before = h.state.scene.clone();
    h.frame(vec![]);
    h.frame(vec![]);
    let at = control(&h, Id::new("simple-open-advanced")).center();
    click(&mut h, at);
    assert_eq!(h.state.interface, InterfaceMode::Advanced);
    assert_eq!(h.state.tab, Tab::Forme);
    assert_eq!(h.state.scene, before);
    assert!(!h.state.can_undo());
}

#[test]
fn simple_tools_reach_mosaic_and_the_complete_graph_without_changing_the_scene() {
    use crate::interface::InterfaceMode;
    let mut h = Harness::new();
    h.state.set_interface(InterfaceMode::Simple);
    let before = h.state.scene.clone();
    h.frame(vec![]);
    h.frame(vec![]);
    let tools = control(&h, Id::new("simple-tools")).center();
    click(&mut h, tools);
    let mosaic = control(&h, Id::new("simple-mosaic")).center();
    click(&mut h, mosaic);
    assert_eq!(h.state.tab, Tab::Mosaic);
    assert_eq!(h.state.interface, InterfaceMode::Simple);
    let tools = control(&h, Id::new("simple-tools")).center();
    click(&mut h, tools);
    let graph = control(&h, Id::new("simple-graph")).center();
    click(&mut h, graph);
    assert_eq!(h.state.tab, Tab::Forme);
    assert_eq!(h.state.interface, InterfaceMode::Advanced);
    assert_eq!(h.state.scene, before);
    assert!(!h.state.can_undo());
    h.state.set_interface(InterfaceMode::Simple);
    assert_eq!(h.state.tab, Tab::Atelier);
}

#[test]
fn mosaic_tab_is_accessible_in_both_languages_and_does_not_undo_the_scene() {
    let mut h = Harness::new();
    h.state.edit(|s| s.seed = s.seed.wrapping_add(1));
    let before = h.state.scene.clone();
    let tab = control(&h, Id::new(("workspace-tab", "MOSAÏQUE"))).center();
    click(&mut h, tab);
    assert_eq!(h.state.tab, Tab::Mosaic);
    for (language, label) in [
        (syfrack_i18n::Language::French, "MOSAÏQUE"),
        (syfrack_i18n::Language::English, "MOSAIC"),
    ] {
        h.language = language;
        for size in [
            egui::vec2(1600.0, 900.0),
            egui::vec2(1000.0, 640.0),
            egui::vec2(760.0, 660.0),
        ] {
            h.screen = Rect::from_min_size(Pos2::ZERO, size);
            h.frame(vec![]);
            h.frame(vec![]);
            assert!(h.labels.iter().any(|s| s == label));
            for action in ["open", "generate", "export"] {
                let rect = control(&h, Id::new(("mosaic-button", action)));
                assert!(
                    h.screen.contains_rect(rect),
                    "{action} outside {size:?}: {rect:?}"
                );
            }
            let image = control(&h, Id::new("mosaic-image-area"));
            assert!(image.width() > 200.0 && image.height() > 200.0);
        }
    }
    h.key(Key::Z, CTRL, Modifiers::NONE);
    assert_eq!(h.state.scene, before);
    assert!(h.state.can_undo());
    h.key(Key::O, CTRL, Modifiers::NONE);
    let dialog = h
        .ctx
        .data(|d| d.get_temp::<FileDialog>(dialog_id()))
        .unwrap();
    assert!(matches!(dialog.action, FileAction::Image(_)));
    assert_eq!(h.state.scene, before);
}

#[test]
fn inspector_navigation_and_view_options_preserve_the_scene() {
    let mut h = Harness::new();
    let before = h.state.scene.clone();
    for &page in workshops::pages(syfrack_core::Dimension::ThreeD) {
        let at = control(
            &h,
            egui::Id::new((
                "inspector-page",
                syfrack_i18n::with_language(h.language, || page.label()),
            )),
        )
        .center();
        click(&mut h, at);
        assert_eq!(h.state.inspector, page);
    }
    h.key(Key::H, Modifiers::NONE, Modifiers::NONE);
    assert!(!h.state.show_gizmos);
    h.key(Key::Tab, Modifiers::NONE, Modifiers::NONE);
    assert!(h.state.image_only);
    assert_eq!(h.state.scene, before);
    assert!(!h.state.can_undo());
}

#[test]
fn projections_share_the_graph_file_and_undo_history() {
    let mut h = Harness::new();
    h.state.load_scene(Scene::example_2d(), None);
    let path = std::path::PathBuf::from("scenes/one-composition.ron");
    h.state.path = Some(path.clone());
    let graph = h.state.scene.creation.clone();
    h.state.switch_dimension(syfrack_core::Dimension::ThreeD);
    assert_eq!(h.state.scene.dimension, syfrack_core::Dimension::ThreeD);
    assert_eq!(h.state.scene.creation, graph);
    assert_eq!(h.state.path, Some(path.clone()));
    h.state.set_param("look.exposure_ev", 1.0);
    h.state.switch_dimension(syfrack_core::Dimension::TwoD);
    assert_eq!(h.state.scene.look.exposure_ev, 1.0);
    assert_eq!(h.state.scene.creation, graph);
    assert_eq!(h.state.path, Some(path));
    assert!(h.state.is_modified());
    assert!(h.state.undo());
    assert_eq!(h.state.scene.dimension, syfrack_core::Dimension::ThreeD);
    assert!(h.state.undo());
    assert_eq!(h.state.scene.look.exposure_ev, 0.0);
}

#[test]
fn texture_graph_parameters_change_only_the_texture_and_reject_zero_scale() {
    let mut h = Harness::new();
    let mut scene = Scene::example();
    scene.material.texture = Some(syfrack_core::FractalTexture::default());
    h.state.load_scene(scene, None);
    let form = h.state.scene.fractal.clone();
    let texture = h.state.scene.material.texture.clone();
    assert!(!h
        .state
        .set_param("material.texture.graph.nodes[5].factor", 0.0));
    assert_eq!(h.state.scene.material.texture, texture);
    assert!(h
        .state
        .set_param("material.texture.graph.nodes[5].factor", 2.5));
    assert_eq!(h.state.scene.fractal, form);
    assert!(h.state.undo());
    assert_eq!(h.state.scene.material.texture, texture);
}

#[test]
fn ocean_controls_activate_a_valid_surface_and_support_undo() {
    let mut h = Harness::new();
    h.screen = Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 1800.0));
    h.body = |ui, state, _, _, _| {
        ui.set_max_width(430.0);
        panels::atmosphere::show(ui, state);
    };
    h.state.scene.atmosphere.seascape = Some(syfrack_core::Seascape::default());
    let before = h.state.scene.clone();
    h.frame(vec![]);
    h.frame(vec![]);
    let toggle = control(
        &h,
        egui::Id::new(("option-toggle", "Surface aquatique avec profondeur")),
    );
    assert!(h.screen.contains_rect(toggle));
    click(&mut h, toggle.center());
    assert!(h
        .state
        .scene
        .atmosphere
        .seascape
        .as_ref()
        .unwrap()
        .surface
        .is_some());
    h.state.scene.validate().unwrap();
    for field in [
        "wave_height",
        "wavelength",
        "foam_amount",
        "shore_width",
        "depth",
        "roughness",
        "scattering",
    ] {
        let path = format!("atmosphere.seascape.surface.{field}");
        let row = control(&h, egui::Id::new(("param-row", path.as_str())));
        assert!(h.screen.contains_rect(row), "{path}");
    }
    assert!(h.state.undo());
    assert_eq!(h.state.scene, before);
}

#[test]
fn compact_workspace_keeps_actions_and_an_image_area_with_a_long_filename() {
    let mut h = Harness::new();
    h.screen = Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 640.0));
    h.state.path = Some(std::path::PathBuf::from(format!(
        "scenes/{}.ron",
        "palais-".repeat(40)
    )));
    let mut scene = Scene::example();
    scene.material.texture = Some(syfrack_core::FractalTexture::default());
    h.state.load_scene(scene, None);
    h.state.scene.atmosphere.seascape = Some(syfrack_core::Seascape::default());
    for &page in workshops::pages(syfrack_core::Dimension::ThreeD) {
        h.state.inspector = page;
        h.frame(vec![]);
        h.frame(vec![]);
        assert!(
            h.view_available.0 >= 500 && h.view_available.1 >= 400,
            "{page:?}: {:?}",
            h.view_available
        );
        let render = control(&h, egui::Id::new("render-open-button"));
        assert!(
            !render.intersects(control(&h, egui::Id::new("document-title"))),
            "title must leave room for document actions"
        );
        assert!(
            h.screen.contains_rect(render),
            "the render action stays visible: {render:?}"
        );
        for &page in workshops::pages(syfrack_core::Dimension::ThreeD) {
            assert!(h.screen.contains_rect(control(
                &h,
                egui::Id::new((
                    "inspector-page",
                    syfrack_i18n::with_language(h.language, || page.label())
                ))
            )));
        }
    }
}

#[test]
fn command_bar_condenses_and_its_history_and_display_controls_work() {
    let mut h = Harness::new();
    h.state.path = Some(format!("scenes/{}.ron", "couronne-très-longue-".repeat(30)).into());
    let original = h.state.scene.clone();
    h.state.edit(|s| s.look.exposure_ev += 1.0);
    for width in [1920.0, 1280.0, 1000.0, 800.0, 700.0] {
        h.screen = Rect::from_min_size(Pos2::ZERO, egui::vec2(width, 900.0));
        h.frame(vec![]);
        h.frame(vec![]);
        let render = control(&h, egui::Id::new("render-open-button"));
        assert!(h.screen.contains_rect(render));
        assert!(!render.intersects(control(&h, egui::Id::new("document-title"))));
        let gallery = control(&h, egui::Id::new(("workspace-tab", "Galerie")));
        assert!(h.screen.contains_rect(gallery));
        assert!(
            gallery.bottom() < if width >= 1500.0 { 52.0 } else { 88.0 },
            "{width}: {gallery:?}"
        );
    }
    let at = control(&h, egui::Id::new(("history", "Annuler"))).center();
    click(&mut h, at);
    assert_eq!(h.state.scene, original);
    let at = control(&h, egui::Id::new(("history", "Rétablir"))).center();
    click(&mut h, at);
    assert_eq!(
        h.state.scene.look.exposure_ev,
        original.look.exposure_ev + 1.0
    );
    let at = control(&h, egui::Id::new("display-menu")).center();
    click(&mut h, at);
    h.frame(vec![]);
    let at = control(&h, egui::Id::new("display-image-only")).center();
    click(&mut h, at);
    assert!(h.state.image_only);
    assert_eq!(
        h.state.scene.look.exposure_ev,
        original.look.exposure_ev + 1.0
    );
}

fn control(h: &Harness, id: egui::Id) -> Rect {
    h.ctx
        .data(|d| d.get_temp::<Rect>(id))
        .expect("control drawn in the preceding frame")
}

fn fold_point(h: &Harness, t: f32) -> Pos2 {
    let r = control(h, egui::Id::new(("param-row", "fractal.nodes[2].limit")));
    let left = r.left() + widgets::label_width(r.width());
    let right = r.right() - widgets::VALUE_W - theme::GRID;
    Pos2::new(left + (right - left) * t, r.center().y)
}

fn library_point(h: &Harness) -> Pos2 {
    control(h, egui::Id::new(("library-entry", "Bulbe"))).center()
}

/// The harness on the Atelier with the Mandelbox as a form graph.
fn atelier_graph_harness() -> Harness {
    let mut h = Harness::new();
    h.body = |ui, state, _, _, _| {
        coalesce_gestures(ui, state);
        shortcuts(ui, state);
        panels::param("fractal.nodes[2].limit", "Pli boîte").show(ui, state);
        panels::fractal::show(ui, state, &[]);
    };
    h.frame(vec![]);
    h.frame(vec![]);
    h
}

fn fold_limit(s: &AppState) -> f32 {
    match s.scene.fractal.graph().node(2).unwrap().kind {
        syfrack_core::NodeKind::BoxFold { limit } => limit,
        _ => panic!("Mandelbox fold"),
    }
}

/// Centre of the first library entry, of the « look réel » switch and
/// of the Atelier's « Éditer la forme » button.

/// « Ouvrir quand même » of the unsaved-edits modal.

/// A library folder holding the Mandelbulb graph as « Bulbe ».
fn library_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("syfrack-ui-lib-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    crate::library::save(
        &dir,
        "Bulbe",
        &syfrack_core::Graph::bundled("mandelbulb").unwrap(),
    )
    .unwrap();
    dir
}

fn library_harness(dir: &std::path::Path) -> Harness {
    let mut h = Harness::new();
    h.body = geometry_compatibility;
    h.form = FormPanel::with_library(dir.to_path_buf());
    h.form.mode = form_panel::FormMode::Expert;
    h.state.tab = Tab::Forme;
    h.frame(vec![]);
    h.frame(vec![]);
    h
}

fn click(h: &mut Harness, at: Pos2) {
    h.frame(vec![Event::PointerMoved(at)]);
    h.button(at, true);
    h.button(at, false);
    h.frame(vec![]);
}

#[test]
fn a_library_click_loads_the_form_in_one_step_and_frames_it() {
    let dir = library_dir("load");
    let mut h = library_harness(&dir);
    let bulb = syfrack_core::Graph::bundled("mandelbulb").unwrap();
    let before = h.state.scene.fractal.clone();
    let at = library_point(&h);
    click(&mut h, at);
    assert_eq!(
        h.state.scene.fractal,
        syfrack_core::Fractal::Graph(bulb.clone())
    );
    assert_eq!(h.state.form.view, form_panel::view_for(&bulb));
    assert!(h.state.undo(), "one step");
    assert_eq!(h.state.scene.fractal, before);
    assert!(!h.state.can_undo());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_library_click_over_unsaved_edits_asks_first() {
    let dir = library_dir("confirm");
    let mut h = library_harness(&dir);
    h.state.set_param("look.exposure_ev", 1.0);
    let before = h.state.scene.fractal.clone();
    let at = library_point(&h);
    click(&mut h, at);
    assert_eq!(h.state.scene.fractal, before, "nothing replaced yet");
    // A rescan under the modal puts another form first.
    crate::library::save(
        &dir,
        "Aaa",
        &syfrack_core::Graph::bundled("mandelbox").unwrap(),
    )
    .unwrap();
    h.form.library.rescan_now();
    h.frame(vec![]);
    assert_eq!(h.form.library.entries[0].name, "Aaa", "reordered");
    // « Ouvrir quand même » loads the form that was clicked.
    let at = control(&h, egui::Id::new("confirm-discard-button")).center();
    click(&mut h, at);
    assert_eq!(
        h.state.scene.fractal,
        syfrack_core::Fractal::Graph(syfrack_core::Graph::bundled("mandelbulb").unwrap()),
        "the clicked Bulbe, not the new first entry"
    );
    assert!(h.state.undo(), "one step");
    assert_eq!(h.state.scene.fractal, before);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_library_entry_dropped_on_the_canvas_joins_the_graph_in_one_step() {
    let dir = library_dir("drop");
    let mut h = library_harness(&dir);
    // On a form graph: the Mandelbox.
    let mut scene = h.state.scene.clone();
    scene.fractal =
        syfrack_core::Fractal::Graph(syfrack_core::Graph::bundled("mandelbox").unwrap());
    assert!(h.state.load_scene(scene, None));
    h.frame(vec![]);
    let before = graph(&h);
    let bulb = syfrack_core::Graph::bundled("mandelbulb").unwrap();
    let added = bulb.nodes.len() - 2; // its Entrée and Sortie stay out
    h.drag(library_point(&h), Pos2::new(800.0, 700.0));
    let after = graph(&h);
    assert_eq!(after.nodes.len(), before.nodes.len() + added);
    assert_eq!(
        h.state.form.selected.len(),
        added,
        "the new cards are selected"
    );
    assert!(after.validate_structure().is_ok());
    assert!(h.state.undo());
    assert_eq!(graph(&h), before, "one step");
    std::fs::remove_dir_all(&dir).unwrap();
}

fn type_name(h: &mut Harness, text: &str) {
    h.frame(vec![Event::Text(text.into())]);
    h.key(Key::Enter, Modifiers::NONE, Modifiers::NONE);
    h.frame(vec![]);
}

#[test]
fn saving_the_form_writes_it_once_and_asks_for_its_thumbnail() {
    let dir = library_dir("save");
    let mut h = forme_harness();
    h.form = FormPanel::with_library(dir.clone());
    h.frame(vec![]);
    h.form.library.open_save_dialog();
    h.frame(vec![]);
    h.frame(vec![]);
    type_name(&mut h, "Ma boîte");
    assert!(!h.form.library.saving(), "closed once written");
    let ron = dir.join("ma-boite.ron");
    assert!(ron.is_file());
    assert_eq!(
        h.form.library.thumbnail_jobs,
        vec![(graph(&h), ron.with_extension("png"))]
    );
    h.form.library.thumbnail_jobs.clear();
    h.frame(vec![]);
    assert!(h.form.library.entries.iter().any(|e| e.name == "Ma boîte"));
    // The same name again: refused until « Remplacer ».
    let before = std::fs::read_to_string(&ron).unwrap();
    h.form.library.open_save_dialog();
    h.frame(vec![]);
    h.frame(vec![]);
    type_name(&mut h, "Ma boîte");
    assert!(h.form.library.saving(), "still open: the file exists");
    assert!(h.form.library.thumbnail_jobs.is_empty());
    assert_eq!(std::fs::read_to_string(&ron).unwrap(), before);
    assert!(!h.state.can_undo(), "saving is not a scene change");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_preview_switch_turns_clay_off_and_on() {
    let mut h = forme_harness();
    assert!(!h.form.clay, "real appearance by default");
    h.form.clay = true;
    h.frame(vec![]);
    let before = h.state.scene.clone();
    let at = control(&h, egui::Id::new(("choice", "Aperçu", "look réel"))).center();
    click(&mut h, at);
    assert!(!h.form.clay);
    assert_eq!(h.state.scene, before, "not a scene change");
    assert!(!h.state.can_undo());
}

#[test]
fn the_control_preview_offers_specialization_before_the_real_look_switch() {
    let mut h = forme_harness();
    h.form.view.can_specialize = true;
    h.frame(vec![]);
    h.frame(vec![]);
    assert!(h.labels.iter().any(|s| s == "Compiler l’aperçu spécialisé"));
    assert!(!h.labels.iter().any(|s| s == "look réel"));
    h.form.view.specialized = true;
    h.frame(vec![]);
    h.frame(vec![]);
    assert!(h.labels.iter().any(|s| s == "Revenir au contrôle GPU"));
    assert!(h.labels.iter().any(|s| s == "look réel"));
}

#[test]
fn edit_the_form_opens_the_forme_tab() {
    let mut h = atelier_graph_harness();
    let at = control(&h, egui::Id::new("edit-form-button")).center();
    click(&mut h, at);
    assert_eq!(h.state.tab, Tab::Forme);
}

#[test]
fn ctrl_z_after_a_slider_drag_undoes_it() {
    let mut h = atelier_graph_harness();
    let before = fold_limit(&h.state);
    // Use the live row, independent of the inspector layout.
    h.drag(fold_point(&h, 0.3), fold_point(&h, 0.9));
    assert_ne!(fold_limit(&h.state), before, "the drag changed the fold");
    assert!(h.state.can_undo());
    // Pointer over the viewport, then Ctrl+Z.
    h.frame(vec![Event::PointerMoved(Pos2::new(800.0, 450.0))]);
    h.key(Key::Z, CTRL, CTRL);
    assert_eq!(fold_limit(&h.state), before, "Ctrl+Z reverted the drag");
}

#[test]
fn ctrl_z_after_a_click_on_a_slider_undoes_it() {
    let mut h = atelier_graph_harness();
    let before = fold_limit(&h.state);
    let at = fold_point(&h, 0.3);
    h.frame(vec![Event::PointerMoved(at)]);
    h.button(at, true);
    h.button(at, false);
    h.frame(vec![]);
    assert_ne!(fold_limit(&h.state), before, "the click moved the fold");
    h.key(Key::Z, CTRL, CTRL);
    assert_eq!(fold_limit(&h.state), before, "Ctrl+Z reverted the click");
}

/// Only the material ramp, full width from the top left corner: its band
/// spans x 8..1592, y 6..18, three stops at x 8, 800, 1592.
fn ramp_only(
    ui: &mut egui::Ui,
    state: &mut AppState,
    _: &mut Viewport,
    _: &mut GalleryPanel,
    _: &mut FormPanel,
) {
    coalesce_gestures(ui, state);
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            let mut ramp = state
                .scene
                .material
                .coloring
                .ramp()
                .expect("the harness scene has a colouring ramp")
                .clone();
            if widgets::ramp_editor(ui, &mut ramp) {
                state.edit(|s| {
                    if let Some(r) = s.material.coloring.ramp_mut() {
                        *r = ramp;
                    }
                });
            }
        });
}

#[test]
fn dragging_a_ramp_stop_out_is_one_undo_step() {
    let mut h = Harness::new();
    h.body = ramp_only;
    h.frame(vec![]);
    let before = h.state.scene.material.coloring.clone();
    h.drag(Pos2::new(800.0, 12.0), Pos2::new(800.0, 90.0));
    let ramp = h
        .state
        .scene
        .material
        .coloring
        .ramp()
        .expect("still a ramp colouring");
    assert_eq!(
        ramp.stops.len(),
        2,
        "the dragged stop left the band and was removed"
    );
    assert!(h.state.undo());
    assert_eq!(
        h.state.scene.material.coloring, before,
        "one undo restores the stop and its colour"
    );
    assert!(!h.state.can_undo(), "the whole drag was one step");
}

#[test]
fn a_press_in_a_panel_closes_a_navigation_gesture() {
    let mut h = atelier_graph_harness();
    let before = fold_limit(&h.state);
    // A wheel zoom just happened: its gesture is open for 0.3 s.
    let now = h.time;
    h.ctx.data_mut(|d| d.insert_temp(viewport::nav_id(), now));
    h.state.begin_drag();
    h.state.edit(|s| s.camera.focal_mm = 50.0);
    let at = fold_point(&h, 0.3);
    h.button(at, true);
    h.button(at, false);
    assert_ne!(fold_limit(&h.state), before);
    assert!(h.state.undo(), "undo the click");
    assert_eq!(fold_limit(&h.state), before);
    assert_eq!(
        h.state.scene.camera.focal_mm, 50.0,
        "the zoom is its own step"
    );
}

#[test]
fn ctrl_s_without_a_file_opens_save_as() {
    let mut h = Harness::new();
    h.state.set_param("look.exposure_ev", 1.0);
    h.key(Key::S, CTRL, CTRL);
    assert!(h.state.banner.is_none(), "no error banner");
    let dialog = h.ctx.data(|d| d.get_temp::<FileDialog>(dialog_id()));
    assert!(dialog.is_some_and(|d| d.action == FileAction::SaveAs));
}

#[test]
fn ctrl_z_with_modifiers_only_on_the_key_event_still_undoes() {
    let mut h = Harness::new();
    let before = h.state.scene.look.exposure_ev;
    h.state.set_param("look.exposure_ev", 1.0);
    h.key(Key::Z, CTRL, Modifiers::NONE);
    assert_eq!(h.state.scene.look.exposure_ev, before);
    h.key(Key::Y, CTRL, Modifiers::NONE);
    assert_eq!(h.state.scene.look.exposure_ev, 1.0, "Ctrl+Y redoes");
}

/// The harness on the Forme tab with the Mandelbox graph.
fn geometry_compatibility(
    ui: &mut egui::Ui,
    state: &mut AppState,
    _: &mut Viewport,
    gallery: &mut GalleryPanel,
    form: &mut FormPanel,
) {
    // Exercise the retained legacy editor against a legacy document. The unified
    // production canvas has separate tests in creation::tests.
    state.scene.creation = None;
    state.scene.creation_draft = None;
    coalesce_gestures(ui, state);
    shortcuts(ui, state);
    form.show_geometry_compatibility(ui, state);
    dialogs(ui, state, gallery, form);
}

fn forme_harness() -> Harness {
    forme_harness_with(|_| {})
}

/// The same, with the graph adjusted before loading.
fn forme_harness_with(f: impl FnOnce(&mut syfrack_core::Graph)) -> Harness {
    let mut h = Harness::new();
    h.body = geometry_compatibility;
    h.form.mode = form_panel::FormMode::Expert;
    let mut scene = h.state.scene.clone();
    let mut g = syfrack_core::Graph::bundled("mandelbox").unwrap();
    f(&mut g);
    scene.creation = None;
    scene.fractal = syfrack_core::Fractal::Graph(g);
    let legacy = scene.clone();
    assert!(h.state.load_scene(scene, None));
    h.state.scene = legacy;
    h.state.tab = Tab::Forme;
    h.frame(vec![]);
    h.frame(vec![]);
    h
}

fn node_pos(s: &AppState, id: u32) -> [f32; 2] {
    s.scene.fractal.to_graph().node(id).unwrap().pos
}

fn screen_of(h: &Harness, world: Pos2) -> Pos2 {
    h.state.form.view.to_screen(h.form.rect.min, world)
}

#[test]
fn dragging_a_card_is_one_undo_step_and_a_click_adds_none() {
    let mut h = forme_harness();
    let before = node_pos(&h.state, 7);
    let g = h.state.scene.fractal.to_graph();
    let at = screen_of(&h, crate::canvas::card_rect(g.node(7).unwrap()).center());
    h.drag(at, at + egui::vec2(80.0, 40.0));
    assert_eq!(node_pos(&h.state, 7), [before[0] + 80.0, before[1] + 40.0]);
    assert_eq!(h.state.form.selected, vec![7]);
    let moved = at + egui::vec2(80.0, 40.0);
    h.button(moved, true);
    h.button(moved, false);
    h.frame(vec![]);
    assert!(h.state.undo());
    assert_eq!(node_pos(&h.state, 7), before, "one undo returns the card");
    assert!(!h.state.can_undo(), "the drag was one step, the click none");
}

#[test]
fn moving_a_repeat_frame_carries_its_body() {
    let mut h = forme_harness();
    let (it, body) = (node_pos(&h.state, 1), node_pos(&h.state, 4));
    let g = h.state.scene.fractal.to_graph();
    // By its title: the middle of the card is its « itérations » rail.
    let r = crate::canvas::card_rect(g.node(1).unwrap());
    let at = screen_of(&h, Pos2::new(r.center().x, r.min.y + 12.0));
    h.drag(at, at + egui::vec2(0.0, 48.0));
    assert_eq!(node_pos(&h.state, 1), [it[0], it[1] + 48.0]);
    assert_eq!(node_pos(&h.state, 4), [body[0], body[1] + 48.0]);
}

#[test]
fn a_lasso_on_the_empty_canvas_selects_the_cards_inside() {
    let mut h = forme_harness();
    let from = screen_of(&h, Pos2::new(-30.0, -30.0));
    let to = screen_of(&h, Pos2::new(200.0, 40.0));
    h.drag(from, to);
    assert_eq!(h.state.form.selected, vec![0]);
    assert!(!h.state.can_undo(), "selecting is not a scene change");
}

fn card_centre(h: &Harness, id: u32) -> Pos2 {
    let g = h.state.scene.fractal.to_graph();
    screen_of(h, crate::canvas::card_rect(g.node(id).unwrap()).center())
}

#[test]
fn a_click_on_an_off_grid_card_leaves_the_scene_untouched() {
    let mut h = forme_harness_with(|g| g.node_mut(7).unwrap().pos = [37.0, 13.0]);
    let at = card_centre(&h, 7);
    h.frame(vec![Event::PointerMoved(at)]);
    h.button(at, true);
    h.button(at, false);
    h.frame(vec![]);
    assert_eq!(h.state.form.selected, vec![7]);
    assert_eq!(node_pos(&h.state, 7), [37.0, 13.0]);
    assert!(!h.state.can_undo() && !h.state.is_modified());
    h.drag(at, at + egui::vec2(20.0, 0.0));
    let p = node_pos(&h.state, 7);
    assert_ne!(p[0], 37.0, "the drag moved the card");
    assert_eq!((p[0] - 37.0) % 8.0, 0.0, "by a multiple of the grid");
    assert_eq!(p[1], 13.0);
}

fn graph(h: &Harness) -> syfrack_core::Graph {
    h.state.scene.fractal.to_graph()
}

fn out_port(h: &Harness, id: u32) -> Pos2 {
    screen_of(
        h,
        crate::canvas::out_port_pos(graph(h).node(id).unwrap(), 0),
    )
}

fn in_port(h: &Harness, id: u32) -> Pos2 {
    screen_of(h, crate::canvas::in_port_pos(graph(h).node(id).unwrap(), 0))
}

/// The Mandelbox graph and a dangling Torsion (id 9) at (600, 600).
fn with_twist() -> Harness {
    forme_harness_with(|g| {
        crate::graph_edit::add_node(
            g,
            syfrack_core::NodeKind::Twist { rate: 0.2 },
            [600.0, 600.0],
        );
    })
}

fn secondary_click(h: &mut Harness, at: Pos2) {
    h.frame(vec![Event::PointerMoved(at)]);
    for pressed in [true, false] {
        h.frame(vec![Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        }]);
    }
}

#[test]
fn a_wire_dropped_on_a_free_input_links_it_in_one_step() {
    let mut h = with_twist();
    h.drag(out_port(&h, 0), in_port(&h, 9));
    assert_eq!(graph(&h).input_of(9, 0), Some((0, 0)));
    assert!(h.state.undo());
    assert_eq!(graph(&h).input_of(9, 0), None);
    assert!(!h.state.can_undo(), "one step");
}

#[test]
fn a_point_wire_on_the_output_flashes_and_writes_nothing() {
    let mut h = forme_harness();
    let before = h.state.scene.clone();
    h.drag(out_port(&h, 0), in_port(&h, 8));
    assert_eq!(h.state.scene, before);
    assert!(!h.state.can_undo());
    assert!(h.form.flash.is_some(), "the refused port flashes");
    assert!(
        h.state.banner.as_ref().is_none_or(|b| !b.error),
        "no error banner: the flash says it"
    );
}

#[test]
fn a_wire_released_on_the_void_opens_the_add_menu_and_links_the_new_node() {
    let mut h = forme_harness();
    let void = screen_of(&h, Pos2::new(700.0, 640.0));
    h.drag(out_port(&h, 0), void);
    let Some(menu) = &h.form.menu else {
        panic!("the add menu opened")
    };
    assert!(matches!(
        &menu.kind,
        form_panel::MenuKind::Add { wire: Some(_), .. }
    ));
    h.frame(vec![Event::Text("miroir".into())]);
    h.key(Key::Enter, Modifiers::NONE, Modifiers::NONE);
    assert!(h.form.menu.is_none(), "Enter picked the first match");
    let g = graph(&h);
    let m = g
        .nodes
        .iter()
        .find(|n| matches!(n.kind, syfrack_core::NodeKind::Mirror { .. }))
        .expect("a Miroir was added");
    assert_eq!(g.input_of(m.id, 0), Some((0, 0)), "linked to the wire");
    assert_eq!(h.state.form.selected, vec![m.id]);
    assert!(h.state.undo());
    assert_eq!(graph(&h).nodes.len(), 9, "one step");
}

#[test]
fn escape_cancels_a_wire_in_progress() {
    let mut h = with_twist();
    let from = out_port(&h, 0);
    h.frame(vec![Event::PointerMoved(from)]);
    h.button(from, true);
    h.frame(vec![Event::PointerMoved(in_port(&h, 9))]);
    assert!(h.form.wire().is_some());
    h.key(Key::Escape, Modifiers::NONE, Modifiers::NONE);
    assert!(h.form.wire().is_none());
    h.button(in_port(&h, 9), false);
    assert_eq!(graph(&h).input_of(9, 0), None);
    assert!(!h.state.can_undo());
}

#[test]
fn delete_ctrl_d_and_ctrl_a_act_on_the_selection() {
    let mut h = with_twist();
    let at = card_centre(&h, 9);
    h.frame(vec![Event::PointerMoved(at)]);
    h.button(at, true);
    h.button(at, false);
    assert_eq!(h.state.form.selected, vec![9]);
    h.key(Key::D, CTRL, CTRL);
    assert_eq!(graph(&h).nodes.len(), 11);
    assert_eq!(h.state.form.selected, vec![10], "the copy is selected");
    h.key(Key::Delete, Modifiers::NONE, Modifiers::NONE);
    assert!(graph(&h).node(10).is_none());
    h.key(Key::A, CTRL, CTRL);
    assert_eq!(h.state.form.selected.len(), 10);
    // Deleting everything is accepted: the Entrée and the Sortie stay,
    // the Sortie waits for a wire.
    h.key(Key::Delete, Modifiers::NONE, Modifiers::NONE);
    let g = graph(&h);
    assert_eq!(g.nodes.len(), 2);
    assert_eq!(g.unwired_inputs(), vec![(8, 0)]);
    g.validate_structure().unwrap();
    assert!(h.state.undo() && h.state.undo() && h.state.undo());
    assert!(
        !h.state.can_undo(),
        "duplicate and two deletes, one step each"
    );
}

#[test]
fn deleting_a_mid_chain_card_bypasses_it() {
    let mut h = forme_harness();
    let r = crate::canvas::card_rect(graph(&h).node(1).unwrap());
    let at = screen_of(&h, Pos2::new(r.center().x, r.min.y + 12.0));
    h.frame(vec![Event::PointerMoved(at)]);
    h.button(at, true);
    h.button(at, false);
    h.key(Key::Delete, Modifiers::NONE, Modifiers::NONE);
    let g = graph(&h);
    assert!(g.node(1).is_none() && g.node(2).is_none(), "body went too");
    assert_eq!(g.input_of(7, 0), Some((0, 0)));
    g.validate().unwrap();
}

#[test]
fn a_twist_dropped_in_the_frame_joins_the_body_in_one_step() {
    let mut h = with_twist();
    let from = card_centre(&h, 9);
    // Between Piège (y 192) and Pli sphère (y 288).
    let to = screen_of(&h, Pos2::new(336.0, 264.0));
    h.drag(from, to);
    let body = graph(&h).body_ids_of(1).to_vec();
    assert_eq!(body, vec![2, 3, 9, 4, 5, 6]);
    assert_stacked(&graph(&h), 1);
    assert!(h.state.undo());
    assert_eq!(node_pos(&h.state, 9), [600.0, 600.0]);
    assert!(!graph(&h).body_ids_of(1).contains(&9));
    assert!(!h.state.can_undo(), "move and membership are one step");
    // And out again.
    assert!(h.state.redo());
    let at = card_centre(&h, 9);
    h.drag(at, at + egui::vec2(400.0, 0.0));
    assert!(!graph(&h).body_ids_of(1).contains(&9));
    assert_stacked(&graph(&h), 1);
}

/// The body of `frame` runs down its column in body order, no overlap.
fn assert_stacked(g: &syfrack_core::Graph, frame: u32) {
    let x = g.node(frame).unwrap().pos[0] + crate::canvas::FRAME_PAD;
    let rects: Vec<Rect> = g
        .body_ids_of(frame)
        .iter()
        .map(|b| crate::canvas::card_rect(g.node(*b).unwrap()))
        .collect();
    assert!(rects.iter().all(|r| r.min.x == x), "one column");
    for w in rects.windows(2) {
        assert!(w[0].max.y <= w[1].min.y, "body order, no overlap");
    }
}

#[test]
fn nudging_a_card_inside_its_frame_leaves_no_step() {
    let mut h = forme_harness_with(|g| crate::canvas::restack_body(g, 1));
    let at = card_centre(&h, 4);
    let at = Pos2::new(at.x - 60.0, at.y - 20.0); // its title
    h.drag(at, at + egui::vec2(16.0, 8.0));
    assert!(!h.state.can_undo(), "the restack put it back: no step");
    assert!(!h.state.is_modified());
}

#[test]
fn leaving_a_frame_restacks_what_stays() {
    let mut h = forme_harness();
    // « Sortir du cadre » runs this edit.
    assert!(h.state.edit_graph(|g| form_panel::leave_frame(g, 3)));
    let g = graph(&h);
    assert!(!g.body_ids_of(1).contains(&3));
    assert_stacked(&g, 1);
    let frame = crate::canvas::frame_rect(&g, g.node(1).unwrap());
    assert!(g.node(3).unwrap().pos[0] > frame.max.x);
}

#[test]
fn dragging_an_inline_rail_sets_the_parameter_in_one_step() {
    let mut h = forme_harness();
    let g = graph(&h);
    let z = h.state.form.view.zoom;
    let card = crate::canvas::card_rect(g.node(5).unwrap());
    let r = Rect::from_min_max(screen_of(&h, card.min), screen_of(&h, card.max));
    let rail = form_panel::row_rail(r, 0, 0, 1, z);
    h.drag(rail.center(), rail.right_center());
    let factor = |h: &Harness| match graph(h).node(5).unwrap().kind {
        syfrack_core::NodeKind::Scale { factor } => factor,
        _ => unreachable!(),
    };
    assert!((factor(&h) - 4.0).abs() < 1e-3, "{}", factor(&h));
    assert_eq!(
        node_pos(&h.state, 5),
        g.node(5).unwrap().pos,
        "the card stayed"
    );
    assert!(h.state.undo());
    assert!(!h.state.can_undo(), "one step");
}

fn scale_rail(h: &Harness) -> Rect {
    let g = graph(h);
    let card = crate::canvas::card_rect(g.node(5).unwrap());
    let r = Rect::from_min_max(screen_of(h, card.min), screen_of(h, card.max));
    form_panel::row_rail(r, 0, 0, 1, h.state.form.view.zoom)
}

fn factor(h: &Harness) -> f32 {
    match graph(h).node(5).unwrap().kind {
        syfrack_core::NodeKind::Scale { factor } => factor,
        _ => unreachable!(),
    }
}

#[test]
fn a_plain_click_on_an_inline_rail_writes_nothing() {
    let mut h = forme_harness_with(|g| {
        if let syfrack_core::NodeKind::Scale { factor } = &mut g.node_mut(5).unwrap().kind {
            *factor = 3.0;
        }
    });
    let at = scale_rail(&h).left_center() + egui::vec2(4.0, 0.0); // far from the knob
    h.frame(vec![Event::PointerMoved(at)]);
    h.button(at, true);
    h.button(at, false);
    h.frame(vec![]);
    assert_eq!(factor(&h), 3.0);
    assert!(!h.state.can_undo());
}

#[test]
fn a_press_on_the_knob_moves_it_at_once() {
    let mut h = forme_harness_with(|g| {
        if let syfrack_core::NodeKind::Scale { factor } = &mut g.node_mut(5).unwrap().kind {
            *factor = 3.0;
        }
    });
    let rail = scale_rail(&h);
    let knob = Pos2::new(rail.left() + rail.width() * 7.0 / 8.0, rail.center().y);
    h.frame(vec![Event::PointerMoved(knob)]);
    h.button(knob, true);
    h.frame(vec![Event::PointerMoved(knob + egui::vec2(2.0, 0.0))]);
    h.button(knob + egui::vec2(2.0, 0.0), false);
    assert!(factor(&h) > 3.0, "{}", factor(&h));
    assert!(h.state.undo() && !h.state.can_undo());
}

#[test]
fn a_double_click_on_an_inline_rail_resets_it_in_one_step() {
    let mut h = forme_harness_with(|g| {
        if let syfrack_core::NodeKind::Scale { factor } = &mut g.node_mut(5).unwrap().kind {
            *factor = 3.0;
        }
    });
    let at = scale_rail(&h).left_center() + egui::vec2(4.0, 0.0);
    h.frame(vec![Event::PointerMoved(at)]);
    for _ in 0..2 {
        h.button(at, true);
        h.button(at, false);
    }
    h.frame(vec![]);
    assert_eq!(factor(&h), 2.0, "the default of « Échelle »");
    assert!(h.state.undo());
    assert_eq!(factor(&h), 3.0);
    assert!(!h.state.can_undo(), "one step");
}

#[test]
fn right_clicks_open_the_card_menu_or_the_add_menu() {
    let mut h = forme_harness();
    let at = card_centre(&h, 7);
    secondary_click(&mut h, at);
    let kind = h.form.menu.as_ref().map(|m| m.kind.clone());
    assert_eq!(kind, Some(form_panel::MenuKind::Card(7)));
    assert_eq!(h.state.form.selected, vec![7]);
    h.key(Key::Escape, Modifiers::NONE, Modifiers::NONE);
    assert!(h.form.menu.is_none());
    let at = screen_of(&h, Pos2::new(900.0, 700.0));
    secondary_click(&mut h, at);
    assert!(matches!(
        h.form.menu.as_ref().map(|m| &m.kind),
        Some(form_panel::MenuKind::Add { wire: None, .. })
    ));
    // A click elsewhere closes it and changes nothing.
    let away = screen_of(&h, Pos2::new(1000.0, 200.0));
    h.button(away, true);
    h.button(away, false);
    assert!(h.form.menu.is_none());
    assert!(!h.state.can_undo());
}

#[test]
fn a_lifted_wire_put_back_on_its_port_changes_nothing() {
    let mut h = with_twist();
    h.drag(out_port(&h, 0), in_port(&h, 9));
    h.drag(in_port(&h, 9), in_port(&h, 9) + egui::vec2(0.0, 1.0));
    assert_eq!(graph(&h).input_of(9, 0), Some((0, 0)));
    assert!(h.state.undo());
    assert!(!h.state.can_undo(), "putting it back is no step");
}

#[test]
fn a_lifted_wire_dropped_on_the_void_and_dismissed_is_removed() {
    // Even an active one: the graph may be incomplete.
    let mut h = forme_harness();
    let void = screen_of(&h, Pos2::new(900.0, 900.0));
    h.drag(in_port(&h, 7), void);
    assert!(h.form.menu.is_some());
    let away = screen_of(&h, Pos2::new(1000.0, 300.0));
    h.button(away, true);
    h.button(away, false);
    assert!(h.form.menu.is_none());
    let g = graph(&h);
    assert_eq!(g.input_of(7, 0), None, "the link went");
    assert!(g.unwired_inputs().contains(&(7, 0)));
    assert!(h.state.banner.as_ref().is_none_or(|b| !b.error));
    assert!(h.state.undo());
    assert!(!h.state.can_undo());
}

#[test]
fn a_lifted_active_wire_rerouted_leaves_its_old_port_unwired() {
    let mut h = with_twist();
    h.drag(in_port(&h, 7), in_port(&h, 9));
    let g = graph(&h);
    assert_eq!(g.input_of(9, 0), Some((1, 0)));
    assert_eq!(g.input_of(7, 0), None);
    assert_eq!(
        g.unwired_inputs(),
        vec![(7, 0)],
        "the fence waits for a wire"
    );
    assert!(h.state.undo() && !h.state.can_undo(), "one step");
    graph(&h).validate().unwrap();
}

#[test]
fn escape_restores_a_lifted_wire_in_every_state() {
    let mut h = forme_harness();
    let void = screen_of(&h, Pos2::new(900.0, 900.0));
    // At the add menu, its search field holding the keyboard.
    h.drag(in_port(&h, 7), void);
    assert!(h.form.menu.is_some());
    h.key(Key::Escape, Modifiers::NONE, Modifiers::NONE);
    assert!(h.form.menu.is_none());
    assert_eq!(graph(&h).input_of(7, 0), Some((1, 0)));
    // At the add menu, the field without the keyboard.
    h.drag(in_port(&h, 7), void);
    h.ctx.memory_mut(|m| m.stop_text_input());
    h.frame(vec![Event::PointerMoved(void)]);
    h.key(Key::Escape, Modifiers::NONE, Modifiers::NONE);
    assert!(h.form.menu.is_none());
    assert_eq!(graph(&h).input_of(7, 0), Some((1, 0)));
    // While dragging.
    let from = in_port(&h, 7);
    h.frame(vec![Event::PointerMoved(from)]);
    h.button(from, true);
    h.frame(vec![Event::PointerMoved(void)]);
    h.key(Key::Escape, Modifiers::NONE, Modifiers::NONE);
    h.button(void, false);
    assert_eq!(graph(&h).input_of(7, 0), Some((1, 0)));
    assert!(h.form.menu.is_none());
    assert!(!h.state.can_undo());
}

#[test]
fn the_move_on_the_release_frame_counts_and_stays_one_step() {
    let mut h = forme_harness();
    let before = node_pos(&h.state, 7);
    let at = card_centre(&h, 7);
    h.frame(vec![Event::PointerMoved(at)]);
    h.button(at, true);
    h.frame(vec![Event::PointerMoved(at + egui::vec2(40.0, 0.0))]);
    // The release carries the last 40 px.
    h.button(at + egui::vec2(80.0, 0.0), false);
    h.frame(vec![]);
    assert_eq!(node_pos(&h.state, 7), [before[0] + 80.0, before[1]]);
    assert!(h.state.undo());
    assert_eq!(node_pos(&h.state, 7), before);
    assert!(!h.state.can_undo(), "one step");
}

#[test]
fn unified_canvas_selection_keeps_the_scene_and_history() {
    let mut h = Harness::new();
    h.state.tab = Tab::Forme;
    h.frame(vec![]);
    h.frame(vec![]);
    let before = h.state.scene.clone();
    let out = before
        .creation
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .find(|n| matches!(n.kind, syfrack_core::CreationKind::Output))
        .unwrap()
        .id;
    let card = control(&h, egui::Id::new(("creation-node", out)));
    click(&mut h, card.min + egui::vec2(20.0, 12.0));
    assert_eq!(h.state.form.selected, [out]);
    assert_eq!(h.state.scene, before);
    assert!(!h.state.can_undo());
}

#[test]
fn explorer_selection_opens_a_scene_and_protects_unsaved_changes() {
    let dir = std::env::temp_dir().join(format!("syfrack-explorer-ui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let scene = Scene::example_for("menger").unwrap();
    scene.save(&dir.join("target.ron")).unwrap();
    let mut h = Harness::new();
    h.state.path = Some(dir.join("current.ron"));
    h.state.set_param("look.exposure_ev", 1.0);
    let before = h.state.scene.clone();
    h.key(Key::O, CTRL, CTRL);
    let id = egui::Id::new(("file-entry", "target.ron"));
    for _ in 0..100 {
        h.frame(vec![]);
        if h.ctx.data(|d| d.get_temp::<Rect>(id)).is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    for _ in 0..4 {
        h.frame(vec![]);
    }
    let at = control(&h, id).center();
    click(&mut h, at);
    assert_eq!(
        h.ctx
            .data(|d| d.get_temp::<FileDialog>(dialog_id()))
            .unwrap()
            .filename,
        "target.ron",
        "selected by click"
    );
    let at = control(&h, egui::Id::new("file-go")).center();
    assert!(
        h.screen.contains(at),
        "explorer actions stay inside the window"
    );
    click(&mut h, at);
    assert_eq!(h.state.scene, before, "confirmation before replacement");
    let at = control(&h, egui::Id::new("confirm-discard-button")).center();
    click(&mut h, at);
    assert_eq!(h.state.scene, scene.render_scene().unwrap());
    assert!(h
        .ctx
        .data(|d| d.get_temp::<FileDialog>(dialog_id()))
        .is_none());
    std::fs::remove_dir_all(dir).unwrap();
}
