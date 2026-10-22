//! One canvas for form states, analytic patterns, orbit density and natural
//! fields. Incomplete edits are saved as a draft; only a valid graph is sent
//! to the common live/export renderer.
use super::{form_panel::FormPanel, theme, widgets};
use crate::{canvas::CanvasView, graph_edit, state::AppState};
use egui::{Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
use syfrack_core::{scene::phenomena::*, *};

mod workflow;
mod workspace;
use workflow::{active_nodes, socket_type, Socket};
type ThumbnailTask = Arc<Mutex<mpsc::Receiver<Result<Vec<(u64, egui::ColorImage)>, String>>>>;
type FreezeTask = Arc<Mutex<mpsc::Receiver<Result<(Scene, Scene), String>>>>;

const WIDTH: f32 = 248.0;
const HEADER: f32 = 52.0;
const ROW: f32 = 24.0;

#[derive(Default)]
pub struct Editor {
    query: String,
    wire: Option<Socket>,
    drag: Option<(Pos2, Vec<(u32, [f32; 2])>)>,
    message: Option<String>,
    source: String,
    source_id: Option<u32>,
    source_kind: Option<CreationKind>,
    framed: bool,
    document_generation: u64,
    task: Option<FreezeTask>,
    cancel: Arc<AtomicBool>,
    source_settings: Option<Phenomena>,
    time: f32,
    atlas: VariantAtlas,
    atlas_path: String,
    gesture_end: bool,
    variant_paths: String,
    variant_count: u32,
    variant_strength: f32,
    variant_seed: u64,
    thumbnails: Vec<(u64, egui::TextureHandle)>,
    thumbnail_task: Option<ThumbnailTask>,
    compare: [Option<u64>; 2],
    focus_branch: bool,
    library_hidden: bool,
    library_add: bool,
    connections: (u64, BTreeMap<Socket, Vec<Socket>>),
    addable: BTreeMap<(Socket, String, String), bool>,
    canvas_size: Vec2,
}
impl Drop for Editor {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
fn label(text: &str) -> String {
    syfrack_i18n::text(text).to_string()
}
fn color(t: SocketType) -> Color32 {
    match t {
        SocketType::Point => Color32::from_rgb(104, 195, 151),
        SocketType::Distance => Color32::from_rgb(109, 155, 214),
        SocketType::Scalar => Color32::from_rgb(231, 184, 105),
        SocketType::Vector => Color32::from_rgb(97, 203, 214),
        SocketType::Complex => Color32::from_rgb(185, 139, 227),
        SocketType::Texture => Color32::from_rgb(217, 139, 171),
        SocketType::State => Color32::from_rgb(218, 132, 233),
    }
}
fn height(n: &CreationNode) -> f32 {
    HEADER + ROW * (n.kind.input_labels().len() + n.kind.output_labels().len()) as f32 + 28.0
}
fn port(n: &CreationNode, index: u32, output: bool) -> Pos2 {
    Pos2::new(
        n.pos[0] + if output { WIDTH } else { 0.0 },
        n.pos[1]
            + HEADER
            + ROW
                * (index as f32
                    + 0.5
                    + if output {
                        n.kind.input_labels().len() as f32
                    } else {
                        0.0
                    }),
    )
}
fn graph(state: &AppState) -> CreationGraph {
    state
        .scene
        .creation_draft
        .clone()
        .or_else(|| state.scene.creation.clone())
        .unwrap_or_else(|| CreationGraph::from_scene(&state.scene))
}
pub fn commit(state: &mut AppState, g: CreationGraph) -> Result<(), String> {
    g.types().map_err(|e| e.to_string())?;
    match g.compile(&state.scene) {
        Ok(mut compiled) => {
            compiled.creation = Some(g);
            compiled.creation_draft = None;
            state.edit(|s| *s = compiled);
            Ok(())
        }
        Err(e) => {
            let text = e.to_string();
            state.edit(|s| s.creation_draft = Some(g));
            Err(text)
        }
    }
}
/// Reuse the graph workshop's complete pattern recipe from the simple view.
pub(super) fn add_pattern(state: &mut AppState) -> Result<(), String> {
    let mut g = graph(state);
    workflow::compose(&mut g, 0).map_err(|e| e.to_string())?;
    let mut base = state.scene.clone();
    base.dimension = Dimension::TwoD;
    let mut compiled = g.compile(&base).map_err(|e| e.to_string())?;
    compiled.creation = Some(g);
    compiled.creation_draft = None;
    state.edit(|s| *s = compiled);
    Ok(())
}
pub fn show(ui: &mut egui::Ui, state: &mut AppState, form: &mut FormPanel, editor: &mut Editor) {
    if editor.document_generation != state.document_generation {
        editor.document_generation = state.document_generation;
        editor.framed = false;
        editor.source_id = None;
        editor.wire = None;
        editor.drag = None;
        editor.message = None;
    }
    if let Some(task) = editor.thumbnail_task.clone() {
        let result = task.lock().unwrap_or_else(|e| e.into_inner()).try_recv();
        match result {
            Ok(result) => {
                editor.thumbnail_task = None;
                match result {
                    Ok(images) => {
                        editor.thumbnails = images
                            .into_iter()
                            .map(|(id, image)| {
                                (
                                    id,
                                    ui.ctx().load_texture(
                                        format!("creation-variant-{id}"),
                                        image,
                                        egui::TextureOptions::LINEAR,
                                    ),
                                )
                            })
                            .collect();
                    }
                    Err(e) => editor.message = Some(e),
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => editor.thumbnail_task = None,
            Err(mpsc::TryRecvError::Empty) => ui
                .ctx()
                .request_repaint_after(std::time::Duration::from_millis(50)),
        }
    }
    if let Some(task) = editor.task.clone() {
        let result = task.lock().unwrap_or_else(|e| e.into_inner()).try_recv();
        match result {
            Ok(Ok((source, frozen))) => {
                editor.task = None;
                if source == state.scene {
                    state.edit(|s| *s = frozen);
                    editor.message = Some(label("Sources figées dans le graphe."));
                } else {
                    editor.message = Some(label(
                        "La scène a changé : le résultat précédent a été écarté.",
                    ));
                }
            }
            Ok(Err(e)) => {
                editor.task = None;
                editor.message = Some(e);
            }
            Err(mpsc::TryRecvError::Disconnected) => editor.task = None,
            Err(mpsc::TryRecvError::Empty) => {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(50));
            }
        }
    }
    let mut g = graph(state);
    state.form.selected.retain(|id| g.node(*id).is_some());
    let before = g.clone();
    egui::Panel::top("creation-toolbar")
        .show(ui, |ui| workspace::toolbar(ui, state, &mut g, form, editor));
    let compact = ui.max_rect().width() < 1250.0;
    if !editor.library_hidden {
        egui::Panel::left("creation-library")
            .default_size(if compact { 210.0 } else { 238.0 })
            .min_size(180.0)
            .max_size(320.0)
            .show(ui, |ui| workspace::library(ui, state, &mut g, form, editor));
    }
    egui::Panel::right("creation-inspector").default_size(if compact { 272.0 } else { 310.0 }).min_size(240.0).max_size(480.0).show(ui,|ui|{
        egui::Panel::bottom("creation-preview").resizable(true).default_size(250.0).show(ui,|ui|form.preview(ui));
        egui::ScrollArea::vertical().id_salt("creation-properties").show(ui,|ui|{
            if let Some(id)=state.form.selected.last().copied(){
                workspace::connections(ui,state,&mut g,id,editor);
                inspector(ui,state,&mut g,id,editor);
            }
            else {ui.heading(label("Composer"));ui.label(label("Reliez les prises de même couleur. Un motif, une flamme et une onde peuvent alimenter les mêmes opérations."));
                ui.label(label("La sortie Scène rassemble couleur, volume et géométrie. Les réglages optiques et le domaine se trouvent sur ce nœud."));}
        });
    });
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(theme::BG))
        .show(ui, |ui| {
            let rect = ui.available_rect_before_wrap();
            form.rect = rect;
            workspace::canvas(ui, state, &mut g, editor, rect);
        });
    if g != before {
        editor.message = commit(state, g).err();
    }
    if editor.gesture_end {
        state.end_drag();
        editor.gesture_end = false;
    }
    if ui.input(|i| i.pointer.any_released()) && editor.drag.is_some() {
        editor.drag = None;
        state.end_drag();
    }
}
fn catalogue() -> Vec<(&'static str, Vec<(String, CreationKind)>)> {
    let field = |name: &str, op| (name.to_string(), CreationKind::Field(op));
    let mut t = FractalTexture::default();
    t.planar = Some(PlanarFractal::default());
    t.scale = 1.0;
    t.octaves = 1;
    let mut result = vec![
        (
            "Sources",
            vec![
                field("Position", FieldOp::Position),
                ("Motif fractal".into(), CreationKind::Texture(Box::new(t))),
                (
                    "Flamme fractale".into(),
                    CreationKind::Flame(FlameField::default()),
                ),
                field(
                    "Onde plane",
                    FieldOp::PlaneWave {
                        wavevector: [4.0, 2.0, 0.0].into(),
                        frequency: 1.0,
                        phase: 0.0,
                    },
                ),
                field(
                    "Onde gaussienne",
                    FieldOp::GaussianWave {
                        center: [0.0; 3].into(),
                        width: 1.0,
                        wavevector: [4.0, 2.0, 0.0].into(),
                        frequency: 1.0,
                    },
                ),
                field("Valeur", FieldOp::Scalar { value: 0.5 }),
                field(
                    "Vecteur / couleur",
                    FieldOp::Vector {
                        value: [0.2, 0.6, 1.0].into(),
                    },
                ),
                field("Complexe", FieldOp::Complex { value: [1.0, 0.0] }),
                field("Temps", FieldOp::Time),
            ],
        ),
        (
            "Combiner et colorer",
            vec![
                field("Somme", FieldOp::Add),
                field("Différence", FieldOp::Subtract),
                field("Produit", FieldOp::Multiply),
                field("Multiplier par une valeur", FieldOp::Scale),
                ("Mélanger les couleurs".into(), CreationKind::Mix),
                ("Palette".into(), CreationKind::Palette(Ramp::default())),
                field("Luminance", FieldOp::Luminance),
                field("Borner", FieldOp::Clamp { min: 0.0, max: 1.0 }),
                field("Puissance", FieldOp::Power { exponent: 2.0 }),
                field("Sinus", FieldOp::Sine),
                field("Cosinus", FieldOp::Cosine),
                field("Exponentielle", FieldOp::Exponential),
                field("Valeur absolue", FieldOp::Absolute),
            ],
        ),
        (
            "Champs et analyse",
            vec![
                field("Intensité", FieldOp::Intensity),
                field("Phase", FieldOp::Phase),
                field("Longueur", FieldOp::Length),
                field("Normaliser", FieldOp::Normalize),
                field("Produit scalaire", FieldOp::Dot),
                field("Produit vectoriel", FieldOp::Cross),
                field("Gradient", FieldOp::Gradient),
                field("Divergence", FieldOp::Divergence),
                field("Rotationnel", FieldOp::Curl),
                field("Construire un complexe", FieldOp::ComplexFromParts),
                field("Produit complexe", FieldOp::ComplexProduct),
                field("Puissance complexe", FieldOp::ComplexPower { power: 2 }),
                field("Distance de la forme", FieldOp::GeometryDistance),
                field("Piège de la forme", FieldOp::GeometryTrap { slot: 0 }),
                field(
                    "Distance sphérique",
                    FieldOp::SphereDistance {
                        center: [0.0; 3].into(),
                        radius: 1.0,
                    },
                ),
                field(
                    "Julia quaternionique",
                    FieldOp::QuaternionJulia {
                        constant: [0.2, -0.3, 0.1, -0.4],
                        slice: 0.35,
                        iterations: 24,
                        bailout: 8.0,
                    },
                ),
                field(
                    "Möbius",
                    FieldOp::Mobius {
                        a: [1.0, 0.0],
                        b: [0.0, 0.0],
                        c: [0.1, 0.0],
                        d: [1.0, 0.0],
                    },
                ),
                (
                    "Échantillonner l’état".into(),
                    CreationKind::SampleState { channel: 0 },
                ),
            ],
        ),
    ];
    let models = [
        ("Morphogenèse", Simulation::default().model),
        (
            "Condensat quantique",
            SimulationModel::GrossPitaevskii {
                dispersion: 0.03,
                interaction: 0.5,
                damping: 0.0,
            },
        ),
        (
            "Croissance diélectrique",
            SimulationModel::DielectricGrowth {
                eta: 2.5,
                relaxation_steps: 48,
                branches_per_step: 1,
            },
        ),
        (
            "Groupe de Schottky",
            SimulationModel::Schottky {
                pairs: vec![CirclePair {
                    source: [-1.2, 0.0],
                    target: [1.2, 0.0],
                    radius: 0.5,
                    twist: 15.0,
                }],
                points: 100000,
                burn_in: 64,
            },
        ),
        (
            "Résonance",
            SimulationModel::Resonance {
                mode: 2,
                iterations: 2048,
            },
        ),
    ];
    result.push((
        "États naturels",
        models
            .into_iter()
            .map(|(name, model)| {
                (
                    name.into(),
                    CreationKind::Simulation(Simulation {
                        name: name.into(),
                        model,
                        steps: 32,
                        dt: 0.01,
                        ..Default::default()
                    }),
                )
            })
            .collect(),
    ));
    result.push((
        "Géométrie",
        graph_edit::ALL_KINDS
            .iter()
            .filter(|k| !matches!(k, NodeKind::Output))
            .map(|k| (k.label().to_string(), CreationKind::Form(k.clone())))
            .collect(),
    ));
    result
}
fn add_connected(
    g: &mut CreationGraph,
    kind: CreationKind,
    state: &mut AppState,
    canvas: Rect,
    anchor: Option<Socket>,
) {
    let origin = state.form.view.to_world(canvas.min, canvas.center());
    let explicit = anchor.is_some();
    let selected = state.form.selected.last().copied();
    let anchor = anchor.or_else(|| selected.map(|id| Socket::new(id, 0, true)));
    let position = g
        .nodes
        .iter()
        .find(|n| matches!(n.kind, CreationKind::Field(FieldOp::Position)))
        .map(|n| n.id);
    let geometric = g
        .nodes
        .iter()
        .find(|n| matches!(n.kind, CreationKind::Form(NodeKind::Input)))
        .map(|n| n.id);
    let auto_position = matches!(
        kind,
        CreationKind::Texture(_)
            | CreationKind::Pattern { .. }
            | CreationKind::Flame(_)
            | CreationKind::Simulation(_)
            | CreationKind::Field(
                FieldOp::PlaneWave { .. }
                    | FieldOp::GaussianWave { .. }
                    | FieldOp::GeometryDistance
                    | FieldOp::GeometryTrap { .. }
                    | FieldOp::SphereDistance { .. }
                    | FieldOp::QuaternionJulia { .. }
            )
    );
    let is_form = matches!(kind, CreationKind::Form(_));
    let sample = matches!(kind, CreationKind::SampleState { .. });
    let mut pos = [origin.x - WIDTH * 0.5, origin.y - 60.0];
    if let Some(s) = anchor {
        if let Some(n) = g.node(s.node) {
            pos = [
                n.pos[0]
                    + if s.output {
                        WIDTH + 88.0
                    } else {
                        -WIDTH - 88.0
                    },
                n.pos[1],
            ];
        }
    }
    // New cards stay clear of existing cards, including variable-height outputs.
    while g.nodes.iter().any(|n| {
        Rect::from_min_size(Pos2::from(n.pos), Vec2::new(WIDTH, height(n)))
            .expand(16.0)
            .intersects(Rect::from_min_size(
                Pos2::from(pos),
                Vec2::new(
                    WIDTH,
                    HEADER
                        + ROW * (kind.input_labels().len() + kind.output_labels().len()) as f32
                        + 28.0,
                ),
            ))
    }) {
        pos[1] += 48.0;
    }
    let id = g.add(kind, pos);
    if let Some(s) = anchor {
        workflow::attach(g, id, s, explicit);
    }
    if auto_position && g.input(id, 0).is_none() {
        let p = position.unwrap_or_else(|| {
            g.add(
                CreationKind::Field(FieldOp::Position),
                [pos[0] - WIDTH - 88.0, pos[1]],
            )
        });
        let _ = g.connect((p, 0), (id, 0));
    } else if sample && g.input(id, 1).is_none() {
        let p = position.unwrap_or_else(|| {
            g.add(
                CreationKind::Field(FieldOp::Position),
                [pos[0] - WIDTH - 88.0, pos[1]],
            )
        });
        let _ = g.connect((p, 0), (id, 1));
    } else if is_form && g.input(id, 0).is_none() {
        if let Some(p) = geometric {
            let _ = g.connect((p, 0), (id, 0));
        }
    }
    state.form.selected = vec![id];
}
fn fit(state: &mut AppState, g: &CreationGraph, size: Vec2) {
    if g.nodes.is_empty() {
        return;
    }
    let bounds = g
        .nodes
        .iter()
        .map(|n| Rect::from_min_size(Pos2::from(n.pos), Vec2::new(WIDTH, height(n))))
        .reduce(Rect::union)
        .unwrap();
    let size = Vec2::new(size.x.max(120.0), size.y.max(120.0));
    let zoom = ((size.x - 80.0) / bounds.width())
        .min((size.y - 80.0) / bounds.height())
        .clamp(0.001, 1.15);
    state.form.view = CanvasView {
        zoom,
        offset: size / (2.0 * zoom) - bounds.center().to_vec2(),
    };
}
fn layout(g: &mut CreationGraph) {
    workflow::arrange(g);
}
fn remove(g: &mut CreationGraph, selected: &mut Vec<u32>) {
    let ids: BTreeSet<_> = selected
        .iter()
        .copied()
        .filter(|id| {
            !g.node(*id)
                .is_some_and(|n| matches!(n.kind, CreationKind::Output))
        })
        .collect();
    g.nodes.retain(|n| !ids.contains(&n.id));
    g.links
        .retain(|l| !ids.contains(&l.from.0) && !ids.contains(&l.to.0));
    for n in &mut g.nodes {
        if let CreationKind::Form(NodeKind::Iterate { body, .. }) = &mut n.kind {
            body.retain(|id| !ids.contains(id));
        }
    }
    selected.retain(|id| !ids.contains(id));
}
fn duplicate(g: &mut CreationGraph, selected: &mut Vec<u32>) {
    let mut wanted: BTreeSet<_> = selected
        .iter()
        .copied()
        .filter(|id| {
            !g.node(*id).is_some_and(|n| {
                matches!(
                    n.kind,
                    CreationKind::Output | CreationKind::Form(NodeKind::Input)
                )
            })
        })
        .collect();
    loop {
        let before = wanted.len();
        for id in wanted.clone() {
            if let Some(CreationNode {
                kind: CreationKind::Form(NodeKind::Iterate { body, .. }),
                ..
            }) = g.node(id)
            {
                wanted.extend(body);
            }
        }
        if before == wanted.len() {
            break;
        }
    }
    let mut next = g.next_id();
    let ids: BTreeMap<_, _> = wanted
        .iter()
        .map(|id| {
            let pair = (*id, next);
            next += 1;
            pair
        })
        .collect();
    let original = g.clone();
    for n in original.nodes.iter().filter(|n| wanted.contains(&n.id)) {
        let mut copy = n.clone();
        copy.id = ids[&n.id];
        copy.pos[0] += 50.0;
        copy.pos[1] += 70.0;
        if let CreationKind::Form(NodeKind::Iterate { body, .. }) = &mut copy.kind {
            for id in body {
                *id = ids[id];
            }
        }
        g.nodes.push(copy);
    }
    for l in original.links {
        if let Some(&to) = ids.get(&l.to.0) {
            g.links.push(Link {
                from: (ids.get(&l.from.0).copied().unwrap_or(l.from.0), l.from.1),
                to: (to, l.to.1),
            });
        }
    }
    *selected = ids.values().copied().collect();
}
fn append_form(g: &mut CreationGraph, form: &Graph, state: &mut AppState) {
    let old: Vec<_> = g.nodes.iter().map(|n| n.id).collect();
    let mut source = CreationGraph::from_scene(&Scene {
        fractal: Fractal::Graph(form.clone()),
        ..Scene::example()
    });
    source
        .nodes
        .retain(|n| matches!(n.kind, CreationKind::Form(_)));
    append(g, &source);
    state.form.selected = g
        .nodes
        .iter()
        .filter(|n| !old.contains(&n.id))
        .map(|n| n.id)
        .collect();
    g.bounding_radius = g.bounding_radius.max(form.bounding_radius_hint);
}
fn append_fields(g: &mut CreationGraph, source: &CreationGraph) {
    let mut source = source.clone();
    source
        .nodes
        .retain(|n| !matches!(n.kind, CreationKind::Form(_) | CreationKind::Output));
    if !g
        .nodes
        .iter()
        .any(|n| matches!(n.kind, CreationKind::Simulation(_)))
    {
        g.settings.domain = source.settings.domain.clone();
    }
    if source.settings.film.is_some() {
        g.settings.film = source.settings.film.clone();
    }
    if source.settings.dispersion.is_some() {
        g.settings.dispersion = source.settings.dispersion.clone();
    }
    if source.settings.lens.is_some() {
        g.settings.lens = source.settings.lens.clone();
    }
    append(g, &source);
}
fn append(g: &mut CreationGraph, source: &CreationGraph) {
    let bottom = g
        .nodes
        .iter()
        .map(|n| n.pos[1] + height(n))
        .fold(0.0f32, f32::max);
    let top = source
        .nodes
        .iter()
        .map(|n| n.pos[1])
        .fold(f32::INFINITY, f32::min);
    let offset = bottom + 180.0 - top;
    let mut next = g.next_id();
    let ids: BTreeMap<_, _> = source
        .nodes
        .iter()
        .map(|n| {
            let pair = (n.id, next);
            next += 1;
            pair
        })
        .collect();
    for n in &source.nodes {
        let mut copy = n.clone();
        copy.id = ids[&n.id];
        copy.pos[0] += 120.0;
        copy.pos[1] += offset;
        if let CreationKind::Form(NodeKind::Iterate { body, .. }) = &mut copy.kind {
            for id in body {
                if let Some(&new) = ids.get(id) {
                    *id = new;
                }
            }
        }
        g.nodes.push(copy);
    }
    for l in &source.links {
        if let (Some(&a), Some(&b)) = (ids.get(&l.from.0), ids.get(&l.to.0)) {
            g.links.push(Link {
                from: (a, l.from.1),
                to: (b, l.to.1),
            });
        }
    }
}
fn inspector(
    ui: &mut egui::Ui,
    state: &mut AppState,
    g: &mut CreationGraph,
    id: u32,
    editor: &mut Editor,
) {
    let candidates: Vec<_> = g
        .nodes
        .iter()
        .filter(|n| matches!(n.kind, CreationKind::Form(_)))
        .map(|n| (n.id, n.name.clone()))
        .collect();
    let Some(n) = g.nodes.iter_mut().find(|n| n.id == id) else {
        return;
    };
    ui.strong(label("Réglages"));
    if matches!(n.kind, CreationKind::Form(_)) {
        ui.checkbox(&mut n.enabled, label("Actif"));
    }
    match &mut n.kind {
        CreationKind::Texture(t) => {
            if let Some(p) = &mut t.planar {
                family(ui, &mut p.family);
            }
            egui::CollapsingHeader::new(label("Palette du motif")).show(ui, |ui| {
                widgets::ramp_editor(ui, &mut t.ramp);
            });
        }
        CreationKind::Pattern { fractal, .. } => family(ui, &mut fractal.family),
        CreationKind::Palette(ramp) => {
            widgets::ramp_editor(ui, ramp);
        }
        CreationKind::Flame(f) => {
            ui.label(label(
                "La couleur et la densité peuvent alimenter des branches différentes.",
            ));
            ui.add(egui::DragValue::new(&mut f.seed).prefix("Seed "));
            egui::CollapsingHeader::new(label("Palette des orbites")).show(ui, |ui| {
                widgets::ramp_editor(ui, &mut f.flame.palette);
            });
            egui::CollapsingHeader::new(label("Transformations de la flamme"))
                .show(ui, |ui| flame_controls(ui, &mut f.flame));
            if f.frozen.is_some() {
                ui.label(label("État figé"));
                if ui.button(label("Libérer l’état figé")).clicked() {
                    f.frozen = None;
                }
            }
        }
        CreationKind::Simulation(s) => {
            ui.label(label(
                "Les entrées Potentiel, Source et Masque peuvent recevoir les autres générateurs.",
            ));
            if ui.button(label("Ajouter un germe")).clicked() {
                s.frozen = None;
                s.seeds.push(SeedBlob::default());
            }
            if s.frozen.is_some() {
                ui.label(label("État figé"));
                if ui.button(label("Libérer l’état figé")).clicked() {
                    s.frozen = None;
                }
            }
            ui.add(egui::DragValue::new(&mut s.seed).prefix("Seed "));
        }
        CreationKind::Form(NodeKind::Iterate { body, .. }) => {
            ui.label(label("Étapes de la boucle, dans leur ordre d’évaluation"));
            let mut remove = None;
            for i in 0..body.len() {
                ui.horizontal(|ui| {
                    let name = candidates
                        .iter()
                        .find(|(id, _)| *id == body[i])
                        .map_or("?", |(_, name)| name.as_str());
                    ui.label(name);
                    if i > 0 && ui.small_button("↑").clicked() {
                        body.swap(i, i - 1);
                    }
                    if i + 1 < body.len() && ui.small_button("↓").clicked() {
                        body.swap(i, i + 1);
                    }
                    if ui.small_button("×").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                body.remove(i);
            }
            ui.menu_button(label("Ajouter une étape"), |ui| {
                for (candidate, name) in &candidates {
                    if *candidate != id && !body.contains(candidate) && ui.button(name).clicked() {
                        body.push(*candidate);
                        ui.close();
                    }
                }
            });
        }
        CreationKind::Output => {
            output_controls(ui, &mut g.settings);
            shared_tools(ui, state, g, editor);
        }
        _ => {}
    }
    // The same parameter visitor drives validation, persistence, CLI and UI.
    let mut temp = state.scene.clone();
    temp.creation = Some(g.clone());
    temp.creation_draft = None;
    let prefix = if matches!(g.node(id).unwrap().kind, CreationKind::Output) {
        "creation.settings.".to_string()
    } else {
        format!("creation.nodes[{id}].")
    };
    let infos: Vec<_> = temp
        .params()
        .into_iter()
        .filter(|p| p.path.starts_with(&prefix))
        .filter(|p| !p.path.ends_with(".supersampling") && !p.path.ends_with(".points_per_sample"))
        .filter(|p| {
            !p.path.contains("palette.stops")
                && !["palette.frequency", "palette.phase", "palette.gamma"]
                    .iter()
                    .any(|suffix| p.path.ends_with(suffix))
        })
        .collect();
    let precision = |p: &ParamInfo| {
        p.path.contains("resolution")
            || p.path.ends_with(".points")
            || p.path.ends_with(".steps")
            || p.path.ends_with(".dt")
    };
    if infos.iter().any(precision) {
        egui::CollapsingHeader::new(label("Précision de la source")).default_open(true).show(ui, |ui| {
            ui.small(label("La finesse de la source et l’échantillonnage du rendu se règlent séparément. Augmenter le rendu ne recrée pas les détails absents d’un atlas."));
            for info in infos.iter().filter(|p| precision(p)) { parameter(ui,state,&mut temp,info,&prefix,editor); }
        });
    }
    for info in infos.iter().filter(|p| !precision(p)) {
        parameter(ui, state, &mut temp, info, &prefix, editor);
    }
    if let Some(next) = temp.creation {
        *g = next;
    }
    egui::CollapsingHeader::new(label("Source du nœud")).show(ui, |ui| {
        let settings = matches!(g.node(id).unwrap().kind, CreationKind::Output);
        if editor.source_id != Some(id)
            || editor.source_kind.as_ref() != Some(&g.node(id).unwrap().kind)
            || (settings && editor.source_settings.as_ref() != Some(&g.settings))
        {
            editor.source = if settings {
                ron::ser::to_string_pretty(&g.settings, Default::default())
            } else {
                ron::ser::to_string_pretty(&g.node(id).unwrap().kind, Default::default())
            }
            .unwrap_or_default();
            editor.source_id = Some(id);
            editor.source_kind = Some(g.node(id).unwrap().kind.clone());
            editor.source_settings = Some(g.settings.clone());
        }
        ui.add(
            egui::TextEdit::multiline(&mut editor.source)
                .code_editor()
                .desired_rows(8),
        );
        if ui.button(label("Appliquer")).clicked() {
            if settings {
                match ron::from_str::<Phenomena>(&editor.source) {
                    Ok(value) => g.settings = value,
                    Err(e) => editor.message = Some(e.to_string()),
                }
            } else {
                match ron::from_str::<CreationKind>(&editor.source) {
                    Ok(kind) => {
                        g.nodes.iter_mut().find(|n| n.id == id).unwrap().kind = kind;
                    }
                    Err(e) => editor.message = Some(e.to_string()),
                }
            }
        }
    });
}
fn parameter(
    ui: &mut egui::Ui,
    state: &mut AppState,
    temp: &mut Scene,
    info: &ParamInfo,
    prefix: &str,
    editor: &mut Editor,
) {
    let mut value = info.value;
    let response = ui
        .horizontal_wrapped(|ui| {
            let name = info
                .path
                .trim_start_matches(prefix)
                .trim_start_matches("texture.")
                .trim_start_matches("planar.");
            ui.label(super::phenomena::parameter_label(name));
            ui.add(
                egui::DragValue::new(&mut value)
                    .speed(if info.path.ends_with(".points") {
                        65_536.0
                    } else if info.path.contains("resolution") {
                        8.0
                    } else if info.path.ends_with(".steps") || info.path.ends_with(".iterations") {
                        1.0
                    } else if info.path.ends_with(".dt") {
                        0.0001
                    } else if info.bounds.max <= 1.0 {
                        0.005
                    } else {
                        0.02
                    })
                    .range(info.bounds.min..=info.bounds.max),
            )
        })
        .inner
        .on_hover_text(&info.path);
    if response.drag_started() {
        state.begin_drag();
    }
    if response.changed() {
        if let Err(e) = temp.set_param(&info.path, value) {
            editor.message = Some(e.to_string());
        }
    }
    if response.drag_stopped() {
        editor.gesture_end = true;
    }
}
fn family(ui: &mut egui::Ui, value: &mut PlanarFamily) {
    egui::ComboBox::from_id_salt("creation-family")
        .selected_text(format!("{value:?}"))
        .show_ui(ui, |ui| {
            for family in [
                PlanarFamily::Julia,
                PlanarFamily::Mandelbrot,
                PlanarFamily::BurningShip,
                PlanarFamily::Newton,
                PlanarFamily::Nova,
                PlanarFamily::Phoenix,
            ] {
                ui.selectable_value(value, family, format!("{family:?}"));
            }
        });
}
fn shared_tools(
    ui: &mut egui::Ui,
    state: &mut AppState,
    g: &mut CreationGraph,
    editor: &mut Editor,
) {
    egui::CollapsingHeader::new(label("Animation")).show(ui, |ui| {
        ui.add(
            egui::DragValue::new(&mut editor.time)
                .speed(0.01)
                .range(0.0..=86400.0)
                .suffix(" s"),
        );
        if ui.button(label("Évaluer à cet instant")).clicked() {
            let mut source = state.scene.clone();
            source.creation = Some(g.clone());
            match Phenomena::scene_at(&source, editor.time) {
                Ok(scene) => *g = scene.creation.unwrap(),
                Err(e) => editor.message = Some(e.to_string()),
            }
        }
        let paths: Vec<_> = state
            .scene
            .params()
            .into_iter()
            .filter(|p| p.path.starts_with("creation.nodes[") && !p.path.contains("palette.stops"))
            .collect();
        ui.menu_button(label("Ajouter une piste"), |ui| {
            egui::ScrollArea::vertical()
                .max_height(300.0)
                .show(ui, |ui| {
                    for p in paths {
                        if ui.button(&p.path).clicked() {
                            if !g.settings.timeline.tracks.iter().any(|t| t.path == p.path) {
                                g.settings.timeline.tracks.push(AnimationTrack {
                                    path: p.path,
                                    interpolation: Interpolation::Smooth,
                                    keys: vec![Keyframe {
                                        time: editor.time,
                                        value: p.value,
                                    }],
                                });
                            }
                            ui.close();
                        }
                    }
                });
        });
        let mut remove = None;
        for (i, t) in g.settings.timeline.tracks.iter_mut().enumerate() {
            ui.push_id(i, |ui| {
                ui.horizontal(|ui| {
                    ui.label(&t.path);
                    if ui.small_button("×").clicked() {
                        remove = Some(i);
                    }
                });
                egui::ComboBox::from_id_salt("interpolation")
                    .selected_text(format!("{:?}", t.interpolation))
                    .show_ui(ui, |ui| {
                        for v in [
                            Interpolation::Step,
                            Interpolation::Linear,
                            Interpolation::Smooth,
                        ] {
                            ui.selectable_value(&mut t.interpolation, v, format!("{v:?}"));
                        }
                    });
                let mut del = None;
                for (j, k) in t.keys.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut k.time)
                                .speed(0.01)
                                .range(0.0..=86400.0)
                                .suffix(" s"),
                        );
                        ui.add(egui::DragValue::new(&mut k.value).speed(0.01));
                        if ui.small_button("×").clicked() {
                            del = Some(j);
                        }
                    });
                }
                if let Some(j) = del {
                    t.keys.remove(j);
                }
                if ui.small_button(label("Ajouter une clé")).clicked() {
                    let value = state
                        .scene
                        .params()
                        .into_iter()
                        .find(|p| p.path == t.path)
                        .map_or(0.0, |p| p.value);
                    t.keys.push(Keyframe {
                        time: editor.time,
                        value,
                    });
                }
                t.keys.sort_by(|a, b| a.time.total_cmp(&b.time));
            });
        }
        if let Some(i) = remove {
            g.settings.timeline.tracks.remove(i);
        }
    });
    egui::CollapsingHeader::new(label("Variantes du graphe")).show(ui, |ui| {
        if ui.button(label("Mémoriser la scène")).clicked() {
            let mut source = state.scene.clone();
            source.creation = Some(g.clone());
            match source.render_scene().and_then(|s| {
                editor
                    .atlas
                    .insert(None, format!("Composition {}", editor.atlas.next_id + 1), s)
            }) {
                Ok(_) => {}
                Err(e) => editor.message = Some(e.to_string()),
            }
        }
        if editor.variant_count == 0 {
            editor.variant_count = 4;
            editor.variant_strength = 0.1;
        }
        ui.menu_button(label("Paramètres à explorer"), |ui| {
            egui::ScrollArea::vertical()
                .max_height(220.0)
                .show(ui, |ui| {
                    for info in state.scene.params().into_iter().filter(|p| {
                        p.path.starts_with("creation.nodes[")
                            && !p.path.contains("palette.stops")
                            && !p.path.ends_with("points_per_sample")
                            && !p.path.ends_with("supersampling")
                    }) {
                        if ui.button(&info.path).clicked() {
                            let mut paths: Vec<_> = editor
                                .variant_paths
                                .split(',')
                                .map(str::trim)
                                .filter(|p| !p.is_empty())
                                .map(str::to_string)
                                .collect();
                            if !paths.contains(&info.path) {
                                paths.push(info.path);
                            }
                            editor.variant_paths = paths.join(",");
                            ui.close();
                        }
                    }
                });
        });
        ui.text_edit_singleline(&mut editor.variant_paths);
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut editor.variant_count).range(1..=16));
            ui.add(
                egui::DragValue::new(&mut editor.variant_strength)
                    .speed(0.005)
                    .range(0.0..=1.0),
            );
            ui.add(egui::DragValue::new(&mut editor.variant_seed).prefix("Seed "));
        });
        if ui.button(label("Créer des variantes")).clicked() {
            let mut source = state.scene.clone();
            source.creation = Some(g.clone());
            let paths: Vec<_> = editor
                .variant_paths
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            let result = (|| -> Result<(), SceneError> {
                if paths.is_empty() {
                    return Err(SceneError::UnknownPath(label(
                        "Choisir un paramètre à explorer.",
                    )));
                }
                let source = source.render_scene()?;
                let variants = VariantAtlas::variations(
                    &source,
                    &paths,
                    editor.variant_count as usize,
                    editor.variant_strength,
                    editor.variant_seed,
                )?;
                let parent = editor.atlas.insert(
                    None,
                    format!("Source {}", editor.atlas.next_id + 1),
                    source,
                )?;
                for (i, s) in variants.into_iter().enumerate() {
                    editor.atlas.insert(
                        Some(parent),
                        format!("Variation {} · {}", parent, i + 1),
                        s,
                    )?;
                }
                Ok(())
            })();
            editor.message = result.err().map(|e| e.to_string());
            editor.variant_seed = editor.variant_seed.wrapping_add(1);
        }
        if ui
            .add_enabled(
                editor.task.is_none() && editor.thumbnail_task.is_none(),
                egui::Button::new(label("Calculer les vignettes GPU")),
            )
            .clicked()
        {
            start_thumbnails(ui, editor);
        }
        ui.horizontal(|ui| {
            for id in editor.compare.into_iter().flatten() {
                if let Some((_, image)) = editor.thumbnails.iter().find(|(key, _)| *key == id) {
                    ui.image((image.id(), Vec2::new(125.0, 70.3)));
                }
            }
        });
        let mut restore = None;
        for b in &editor.atlas.branches {
            ui.horizontal(|ui| {
                if let Some((_, image)) = editor.thumbnails.iter().find(|(id, _)| *id == b.id) {
                    ui.image((image.id(), Vec2::new(64.0, 36.0)));
                }
                if ui.button(&b.label).clicked() {
                    restore = Some(b.scene.clone());
                }
                for (slot, name) in ["A", "B"].into_iter().enumerate() {
                    if ui.small_button(name).clicked() {
                        editor.compare[slot] = Some(b.id);
                    }
                }
            });
        }
        if let Some(s) = restore {
            *g = s
                .creation
                .clone()
                .unwrap_or_else(|| CreationGraph::from_scene(&s));
            state.edit(|scene| *scene = s);
        }
        if editor.atlas_path.is_empty() {
            editor.atlas_path = "renders/creation-atlas.ron".into();
        }
        ui.text_edit_singleline(&mut editor.atlas_path);
        if ui.button(label("Enregistrer l’atlas")).clicked() {
            let result = (|| -> Result<(), String> {
                editor.atlas.validate().map_err(|e| e.to_string())?;
                let text = ron::ser::to_string_pretty(&editor.atlas, Default::default())
                    .map_err(|e| e.to_string())?;
                std::fs::write(&editor.atlas_path, text).map_err(|e| e.to_string())
            })();
            editor.message = result.err();
        }
        if ui.button(label("Charger l’atlas")).clicked() {
            match std::fs::read_to_string(&editor.atlas_path)
                .map_err(|e| e.to_string())
                .and_then(|text| ron::from_str::<VariantAtlas>(&text).map_err(|e| e.to_string()))
                .and_then(|a| {
                    a.validate().map_err(|e| e.to_string())?;
                    Ok(a)
                }) {
                Ok(a) => {
                    editor.cancel.store(true, Ordering::Relaxed);
                    editor.thumbnail_task = None;
                    editor.thumbnails.clear();
                    editor.compare = [None; 2];
                    editor.atlas = a;
                }
                Err(e) => editor.message = Some(e),
            }
        }
    });
}
fn output_controls(ui: &mut egui::Ui, p: &mut Phenomena) {
    ui.label(label("Le même résultat alimente l’aperçu et l’export. Les champs géométriques utilisent le domaine et sa résolution."));
    ui.horizontal(|ui| {
        let mut on = p.film.is_some();
        if ui.checkbox(&mut on, label("Nacre")).changed() {
            p.film = on.then(ThinFilm::default);
        }
        let mut on = p.dispersion.is_some();
        if ui.checkbox(&mut on, "Dispersion").changed() {
            p.dispersion = on.then(ChromaticDispersion::default);
        }
    });
    let mut on = p.lens.is_some();
    if ui.checkbox(&mut on, label("Lentille de Kerr")).changed() {
        p.lens = on.then(RelativisticLens::default);
    }
    egui::ComboBox::from_id_salt("creation-isosurface")
        .selected_text(format!("{:?}", p.bindings.geometry_mode))
        .show_ui(ui, |ui| {
            for mode in [
                FieldGeometry::Replace,
                FieldGeometry::Union,
                FieldGeometry::Intersection,
                FieldGeometry::None,
            ] {
                ui.selectable_value(&mut p.bindings.geometry_mode, mode, format!("{mode:?}"));
            }
        });
    if ui.button(label("Ajouter un guide")).clicked() {
        p.guides.push(ArtisticGuide {
            center: [0.0; 3].into(),
            radius: 1.0,
            strength: 1.0,
            kind: GuideKind::Attractor,
        });
    }
    egui::CollapsingHeader::new(label("Animation et guides · source")).show(ui, |ui| {
        ui.label(label(
            "Les chemins animables sont ceux du graphe creation.nodes et de creation.settings.",
        ));
        // This source lives in the same output inspector, with the same scene
        // validation, rather than in a separate generator window.
        ui.label(format!("{} · {}", p.timeline.tracks.len(), p.guides.len()));
    });
}
fn flame_controls(ui: &mut egui::Ui, flame: &mut Flame) {
    let mut remove = None;
    for (i, t) in flame.transforms.iter_mut().enumerate() {
        egui::CollapsingHeader::new(format!("{} {}", label("Transformation"), i + 1)).show(
            ui,
            |ui| {
                for (j, term) in t.terms.iter_mut().enumerate() {
                    let name = format!("{:?}", term.variation);
                    egui::ComboBox::from_id_salt(("variation", i, j))
                        .selected_text(&name)
                        .show_ui(ui, |ui| {
                            for v in [
                                FlameVariation::Linear,
                                FlameVariation::Sinusoidal,
                                FlameVariation::Spherical,
                                FlameVariation::Swirl,
                                FlameVariation::Horseshoe,
                                FlameVariation::Disc,
                                FlameVariation::Bubble,
                                FlameVariation::Julian {
                                    power: 3,
                                    distance: 1.0,
                                },
                                FlameVariation::Curl { c1: 0.2, c2: 0.05 },
                            ] {
                                let text = format!("{v:?}");
                                ui.selectable_value(&mut term.variation, v, text);
                            }
                        });
                }
                if t.terms.len() < 16 && ui.button(label("Ajouter une variation")).clicked() {
                    t.terms.push(FlameTerm {
                        amount: 0.2,
                        variation: FlameVariation::Swirl,
                    });
                }
                if ui.button(label("Retirer la transformation")).clicked() {
                    remove = Some(i);
                }
            },
        );
    }
    if let Some(i) = remove {
        if flame.transforms.len() > 1 {
            flame.transforms.remove(i);
        }
    }
    if flame.transforms.len() < 16 && ui.button(label("Ajouter une transformation")).clicked() {
        flame.transforms.push(FlameTransform::default());
    }
    let mut final_on = flame.final_transform.is_some();
    if ui
        .checkbox(&mut final_on, label("Transformation finale"))
        .changed()
    {
        flame.final_transform = final_on.then(FlameTransform::default);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mixed_graph_edits_preserve_drafts_and_only_render_active_changes() {
        let mut state = AppState::new(Scene::example_2d(), None);
        state.take_preview_dirty();
        let original = state.scene.clone();
        let mut g = graph(&state);
        let out = g
            .nodes
            .iter()
            .find(|n| matches!(n.kind, CreationKind::Output))
            .unwrap()
            .id;
        let id = g.add(CreationKind::Flame(FlameField::default()), [0.0; 2]);
        commit(&mut state, g.clone()).unwrap();
        assert!(
            !state.take_preview_dirty(),
            "an unused source does not rebuild the preview"
        );
        g.connect((id, 0), (out, 1)).unwrap();
        assert!(commit(&mut state, g.clone()).is_err());
        assert!(state.scene.creation_draft.is_some());
        assert_eq!(state.scene.phenomena, original.phenomena);
        assert!(!state.take_preview_dirty());
        let position = g
            .nodes
            .iter()
            .find(|n| matches!(n.kind, CreationKind::Field(FieldOp::Position)))
            .unwrap()
            .id;
        g.connect((position, 0), (id, 0)).unwrap();
        commit(&mut state, g).unwrap();
        assert!(state.scene.creation_draft.is_none());
        assert!(state.take_preview_dirty());
        assert!(state.scene.canvas.flame.is_none());
        assert!(state
            .scene
            .phenomena
            .as_ref()
            .unwrap()
            .bindings
            .rgb
            .is_some());
        state.undo();
        assert!(state.scene.creation_draft.is_some());
    }
    #[test]
    fn the_real_canvas_displays_one_scene_output_and_all_sources_in_both_languages() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut state = AppState::new(Scene::example_2d(), None);
        let mut form = FormPanel::new();
        for language in [
            syfrack_i18n::Language::French,
            syfrack_i18n::Language::English,
        ] {
            for _ in 0..2 {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            Pos2::ZERO,
                            Vec2::new(1600.0, 900.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        syfrack_i18n::with_language(language, || form.show(ui, &mut state));
                    },
                );
                output.textures_delta.clear();
            }
            let g = graph(&state);
            assert_eq!(
                g.nodes
                    .iter()
                    .filter(|n| matches!(n.kind, CreationKind::Output))
                    .count(),
                1
            );
            for n in g.nodes {
                let rect = ctx.data(|d| d.get_temp::<Rect>(egui::Id::new(("creation-node", n.id))));
                assert!(rect.is_some(), "card {}", n.name);
            }
        }
        assert!(!state.can_undo());
    }
}

