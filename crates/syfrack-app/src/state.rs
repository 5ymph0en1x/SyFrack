use std::path::{Path, PathBuf};

use syfrack_core::{Fractal, Graph, NodeId, Scene, SceneError};

use crate::canvas::CanvasView;

/// Undo history depth: older steps are dropped.
const UNDO_DEPTH: usize = 200;

/// A one-line message under the title bar; red when `error`.
pub struct Banner {
    pub text: String,
    pub error: bool,
}

impl Banner {
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            error: true,
        }
    }

    pub fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            error: false,
        }
    }
}

/// The three screens of the title bar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Atelier,
    Forme,
    Galerie,
}

/// Workspace choices stay outside scene files and undo history.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Inspector {
    #[default]
    Scene,
    Lights,
    Material,
    Texture,
    Atmosphere,
    Look,
}

impl Inspector {
    pub const ALL: [Self; 6] = [
        Self::Scene,
        Self::Lights,
        Self::Material,
        Self::Texture,
        Self::Atmosphere,
        Self::Look,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Scene => syfrack_i18n::tr!("Scène"),
            Self::Lights => syfrack_i18n::tr!("Lumières"),
            Self::Material => syfrack_i18n::tr!("Matière"),
            Self::Texture => "Texture",
            Self::Atmosphere => syfrack_i18n::tr!("Atmosphère"),
            Self::Look => "Image",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Scene => syfrack_i18n::tr!("Géométrie, caméra et cadrage."),
            Self::Lights => syfrack_i18n::tr!("Sources, couleurs et douceur des ombres."),
            Self::Material => syfrack_i18n::tr!("Couleur de base et finitions de la surface."),
            Self::Texture => {
                syfrack_i18n::tr!("Un motif généré par un graphe fractal indépendant.")
            }
            Self::Atmosphere => syfrack_i18n::tr!("Brouillard, mer et ciel procéduraux."),
            Self::Look => syfrack_i18n::tr!("Exposition, couleurs et développement de l’image."),
        }
    }
}

/// What the « Forme » canvas shows: outside the undo history (selecting,
/// panning or zooming is not a scene change).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FormState {
    pub selected: Vec<NodeId>,
    pub view: CanvasView,
    /// Nodes refused by validation or compilation, with the message.
    pub errors: Vec<(NodeId, String)>,
}

struct Workspace {
    scene: Scene,
    path: Option<PathBuf>,
    undo: Vec<Scene>,
    redo: Vec<Scene>,
    dirty: bool,
    form: FormState,
}

/// The edited scene and its history. Every scene change goes through
/// `edit` or `set_param`: no undo step, no modification.
pub struct AppState {
    pub scene: Scene,
    pub path: Option<PathBuf>,
    undo: Vec<Scene>,
    redo: Vec<Scene>,
    dirty_since_save: bool,
    preview_dirty: bool,
    drag_in_progress: bool,
    /// The current drag already pushed its undo step.
    drag_has_snapshot: bool,
    /// Modified flag and redo history before the current drag's step.
    drag_restore: Option<(bool, Vec<Scene>)>,
    pub banner: Option<Banner>,
    pub tab: Tab,
    pub inspector: Inspector,
    pub show_gizmos: bool,
    pub image_only: bool,
    pub form: FormState,
    /// Keep the other workshop's composition when the artist changes dimension.
    workspaces: [Option<Workspace>; 2],
}

impl AppState {
    /// The preview is built from `scene` alongside, so it starts clean.
    pub fn new(scene: Scene, path: Option<PathBuf>) -> Self {
        Self {
            scene,
            path,
            undo: Vec::new(),
            redo: Vec::new(),
            dirty_since_save: false,
            preview_dirty: false,
            drag_in_progress: false,
            drag_has_snapshot: false,
            drag_restore: None,
            banner: None,
            tab: Tab::default(),
            inspector: Inspector::default(),
            show_gizmos: true,
            image_only: false,
            form: FormState::default(),
            workspaces: [None, None],
        }
    }

    /// Each workshop keeps its own scene, file and undo history during this session.
    pub fn switch_dimension(&mut self, dimension: syfrack_core::Dimension) {
        use syfrack_core::{Dimension, Tonemap};
        if self.scene.dimension == dimension {
            return;
        }
        let index = |d| if d == Dimension::TwoD { 0 } else { 1 };
        self.end_drag();
        let next = self.workspaces[index(dimension)].take().unwrap_or_else(|| {
            let scene = if dimension == Dimension::TwoD {
                if self
                    .scene
                    .material
                    .texture
                    .as_ref()
                    .and_then(|t| t.planar.as_ref())
                    .is_some()
                {
                    let mut s = self.scene.clone();
                    s.dimension = Dimension::TwoD;
                    let b = s.camera.basis();
                    if b.forward.z.abs() > 0.01 {
                        let distance = -b.origin.z / b.forward.z;
                        let center = b.origin + b.forward * distance;
                        s.canvas.center = [center.x, center.y];
                        s.canvas.zoom = (3.0 / (distance.abs() * b.tan_half_fov).max(0.01))
                            .clamp(0.01, 10000.0);
                    }
                    s.look.tonemap = Tonemap::Direct;
                    s.look.auto_exposure = None;
                    s.fractal = syfrack_core::Fractal::Graph(
                        syfrack_core::Graph::bundled("sphere").unwrap(),
                    );
                    s.lights.clear();
                    s.atmosphere = syfrack_core::Atmosphere::none();
                    s
                } else {
                    Scene::example_2d()
                }
            } else {
                Scene::example_for("mandelbulb").expect("bundled form")
            };
            Workspace {
                scene,
                path: None,
                undo: vec![],
                redo: vec![],
                dirty: false,
                form: FormState::default(),
            }
        });
        self.workspaces[index(self.scene.dimension)] = Some(Workspace {
            scene: self.scene.clone(),
            path: self.path.take(),
            undo: std::mem::take(&mut self.undo),
            redo: std::mem::take(&mut self.redo),
            dirty: self.dirty_since_save,
            form: self.form.clone(),
        });
        self.scene = next.scene;
        self.path = next.path;
        self.undo = next.undo;
        self.redo = next.redo;
        self.dirty_since_save = next.dirty;
        self.form = next.form;
        self.preview_dirty = true;
        self.tab = Tab::Atelier;
        self.inspector = Inspector::Scene;
    }

    /// Records the scene before a change; inside a drag only the first change does.
    fn snapshot_for_edit(&mut self) {
        if !(self.drag_in_progress && self.drag_has_snapshot) {
            self.undo.push(self.scene.clone());
            if self.undo.len() > UNDO_DEPTH {
                self.undo.remove(0);
            }
            self.drag_has_snapshot = self.drag_in_progress;
            if self.drag_in_progress {
                // What to give back if the gesture ends where it started.
                self.drag_restore = Some((self.dirty_since_save, self.redo.clone()));
            }
        }
        self.redo.clear();
        self.dirty_since_save = true;
        // A refusal is shown until the next change.
        self.form.errors.clear();
    }

    /// Every scene change goes through here: it records an undo step. The
    /// preview restarts only when a render input changed: moving or
    /// renaming a card does not (`render_differs`).
    pub fn edit(&mut self, f: impl FnOnce(&mut Scene)) {
        self.snapshot_for_edit();
        let before = render_view(&self.scene);
        f(&mut self.scene);
        self.preview_dirty |= render_differs(&before, &self.scene);
    }

    /// Re-importing the same panorama path may still change its file contents.
    pub fn refresh_preview(&mut self) {
        self.preview_dirty = true;
    }