fn start_thumbnails(ui: &egui::Ui, editor: &mut Editor) {
    let Some(runtime) = super::phenomena::shared_runtime(ui.ctx()) else {
        editor.message = Some(label("Le moteur GPU doit être disponible pour ce calcul."));
        return;
    };
    let branches = editor
        .atlas
        .branches
        .iter()
        .rev()
        .take(32)
        .cloned()
        .collect::<Vec<_>>();
    let (tx, rx) = mpsc::channel();
    editor.thumbnail_task = Some(Arc::new(Mutex::new(rx)));
    editor.cancel = Arc::new(AtomicBool::new(false));
    let control = syfrack_render::phenomena::PreparationControl {
        paused: runtime.paused,
        cancelled: editor.cancel.clone(),
    };
    std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<Vec<(u64, egui::ColorImage)>> {
            let mut images = vec![];
            for branch in branches {
                control.checkpoint()?;
                runtime
                    .renderer
                    .prepare_phenomena_controlled(&branch.scene, &control)?;
                control.checkpoint()?;
                let mut cfg = syfrack_render::RenderConfig::new(160, 90, 4);
                cfg.max_bounces = 1;
                cfg.max_steps = 128;
                cfg.tile_size = 16;
                let mut session = syfrack_render::RenderSession::new(
                    runtime.renderer.clone(),
                    &branch.scene,
                    &cfg,
                )?;
                while session.progress().samples_min < cfg.samples_per_pixel {
                    control.checkpoint()?;
                    session.step(std::time::Duration::from_millis(12))?;
                }
                let out = session.readback()?;
                let mut look = branch.scene.look.clone();
                look.output = OutputSpace::Srgb;
                let encoded = syfrack_color::develop_and_encode(&out.color, &look, 8);
                let data: Vec<_> = encoded.data.iter().map(|v| *v as u8).collect();
                let image = if encoded.has_alpha() {
                    egui::ColorImage::from_rgba_unmultiplied([160, 90], &data)
                } else {
                    egui::ColorImage::from_rgb([160, 90], &data)
                };
                images.push((branch.id, image));
            }
            Ok(images)
        })();
        let _ = tx.send(result.map_err(|e| format!("{e:#}")));
    });
}