    /// Sets one bounded parameter. A refused value leaves the scene and the
    /// history untouched and raises an error banner.
    pub fn set_param(&mut self, path: &str, value: f32) -> bool {
        let mut trial = self.scene.clone();
        // A form value can be in bounds yet break a graph rule (a zero
        // scale, the product of nested iterations).
        let checked = trial
            .set_param(path, value)
            .and_then(|_| match &trial.fractal {
                Fractal::Graph(g) if path.starts_with("fractal.") => g.validate_structure(),
                _ if path.starts_with("material.texture.graph.") => trial
                    .material
                    .texture
                    .as_ref()
                    .map_or(Ok(()), |t| t.graph.validate()),
                _ => Ok(()),
            });
        match checked {
            Ok(()) => {
                // A slider may round its display value without changing the parameter.
                if trial == self.scene {
                    return true;
                }
                self.snapshot_for_edit();
                self.scene = trial;
                self.preview_dirty = true;
                true
            }
            Err(e) => {
                self.banner = Some(Banner::error(e.to_string()));
                false
            }
        }
    }

    /// Edits the form graph on a copy and keeps it when its structure
    /// still holds (an incomplete graph, with unwired ports, is accepted:
    /// only rendering needs the full `Graph::validate`). A refusal leaves the
    /// scene and the history untouched, raises an error banner and marks the
    /// faulty node (`SceneError::Node`), else the first selected one. An edit
    /// that changes nothing adds no undo step.
    pub fn edit_graph(&mut self, f: impl FnOnce(&mut Graph) -> Result<(), SceneError>) -> bool {
        let mut g = self.scene.fractal.to_graph();
        match f(&mut g).and_then(|_| g.validate_structure()) {
            Ok(()) => {
                if !matches!(&self.scene.fractal, Fractal::Graph(cur) if *cur == g) {
                    self.edit(|s| s.fractal = Fractal::Graph(g));
                }
                self.form.errors.clear();
                true
            }
            Err(e) => {
                let node = match &e {
                    SceneError::Node { id, .. } => Some(*id),
                    _ => self.form.selected.first().copied(),
                };
                let msg = e.to_string();
                self.form.errors = node.map(|n| vec![(n, msg.clone())]).unwrap_or_default();
                self.banner = Some(Banner::error(msg));
                false
            }
        }
    }

    /// Starts coalescing edits into one undo step. Idempotent within a drag.
    pub fn begin_drag(&mut self) {
        if !self.drag_in_progress {
            self.drag_in_progress = true;
            self.drag_has_snapshot = false;
        }
    }

    /// Closes the gesture. One that left the scene as it found it (a card
    /// nudged inside its frame and restacked back) takes its step back, with
    /// the modified flag and the redo history it found.
    pub fn end_drag(&mut self) {
        if let Some((dirty, redo)) = self.drag_restore.take() {
            if self.drag_has_snapshot && self.undo.last() == Some(&self.scene) {
                self.undo.pop();
                self.dirty_since_save = dirty;
                self.redo = redo;
            }
        }
        self.drag_in_progress = false;
        self.drag_has_snapshot = false;
    }

    /// Restores the previous step. Inside a drag, the edits that follow start
    /// a new step instead of vanishing into the undone one.
    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo.pop() else {
            return false;
        };
        self.preview_dirty |= render_differs(&prev, &self.scene);
        self.redo.push(std::mem::replace(&mut self.scene, prev));
        self.drag_has_snapshot = false;
        self.drag_restore = None;
        self.dirty_since_save = true;
        self.form.errors.clear();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.preview_dirty |= render_differs(&next, &self.scene);
        self.undo.push(std::mem::replace(&mut self.scene, next));
        self.drag_has_snapshot = false;
        self.drag_restore = None;
        self.dirty_since_save = true;
        self.form.errors.clear();
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// True once after any change: the preview must be rebuilt.
    pub fn take_preview_dirty(&mut self) -> bool {
        std::mem::take(&mut self.preview_dirty)
    }

    pub fn has_unsaved_workspaces(&self) -> bool {
        self.dirty_since_save || self.workspaces.iter().flatten().any(|w| w.dirty)
    }

    pub fn workshop_modified(&self, dimension: syfrack_core::Dimension) -> bool {
        if self.scene.dimension == dimension {
            self.dirty_since_save
        } else {
            self.workspaces[if dimension == syfrack_core::Dimension::TwoD {
                0
            } else {
                1
            }]
            .as_ref()
            .is_some_and(|w| w.dirty)
        }
    }

    pub fn is_modified(&self) -> bool {
        self.dirty_since_save
    }

    pub fn load(&mut self, path: &Path) -> bool {
        match Scene::load(path) {
            Ok(scene) => self.load_scene(scene, Some(path.to_path_buf())),
            Err(e) => {
                self.banner = Some(Banner::error(format!("{} : {e}", path.display())));
                false
            }
        }
    }

    /// Replaces the whole scene (a file, a gallery image): history cleared,
    /// preview marked for rebuild. An invalid scene is refused with a banner.
    pub fn load_scene(&mut self, scene: Scene, source: Option<PathBuf>) -> bool {
        let label = source.as_ref().map_or_else(
            || syfrack_i18n::tr!("scène").to_string(),
            |p| p.display().to_string(),
        );
        // An incomplete form graph opens: the editor shows its unwired ports.
        if let Err(e) = scene.validate_editable() {
            self.banner = Some(Banner::error(format!("{label} : {e}")));
            return false;
        }
        self.workspaces = [None, None];
        self.scene = scene;
        self.path = source;
        self.undo.clear();
        self.redo.clear();
        self.end_drag();
        self.form.selected.clear();
        self.form.errors.clear();
        self.dirty_since_save = false;
        self.preview_dirty = true;
        self.banner = Some(Banner::info(syfrack_i18n::tr_format!(
            "chargé {label}",
            label = label
        )));
        true
    }

    pub fn save(&mut self) -> bool {
        match self.path.clone() {
            Some(p) => self.save_as(&p),
            None => {
                self.banner = Some(Banner::error(syfrack_i18n::tr!(
                    "aucun fichier : utiliser Enregistrer sous"
                )));
                false
            }
        }
    }

    pub fn save_as(&mut self, path: &Path) -> bool {
        match self.scene.save(path) {
            Ok(()) => {
                self.path = Some(path.to_path_buf());
                self.dirty_since_save = false;
                self.banner = Some(Banner::info(syfrack_i18n::tr_format!(
                    "enregistré {}",
                    path.display()
                )));
                true
            }
            Err(e) => {
                self.banner = Some(Banner::error(format!("{} : {e}", path.display())));
                false
            }
        }
    }
}

/// The scene as the renderer sees it: the form's card positions and names
/// cleared (they are neither in the topology fingerprint nor in the
/// parameter buffer; a name only reaches WGSL comments).
fn render_view(scene: &Scene) -> Scene {
    let mut s = scene.clone();
    let Fractal::Graph(g) = &mut s.fractal;
    for n in &mut g.nodes {
        n.pos = [0.0; 2];
        n.name.clear();
    }
    s
}

/// Whether the preview must restart to go from `a` to `b`.
fn render_differs(a: &Scene, b: &Scene) -> bool {
    a != b && render_view(a) != render_view(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use syfrack_core::{NodeKind, Scene, SceneError};

    #[test]
    fn edit_pushes_undo_and_clears_redo() {
        let mut s = AppState::new(Scene::example(), None);
        assert!(!s.can_undo());
        s.set_param("look.exposure_ev", 1.0);
        assert!(s.can_undo() && !s.can_redo());
        assert!(s.take_preview_dirty());
        assert!(!s.take_preview_dirty());
        assert!(s.undo());
        assert_eq!(s.scene.look.exposure_ev, 0.0);
        assert!(s.can_redo());
        assert!(s.redo());
        assert_eq!(s.scene.look.exposure_ev, 1.0);
        s.set_param("look.exposure_ev", 2.0);
        assert!(!s.can_redo(), "a new edit clears redo");
    }

    #[test]
    fn invalid_set_param_is_refused_with_a_banner_and_no_undo_step() {
        let mut s = AppState::new(Scene::example(), None);
        assert!(!s.set_param("look.exposure_ev", 99.0));
        assert!(s.banner.as_ref().map(|b| b.error).unwrap_or(false));
        assert!(!s.can_undo());
        assert!(!s.take_preview_dirty());
    }

    #[test]
    fn drag_coalesces_into_one_undo_step() {
        let mut s = AppState::new(Scene::example(), None);
        s.begin_drag();
        for i in 1..=20 {
            s.set_param("look.exposure_ev", i as f32 * 0.1);
        }
        s.end_drag();
        assert!(s.undo());
        assert_eq!(
            s.scene.look.exposure_ev, 0.0,
            "one undo reverts the whole drag"
        );
        assert!(!s.can_undo());
    }

    #[test]
    fn begin_drag_is_idempotent_within_a_drag() {
        let mut s = AppState::new(Scene::example(), None);
        s.begin_drag();
        s.edit(|sc| sc.look.exposure_ev = 0.5);
        s.begin_drag();
        s.edit(|sc| sc.look.exposure_ev = 1.0);
        s.end_drag();
        s.edit(|sc| sc.look.exposure_ev = 1.5);
        assert!(s.undo());
        assert_eq!(s.scene.look.exposure_ev, 1.0);
        assert!(s.undo());
        assert_eq!(s.scene.look.exposure_ev, 0.0, "the drag is one step");
        assert!(!s.can_undo());
    }

    #[test]
    fn an_undo_inside_a_drag_starts_a_new_step_for_the_rest_of_it() {
        let mut s = AppState::new(Scene::example(), None);
        s.begin_drag();
        s.set_param("look.exposure_ev", 1.0);
        assert!(s.undo());
        s.set_param("look.exposure_ev", 2.0);
        s.end_drag();
        assert!(s.can_undo(), "the edit after the undo is recorded");
        assert!(s.undo());
        assert_eq!(s.scene.look.exposure_ev, 0.0);
    }

    #[test]
    fn random_edit_sequences_undo_back_to_the_start() {
        let mut s = AppState::new(Scene::example(), None);
        let start = s.scene.clone();
        let params = start.params();
        let mut seed = 1u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / 16_777_216.0
        };
        for _ in 0..200 {
            let p = &params[(rnd() * params.len() as f32) as usize % params.len()];
            let v = p.bounds.min + rnd() * (p.bounds.max - p.bounds.min);
            s.set_param(&p.path, v);
        }
        while s.undo() {}
        assert_eq!(s.scene, start);
        while s.redo() {}
        assert_ne!(s.scene, start);
    }

    #[test]
    fn load_scene_replaces_the_scene_and_resets_the_history() {
        let mut s = AppState::new(Scene::example(), Some("old.ron".into()));
        s.set_param("look.exposure_ev", 1.0);
        assert!(s.take_preview_dirty());
        let mut other = Scene::example();
        other.seed = 99;
        assert!(s.load_scene(other, None));
        assert_eq!(s.scene.seed, 99);
        assert!(s.path.is_none());
        assert!(!s.can_undo() && !s.can_redo() && !s.is_modified());
        assert!(s.take_preview_dirty(), "the preview must be rebuilt");
        assert!(!s.banner.as_ref().unwrap().error);
    }

    #[test]
    fn load_scene_refuses_an_invalid_scene_and_keeps_the_current_one() {
        let mut s = AppState::new(Scene::example(), None);
        let mut bad = Scene::example();
        bad.look.exposure_ev = 99.0;
        assert!(!s.load_scene(bad, None));
        assert_eq!(s.scene.look.exposure_ev, 0.0);
        assert!(s.banner.as_ref().unwrap().error);
        assert!(!s.take_preview_dirty());
    }

    #[test]
    fn edit_graph_refuses_an_invalid_result_with_a_banner_and_keeps_the_scene() {
        let mut st = AppState::new(Scene::example_for("mandelbox").unwrap(), None);
        let before = st.scene.clone();
        // A zero scale breaks the structure.
        let ok = st.edit_graph(|g| {
            if let NodeKind::Scale { factor } = &mut g.node_mut(5).unwrap().kind {
                *factor = 0.0;
            }
            Ok(())
        });
        assert!(!ok);
        assert_eq!(st.scene, before);
        assert!(st.banner.as_ref().is_some_and(|b| b.error));
        assert!(!st.form.errors.is_empty());
        assert!(!st.can_undo());
        let ok = st.edit_graph(|g| {
            g.nodes[2].enabled = false;
            Ok(())
        });
        assert!(ok && st.can_undo() && st.form.errors.is_empty());
    }

    #[test]
    fn edit_graph_accepts_an_incomplete_graph() {
        let mut st = AppState::new(Scene::example_for("mandelbox").unwrap(), None);
        assert!(st.edit_graph(|g| {
            g.links.clear();
            Ok(())
        }));
        assert!(st.can_undo() && st.form.errors.is_empty());
        assert!(st.scene.validate_editable().is_ok());
        assert!(st.scene.validate().is_err(), "not renderable yet");
    }

    #[test]
    fn a_drag_that_ends_where_it_started_leaves_no_step() {
        let mut s = AppState::new(Scene::example(), None);
        s.set_param("look.exposure_ev", 1.0);
        assert!(s.undo(), "a redo step to keep");
        assert!(s.save_as(
            &std::env::temp_dir().join(format!("syfrack-nudge-{}.ron", std::process::id()))
        ));
        let seed = s.scene.seed;
        s.begin_drag();
        s.edit(|sc| sc.seed = seed + 1);
        s.edit(|sc| sc.seed = seed);
        s.end_drag();
        assert!(!s.can_undo(), "no empty step");
        assert!(!s.is_modified(), "not modified");
        assert!(s.can_redo(), "the redo history survives");
        // A drag that changes something still records its step.
        s.begin_drag();
        s.edit(|sc| sc.seed = seed + 2);
        s.end_drag();
        assert!(s.can_undo() && s.is_modified());
        if let Some(p) = s.path.clone() {
            std::fs::remove_file(p).ok();
        }
    }

    #[test]
    fn a_no_op_graph_edit_adds_no_step() {
        let mut scene = Scene::example_for("mandelbox").unwrap();
        scene.fractal = Fractal::Graph(scene.fractal.to_graph());
        let mut st = AppState::new(scene, None);
        assert!(st.edit_graph(|_| Ok(())));
        assert!(!st.can_undo() && !st.is_modified() && !st.take_preview_dirty());
    }

    #[test]
    fn moving_or_renaming_a_card_keeps_the_preview_but_a_value_restarts_it() {
        let mut st = AppState::new(Scene::example_for("mandelbox").unwrap(), None);
        // A card drag: one position edit per frame.
        st.begin_drag();
        for k in 1..=5 {
            assert!(st.edit_graph(|g| {
                g.node_mut(5).unwrap().pos = [10.0 * k as f32, 20.0];
                Ok(())
            }));
            assert!(!st.take_preview_dirty(), "a move is not a render input");
        }
        st.end_drag();
        assert!(st.can_undo() && st.is_modified(), "still an undoable edit");
        assert!(st.edit_graph(|g| {
            g.node_mut(5).unwrap().name = "Zoom".into();
            Ok(())
        }));
        assert!(!st.take_preview_dirty(), "a name is not a render input");
        // Undoing a move does not restart either.
        assert!(st.undo());
        assert!(!st.take_preview_dirty());
        // A slider: through set_param and through a graph edit.
        assert!(st.set_param("fractal.nodes[5].factor", 2.5));
        assert!(st.take_preview_dirty());
        assert!(st.edit_graph(|g| {
            if let NodeKind::Scale { factor } = &mut g.node_mut(5).unwrap().kind {
                *factor = 2.25;
            }
            Ok(())
        }));
        assert!(st.take_preview_dirty());
        // Any other scene change still restarts.
        st.edit(|s| s.seed += 1);
        assert!(st.take_preview_dirty());
    }

    #[test]
    fn errors_last_until_the_next_change() {
        let mut scene = Scene::example_for("mandelbox").unwrap();
        scene.fractal = Fractal::Graph(scene.fractal.to_graph());
        let mut st = AppState::new(scene, None);
        let refuse = |st: &mut AppState| {
            assert!(!st.edit_graph(|_| Err(SceneError::Node {
                id: 3,
                msg: "x".into()
            })));
            assert_eq!(st.form.errors.first().map(|e| e.0), Some(3));
        };
        refuse(&mut st);
        assert!(st.set_param("fractal.nodes[5].factor", 2.5));
        assert!(st.form.errors.is_empty(), "set_param clears");
        refuse(&mut st);
        assert!(st.undo());
        assert!(st.form.errors.is_empty(), "undo clears");
        refuse(&mut st);
        assert!(st.redo());
        assert!(st.form.errors.is_empty(), "redo clears");
        refuse(&mut st);
        st.edit(|s| s.seed = 3);
        assert!(st.form.errors.is_empty(), "edit clears");
    }

    #[test]
    fn an_incomplete_graph_scene_loads() {
        let mut st = AppState::new(Scene::example(), None);
        let mut scene = Scene::example();
        let mut g = syfrack_core::Graph::new_minimal();
        g.links.clear();
        scene.fractal = Fractal::Graph(g);
        assert!(st.load_scene(scene, None));
    }

    #[test]
    fn set_param_refuses_a_value_that_breaks_a_graph_rule() {
        let mut scene = Scene::example_for("mandelbox").unwrap();
        scene.fractal = Fractal::Graph(scene.fractal.to_graph());
        let mut st = AppState::new(scene, None);
        assert!(!st.set_param("fractal.nodes[5].factor", 0.0));
        assert!(!st.can_undo());
        assert!(st.banner.as_ref().is_some_and(|b| b.error));
        assert!(st.set_param("fractal.nodes[5].factor", 2.5));
    }

    #[test]
    fn edit_graph_marks_the_node_named_in_the_message() {
        let mut st = AppState::new(Scene::example_for("mandelbox").unwrap(), None);
        st.form.selected = vec![7];
        let ok = st.edit_graph(|g| {
            if let NodeKind::Scale { factor } = &mut g.node_mut(5).unwrap().kind {
                *factor = 99.0;
            }
            Ok(())
        });
        assert!(!ok);
        assert_eq!(st.form.errors.first().map(|e| e.0), Some(5));
        let ok = st.edit_graph(|_| Err(SceneError::Graph("refusé".into())));
        assert!(!ok);
        assert_eq!(
            st.form.errors.first().map(|e| e.0),
            Some(7),
            "else the selection"
        );
    }

    #[test]
    fn save_and_load_roundtrip_through_a_temp_file() {
        let dir = std::env::temp_dir().join(format!("syfrack-state-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.ron");
        let mut s = AppState::new(Scene::example(), None);
        s.set_param("look.exposure_ev", 0.5);
        assert!(s.is_modified());
        assert!(s.save_as(&path));
        assert!(!s.is_modified());
        let mut t = AppState::new(Scene::example(), None);
        assert!(t.load(&path));
        assert_eq!(t.scene.look.exposure_ev, 0.5);
        assert!(!t.load(&dir.join("missing.ron")));
        assert!(t.banner.as_ref().unwrap().error);
        std::fs::remove_dir_all(dir).ok();
    }
}
