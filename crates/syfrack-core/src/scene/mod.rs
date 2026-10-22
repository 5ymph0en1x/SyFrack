pub mod atmosphere;
pub mod camera;
pub mod clay;
pub mod creation;
pub mod flame;
pub mod fractal;
pub mod graph;
pub mod light;
pub mod look;
pub mod material;
pub mod nebula;
pub mod optics;
pub mod palette;
pub mod phenomena;
pub mod planar;
pub mod seascape;
pub mod texture;
pub mod workspace;

pub use atmosphere::{Atmosphere, Dunes, GroundPlane, PortalWorld};
pub use camera::{Camera, CameraBasis, CameraMode};
pub use clay::{clay_scene, thumbnail_scene};
pub use creation::{CreationGraph, CreationKind, CreationNode, SocketType};
pub use flame::{Flame, FlameTerm, FlameTransform, FlameVariation};
pub use fractal::{DeSample, Fractal};
pub use graph::{Graph, Link, Measure, Node, NodeId, NodeKind, Placement, PortType, TrapMode};
pub use light::{Color, Light, MAX_LIGHTS};
pub use look::{Grain, Look, OutputSpace, Tonemap, Vignette, Zone};
pub use material::{
    Coloring, Finish, FinishDetail, FinishPattern, Material, OrbitColoring, PaletteInterpolation,
    Ramp, SurfaceVariation, MAX_PALETTE_STOPS,
};
pub use nebula::{Nebula, Particles};
pub use optics::Optics;
pub use phenomena::Phenomena;
pub use planar::{PlanarFamily, PlanarFractal};
pub use seascape::{OceanSurface, Seascape};
pub use texture::{FanMapping, FractalTexture, TextureChannel, TextureMask};
pub use workspace::{Canvas2D, Dimension};

use crate::params::{Bounds, ParamInfo, Params};
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum SceneError {
    #[error("scene parse error: {0}")]
    Parse(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parameter `{path}` = {value} is outside [{min}, {max}]")]
    OutOfBounds {
        path: String,
        value: f32,
        min: f32,
        max: f32,
    },
    #[error("unknown parameter path `{0}`")]
    UnknownPath(String),
    #[error("a scene needs at least one light")]
    NoLights,
    #[error("a scene may have at most {MAX_LIGHTS} lights, got {0}")]
    TooManyLights(usize),
    #[error("ramp at `{path}` has {stops} stops, needs 2 to 16")]
    InvalidRamp { path: String, stops: usize },
    #[error("graph: {0}")]
    Graph(String),
    /// A fault of one node of the form graph.
    #[error("graph: node {id}: {msg}")]
    Node { id: graph::NodeId, msg: String },
}

/// The message of a scene written with the hand-written formulas or the
/// older colourings, which this version no longer reads.
pub const PRE_PLAN4: &str = "legacy scene format: use the previous version to convert this scene";

/// Whether `text` names a removed fractal variant or colouring (whitespace
/// ignored): `fractal: Mandelbox(…)`, `coloring: OrbitTrap(…)`, ….
pub fn is_pre_plan4(text: &str) -> bool {
    let flat: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    ["Sphere", "Mandelbulb", "Mandelbox", "MengerFold"]
        .iter()
        .any(|v| flat.contains(&format!("fractal:{v}(")))
        || ["OrbitTrap", "Normal", "Iteration"]
            .iter()
            .any(|v| flat.contains(&format!("coloring:{v}(")))
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creation: Option<CreationGraph>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creation_draft: Option<CreationGraph>,
    #[serde(default)]
    pub dimension: Dimension,
    #[serde(default)]
    pub canvas: Canvas2D,
    pub seed: u64,
    #[serde(default = "default_form")]
    pub fractal: Fractal,
    pub camera: Camera,
    pub lights: Vec<Light>,
    pub material: Material,
    pub atmosphere: Atmosphere,
    pub look: Look,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phenomena: Option<Phenomena>,
}

fn default_form() -> Fractal {
    Fractal::Graph(Graph::bundled("sphere").expect("bundled sphere"))
}

// Canonical documents contain one graph. Lowered field/geometry IR is a runtime
// cache, not a second editable source serialized alongside it.
impl Serialize for Scene {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Saved<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            creation: Option<&'a CreationGraph>,
            #[serde(skip_serializing_if = "Option::is_none")]
            creation_draft: Option<&'a CreationGraph>,
            dimension: Dimension,
            canvas: &'a Canvas2D,
            seed: u64,
            #[serde(
                skip_serializing_if = "Option::is_none",
                serialize_with = "serialize_legacy_form"
            )]
            fractal: Option<&'a Fractal>,
            camera: &'a Camera,
            lights: &'a [Light],
            material: &'a Material,
            atmosphere: &'a Atmosphere,
            look: &'a Look,
            #[serde(skip_serializing_if = "Option::is_none")]
            phenomena: Option<&'a Phenomena>,
        }
        let canonical = self.creation.is_some();
        let mut material = self.material.clone();
        if canonical
            && self
                .creation
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .any(|n| matches!(n.kind, CreationKind::Texture(_)))
        {
            material.texture = None;
        }
        Saved {
            creation: self.creation.as_ref(),
            creation_draft: self.creation_draft.as_ref(),
            dimension: self.dimension,
            canvas: &self.canvas,
            seed: self.seed,
            fractal: (!canonical).then_some(&self.fractal),
            camera: &self.camera,
            lights: &self.lights,
            material: &material,
            atmosphere: &self.atmosphere,
            look: &self.look,
            phenomena: if canonical {
                None
            } else {
                self.phenomena.as_ref()
            },
        }
        .serialize(serializer)
    }
}
fn serialize_legacy_form<S: serde::Serializer>(
    form: &Option<&Fractal>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    form.expect("an omitted form is skipped by serde")
        .serialize(serializer)
}

impl Scene {
    /// An immediately usable flat Julia canvas, independent of lights and surfaces.
    pub fn example_2d() -> Self {
        let mut scene = Self::example();
        scene.dimension = Dimension::TwoD;
        scene.material.texture = Some(FractalTexture {
            planar: Some(PlanarFractal {
                julia: [-0.8, 0.156],
                center: [0.08, 0.02],
                zoom: 5.0,
                angle: 23.0,
                iterations: 192,
                stripes: 8.0,
                ..PlanarFractal::default()
            }),
            channel: TextureChannel::Orbit {
                channels: OrbitColoring {
                    traps: [1.0, 0.08, 0.18, 0.0],
                    ..OrbitColoring::default()
                },
            },
            scale: 0.65,
            ramp: palette::neon_palette(0),
            offset: Vec3::ZERO,
            rotation: Vec3::ZERO,
            octaves: 1,
            contrast: 1.0,
            roughness: 0.0,
            bump: 0.0,
            ..FractalTexture::default()
        });
        scene.look.tonemap = Tonemap::Direct;
        scene.look.neon_softness = 0.6;
        scene.look.auto_exposure = None;
        scene.lights.clear();
        scene
    }

    /// A sphere resting on a ground plane, lit by a warm sun and a blue sky.
    pub fn example() -> Self {
        Self {
            creation: None,
            creation_draft: None,
            dimension: Dimension::ThreeD,
            canvas: Canvas2D::default(),
            seed: 7,
            fractal: Fractal::Graph(Graph::bundled("sphere").expect("graphs/sphere.ron")),
            camera: Camera {
                mode: CameraMode::Orbit {
                    point: Vec3::ZERO,
                    distance: 5.0,
                    azimuth: 30.0,
                    elevation: 12.0,
                    roll: 0.0,
                },
                focal_mm: 28.0,
                f_number: 0.0,
                focus_distance: 5.0,
            },
            lights: vec![
                Light::Sun {
                    direction: Vec3::new(-0.4, 0.8, 0.3),
                    color: Color::Kelvin(5500.0),
                    intensity: 6.0,
                    angular_size: 1.0,
                },
                Light::Environment {
                    zenith: Color::Linear([0.2, 0.35, 0.7]),
                    horizon: Color::Linear([0.6, 0.65, 0.7]),
                    ground: Color::Linear([0.15, 0.12, 0.1]),
                    intensity: 0.7,
                    rotation: 0.0,
                },
            ],
            material: Material {
                albedo: [0.7, 0.65, 0.6],
                roughness: 0.4,
                metallic: 0.0,
                specular: 0.04,
                coloring: Coloring::None,
                texture: None,
                surface: None,
                finishes: vec![],
                optics: None,
            },
            atmosphere: Atmosphere::none(),
            look: Look::neutral(),
            phenomena: None,
        }
    }

    pub fn from_ron(text: &str) -> Result<Self, SceneError> {
        let scene: Self = ron::from_str(text).map_err(|e| {
            if is_pre_plan4(text) {
                SceneError::Parse(format!("{PRE_PLAN4} ({e})"))
            } else {
                SceneError::Parse(e.to_string())
            }
        })?;
        if scene.creation.is_some() {
            scene.render_scene()
        } else {
            Ok(scene)
        }
    }

    pub fn to_ron(&self) -> String {
        let config = ron::ser::PrettyConfig::new().depth_limit(6);
        ron::ser::to_string_pretty(self, config).expect("scene is always serialisable")
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, SceneError> {
        let path = path.as_ref();
        let mut scene = Self::from_ron(&std::fs::read_to_string(path)?)?;
        scene.resolve_asset_paths(path.parent().unwrap_or_else(|| Path::new(".")))?;
        Ok(scene)
    }

    /// Make panorama references independent of later working-directory changes.
    /// Missing assets remain editable; the renderer reports their load error.
    pub fn resolve_asset_paths(&mut self, base: &Path) -> Result<(), SceneError> {
        let base = std::env::current_dir()?.join(base);
        for light in &mut self.lights {
            if let Light::Hdri { path, .. } = light {
                if path.is_relative() {
                    *path = base.join(&*path);
                }
            }
        }
        Ok(())
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), SceneError> {
        std::fs::write(path, self.to_ron())?;
        Ok(())
    }
}

impl Scene {
    /// A starting scene for a bundled form (`graphs/<name>.ron`): the
    /// classics keep their tuned camera and colour ramp, the other forms are
    /// framed at 2.5 times their estimated radius.
    pub fn example_for(name: &str) -> Result<Self, SceneError> {
        if name == "flame" {
            let mut scene = Self::example_2d();
            scene.canvas.flame = Some(Flame::default());
            scene.material.texture = None;
            return Ok(scene);
        }
        let graph = Graph::bundled(name).ok_or_else(|| {
            let names: Vec<&str> = graph::bundled::BUNDLED.iter().map(|(n, _)| *n).collect();
            SceneError::Graph(format!("forme inconnue `{name}` ({})", names.join(", ")))
        })?;
        let mut s = Self::example();
        s.fractal = Fractal::Graph(graph);
        let orbit = |distance: f32, azimuth: f32, elevation: f32| CameraMode::Orbit {
            point: Vec3::ZERO,
            distance,
            azimuth,
            elevation,
            roll: 0.0,
        };
        match name {
            "sphere" => {}
            "mandelbulb" => {
                s.camera.mode = orbit(3.2, 35.0, 25.0);
                s.camera.focal_mm = 40.0;
                s.camera.focus_distance = 2.2;
                s.material.albedo = [0.85, 0.8, 0.7];
                s.material.coloring = Coloring::Trap {
                    slot: 0,
                    ramp: Ramp {
                        stops: vec![[0.9, 0.3, 0.1], [0.95, 0.85, 0.6], [0.3, 0.5, 0.8]],
                        ..Ramp::default()
                    },
                };
            }
            "mandelbox" => {
                s.camera.mode = orbit(11.0, 28.0, 22.0);
                s.camera.focal_mm = 35.0;
                s.camera.focus_distance = 9.0;
                s.material.albedo = [0.8, 0.78, 0.75];
                s.material.roughness = 0.35;
                s.material.coloring = Coloring::Trap {
                    slot: 0,
                    ramp: Ramp {
                        stops: vec![[0.6, 0.6, 0.65], [0.9, 0.7, 0.4], [0.2, 0.3, 0.5]],
                        ..Ramp::default()
                    },
                };
            }
            "menger" => {
                s.camera.mode = orbit(3.6, 40.0, 30.0);
                s.camera.focal_mm = 40.0;
                s.camera.focus_distance = 2.6;
                s.material.coloring = Coloring::Trap {
                    slot: 5,
                    ramp: Ramp {
                        stops: vec![[0.5, 0.45, 0.4], [0.9, 0.9, 0.95]],
                        ..Ramp::default()
                    },
                };
            }
            _ => {
                let distance = 2.5 * s.fractal.graph().estimate_bounding_radius().max(0.5);
                s.camera.mode = orbit(distance, 30.0, 20.0);
                s.camera.focus_distance = distance;
            }
        }
        Ok(s)
    }
}

impl Params for Scene {
    fn visit_params(&mut self, prefix: &str, f: &mut dyn FnMut(&str, &mut f32, Bounds)) {
        use crate::params::join;
        self.canvas.visit_params(&join(prefix, "canvas"), f);
        self.fractal.visit_params(&join(prefix, "fractal"), f);
        self.camera.visit_params(&join(prefix, "camera"), f);
        for (i, light) in self.lights.iter_mut().enumerate() {
            light.visit_params(&format!("{}[{i}]", join(prefix, "lights")), f);
        }
        self.material.visit_params(&join(prefix, "material"), f);
        self.atmosphere.visit_params(&join(prefix, "atmosphere"), f);
        self.look.visit_params(&join(prefix, "look"), f);
        if let Some(g) = &mut self.creation {
            g.visit_params(&join(prefix, "creation"), f);
        }
        if let Some(phenomena) = &mut self.phenomena {
            phenomena.visit_params(&join(prefix, "phenomena"), f);
        }
    }
}

impl Scene {
    /// Every numeric parameter with its path and bounds.
    pub fn params(&self) -> Vec<ParamInfo> {
        let mut out = Vec::new();
        let mut copy = self.clone();
        copy.visit_params("", &mut |path, v, bounds| {
            out.push(ParamInfo {
                path: path.to_string(),
                value: *v,
                bounds,
            });
        });
        out
    }

    /// Checks light count and every parameter against its valid range.
    pub fn validate(&self) -> Result<(), SceneError> {
        if let Some(g) = &self.creation {
            g.compile(self)?;
            return Ok(());
        }
        self.fractal.graph().validate()?;
        self.validate_rest()
    }

    /// The same checks with the form graph allowed to be incomplete
    /// (unwired ports): what an editor may open and save.
    pub fn validate_editable(&self) -> Result<(), SceneError> {
        if let Some(g) = &self.creation {
            g.types()?;
            g.compile(self)?;
            return Ok(());
        }
        self.fractal.graph().validate_structure()?;
        self.validate_rest()
    }

    fn validate_rest(&self) -> Result<(), SceneError> {
        if let Some(phenomena) = &self.phenomena {
            phenomena.validate()?;
            if phenomena.lens.as_ref().is_some_and(|l| l.mass > 0.0) {
                if self.dimension != Dimension::ThreeD
                    || self.atmosphere.fog_density > 0.0
                    || self.atmosphere.nebula.is_some()
                    || self.atmosphere.portal.is_some()
                    || self.atmosphere.seascape.is_some()
                {
                    return Err(SceneError::Graph("Kerr rays require 3D geometry and natural-field volumes; disable the Euclidean fog, ocean, nebula and portal transports".into()));
                }
                if self
                    .material
                    .optics
                    .iter()
                    .chain(
                        self.material
                            .finishes
                            .iter()
                            .filter_map(|f| f.optics.as_ref()),
                    )
                    .any(|o| o.subsurface > 0.0)
                {
                    return Err(SceneError::Graph(
                        "subsurface free-flight is not supported on Kerr rays".into(),
                    ));
                }
            }
        }
        if let Some(flame) = &self.canvas.flame {
            flame.validate()?;
            if self.dimension != Dimension::TwoD {
                return Err(SceneError::Graph("flamme : utiliser l'atelier 2D".into()));
            }
        }
        if self.dimension == Dimension::TwoD
            && self.canvas.flame.is_none()
            && !self
                .phenomena
                .as_ref()
                .is_some_and(|p| p.bindings.color.is_some() || p.bindings.rgb.is_some())
            && self
                .material
                .texture
                .as_ref()
                .and_then(|t| t.planar.as_ref())
                .is_none()
        {
            return Err(SceneError::Graph(
                syfrack_i18n::tr!("le mode 2D nécessite un motif complexe").into(),
            ));
        }
        if self.lights.is_empty() && self.dimension == Dimension::ThreeD {
            return Err(SceneError::NoLights);
        }
        if self.lights.len() > MAX_LIGHTS {
            return Err(SceneError::TooManyLights(self.lights.len()));
        }
        for light in &self.lights {
            if let Light::Tube { start, end, .. } = light {
                if start.distance_squared(*end) < 1e-10 {
                    return Err(SceneError::Graph(
                        syfrack_i18n::tr!("les extrémités d'un tube néon doivent être distinctes")
                            .into(),
                    ));
                }
            }
        }
        self.material.validate_ramps("material")?;
        if self
            .atmosphere
            .ground
            .as_ref()
            .is_some_and(|g| g.finish as usize > self.material.finishes.len())
        {
            return Err(SceneError::Graph(
                syfrack_i18n::tr!("matériau du sol absent de la palette").into(),
            ));
        }
        for node in &self.fractal.graph().nodes {
            if node.enabled {
                if let NodeKind::MaterialTag { index } = node.kind {
                    if index as usize > self.material.finishes.len() {
                        return Err(SceneError::Node {
                            id: node.id,
                            msg: syfrack_i18n::tr!("matériau absent de la palette").into(),
                        });
                    }
                }
            }
        }
        for light in &self.lights {
            if let Light::Projector {
                inner_angle,
                outer_angle,
                ..
            } = light
            {
                if inner_angle >= outer_angle {
                    return Err(SceneError::Graph(syfrack_i18n::tr!("projecteur : le demi-angle intérieur doit être inférieur au demi-angle extérieur").into()));
                }
            }
        }
        crate::params::validate_params(self, "")
    }

    /// Sets one parameter by path, refusing values outside its valid range.
    pub fn set_param(&mut self, path: &str, value: f32) -> Result<(), SceneError> {
        if let Some(g) = &self.creation {
            let mapped = if let Some(rest) = path.strip_prefix("fractal.nodes[") {
                Some(format!("creation.nodes[{rest}"))
            } else if path == "fractal.safety" {
                Some("creation.safety".into())
            } else if path == "fractal.bounding_radius_hint" {
                Some("creation.bounding_radius".into())
            } else if let Some(rest) = path.strip_prefix("canvas.flame.") {
                g.nodes
                    .iter()
                    .find(|n| matches!(n.kind, CreationKind::Flame(_)))
                    .map(|n| format!("creation.nodes[{}].flame.{rest}", n.id))
            } else if let Some(rest) = path.strip_prefix("material.texture.") {
                let out = g
                    .nodes
                    .iter()
                    .find(|n| matches!(n.kind, CreationKind::Output))
                    .map(|n| n.id);
                let selected = out.and_then(|id| g.input(id, 7)).map(|a| a.0).or_else(|| {
                    g.nodes
                        .iter()
                        .find(|n| matches!(n.kind, CreationKind::Texture(_)))
                        .map(|n| n.id)
                });
                selected.map(|id| format!("creation.nodes[{id}].texture.{rest}"))
            } else {
                None
            };
            if let Some(mapped) = mapped {
                return self.set_param(&mapped, value);
            }
        }
        let mut found: Option<Bounds> = None;
        self.visit_params("", &mut |p, v, bounds| {
            if p == path {
                found = Some(bounds);
                if bounds.contains(value) {
                    *v = value;
                }
            }
        });
        match found {
            None => Err(SceneError::UnknownPath(path.to_string())),
            Some(b) if !b.contains(value) => Err(SceneError::OutOfBounds {
                path: path.to_string(),
                value,
                min: b.min,
                max: b.max,
            }),
            Some(_) => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dunes_and_portal_roundtrip_validate_and_keep_old_scenes_compatible() {
        let ground: GroundPlane = ron::from_str("(height:0.0,finish:0)").unwrap();
        assert!(ground.dunes.is_none());
        let mut scene = Scene::example();
        scene.atmosphere.ground = Some(GroundPlane {
            dunes: Some(Dunes::default()),
            ..ground
        });
        scene.atmosphere.portal = Some(PortalWorld::default());
        assert_eq!(Scene::from_ron(&scene.to_ron()).unwrap(), scene);
        assert!(clay::clay_scene(&scene).atmosphere.portal.is_none());
        scene
            .atmosphere
            .ground
            .as_mut()
            .unwrap()
            .dunes
            .as_mut()
            .unwrap()
            .wavelength = 0.0;
        assert!(scene.validate().is_err());
        scene
            .atmosphere
            .ground
            .as_mut()
            .unwrap()
            .dunes
            .as_mut()
            .unwrap()
            .wavelength = 38.0;
        scene.atmosphere.portal.as_mut().unwrap().radius = f32::NAN;
        assert!(scene.validate().is_err());
    }

    #[test]
    fn analytic_ground_is_optional_and_validates_its_finish_and_height() {
        let mut scene = Scene::example();
        assert!(
            Scene::from_ron(&scene.to_ron().replace("ground: None,", ""))
                .unwrap()
                .atmosphere
                .ground
                .is_none()
        );
        scene.atmosphere.ground = Some(GroundPlane {
            height: -0.45,
            finish: 0,
            ..GroundPlane::default()
        });
        assert_eq!(Scene::from_ron(&scene.to_ron()).unwrap(), scene);
        scene.atmosphere.ground.as_mut().unwrap().finish = 1;
        assert!(scene.validate().is_err());
        scene.material.finishes.push(Finish::default());
        assert!(clay::clay_scene(&scene).validate().is_ok());
        scene.atmosphere.ground.as_mut().unwrap().finish = 0;
        scene.atmosphere.ground.as_mut().unwrap().height = f32::NAN;
        assert!(scene.validate().is_err());
    }

    #[test]
    fn hdri_references_roundtrip_and_resolve_against_the_scene_file() {
        let dir = std::env::temp_dir().join(format!("syfrack-hdri-scene-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut scene = Scene::example();
        scene.lights.push(Light::Hdri {
            path: "studio.exr".into(),
            intensity: 2.0,
            rotation: 90.0,
        });
        assert_eq!(Scene::from_ron(&scene.to_ron()).unwrap(), scene);
        let file = dir.join("scene.ron");
        scene.save(&file).unwrap();
        let loaded = Scene::load(&file).unwrap();
        assert!(
            matches!(&loaded.lights[2], Light::Hdri { path, .. } if path == &dir.join("studio.exr"))
        );
        assert_eq!(
            loaded
                .params()
                .iter()
                .find(|p| p.path == "lights[2].hdri.rotation")
                .unwrap()
                .value,
            90.0
        );
        std::fs::remove_file(file).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn example_roundtrips_through_ron() {
        let scene = Scene::example();
        let text = scene.to_ron();
        let back = Scene::from_ron(&text).expect("parse");
        assert_eq!(back, scene);
    }

    #[test]
    fn hand_written_ron_parses() {
        let text = r#"
            (
                seed: 42,
                fractal: Graph((
                    nodes: [
                        (id: 0, kind: Input, name: "in", pos: (0.0, 0.0), enabled: true),
                        (id: 1, kind: Sphere(radius: 1.5), name: "s", pos: (240.0, 0.0), enabled: true),
                        (id: 2, kind: Output, name: "out", pos: (480.0, 0.0), enabled: true),
                    ],
                    links: [(from: (0, 0), to: (1, 0)), (from: (1, 0), to: (2, 0))],
                    safety: 1.0,
                    bounding_radius_hint: 2.0,
                )),
                camera: (
                    mode: Free(position: (0.0, 0.0, 5.0), target: (0.0, 0.0, 0.0), roll: 0.0),
                    focal_mm: 35.0, f_number: 0.0, focus_distance: 5.0,
                ),
                lights: [
                    Sun(direction: (0.0, 1.0, 0.0), color: Kelvin(6500.0), intensity: 2.0, angular_size: 1.0),
                ],
                material: (albedo: (0.8, 0.8, 0.8), roughness: 0.5, metallic: 0.0, specular: 0.04, coloring: None),
                atmosphere: (fog_density: 0.0, fog_color: (1.0, 1.0, 1.0), fog_height: 0.0, fog_falloff: 1.0, anisotropy: 0.0, cavity_darkness: 0.0),
                look: (
                    exposure_ev: 0.0, auto_exposure: None, temperature: 6500.0, tint: 0.0,
                    shadows: (offset: (0.0, 0.0, 0.0), gain: 0.0),
                    midtones: (offset: (0.0, 0.0, 0.0), gain: 0.0),
                    highlights: (offset: (0.0, 0.0, 0.0), gain: 0.0),
                    tonemap: Agx(slope: 1.0, power: 1.0, offset: 0.0),
                    saturation: 1.0, vibrance: 0.0, vignette: None, grain: None,
                    output: Srgb, dither: true,
                ),
            )
        "#;
        let scene = Scene::from_ron(text).expect("parse");
        assert_eq!(scene.seed, 42);
        assert_eq!(scene.lights.len(), 1);
        let g = scene.fractal.graph();
        assert_eq!(g.nodes.len(), 3);
        assert!(matches!(g.nodes[1].kind, NodeKind::Sphere { radius } if radius == 1.5));
        scene.validate().unwrap();
    }

    #[test]
    fn example_for_each_bundled_form_validates_and_roundtrips() {
        for (name, _) in graph::bundled::BUNDLED {
            let s = Scene::example_for(name).unwrap();
            s.validate().unwrap();
            assert_eq!(Scene::from_ron(&s.to_ron()).unwrap(), s);
        }
        assert_eq!(Scene::example_for("sphere").unwrap(), Scene::example());
        assert!(matches!(
            Scene::example_for("cube"),
            Err(SceneError::Graph(m)) if m.contains("cube") && m.contains("mandelbox")
        ));
    }

    #[test]
    fn a_pre_plan4_scene_is_refused_with_a_clear_message() {
        let text = Scene::example().to_ron();
        assert!(!is_pre_plan4(&text));
        let fractal_start = text.find("fractal:").unwrap();
        let camera_start = text.find("camera:").unwrap();
        let old = format!(
            "{}fractal: Mandelbox(scale: 2.0, min_radius: 0.5, fold_limit: 1.0, iterations: 12),\n    {}",
            &text[..fractal_start],
            &text[camera_start..]
        );
        match Scene::from_ron(&old) {
            Err(SceneError::Parse(m)) => assert!(m.starts_with(PRE_PLAN4), "{m}"),
            other => panic!("{other:?}"),
        }
        let old = text.replacen(
            "coloring: r#None",
            "coloring: OrbitTrap(ramp: (stops: []))",
            1,
        );
        assert_ne!(old, text, "the example's coloring is written `r#None`");
        assert!(is_pre_plan4(&old));
        assert!(Scene::from_ron(&old).is_err());
    }

    #[test]
    fn bad_ron_is_a_parse_error() {
        let err = Scene::from_ron("(seed: 1").unwrap_err();
        assert!(matches!(err, SceneError::Parse(_)));
    }

    #[test]
    fn kelvin_colors_are_plausible() {
        let warm = Color::Kelvin(2000.0).to_linear();
        let neutral = Color::Kelvin(6500.0).to_linear();
        let cool = Color::Kelvin(15000.0).to_linear();
        assert!(warm[0] > warm[2]);
        assert!(cool[2] > cool[0]);
        assert!((neutral[0] - neutral[2]).abs() < 0.15);
        assert!(neutral.iter().all(|c| *c <= 1.0 && *c > 0.0));
    }

    #[test]
    fn ramp_interpolates_between_stops() {
        let ramp = Ramp {
            stops: vec![[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]],
            ..Ramp::default()
        };
        assert_eq!(ramp.sample(0.0), [0.0, 0.0, 0.0]);
        assert_eq!(ramp.sample(1.0), [1.0, 1.0, 1.0]);
        let mid = ramp.sample(0.5);
        assert!((mid[0] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn example_scene_is_valid() {
        Scene::example().validate().expect("example must validate");
    }

    #[test]
    fn params_lists_expected_paths() {
        let paths: Vec<String> = Scene::example()
            .params()
            .into_iter()
            .map(|p| p.path)
            .collect();
        for expected in [
            "fractal.nodes[2].radius",
            "fractal.nodes[3].offset",
            "camera.orbit.elevation",
            "camera.focal_mm",
            "lights[0].sun.intensity",
            "lights[0].sun.color.kelvin",
            "lights[1].environment.zenith.r",
            "material.albedo.r",
            "atmosphere.fog_density",
            "look.exposure_ev",
            "look.tonemap.agx.slope",
        ] {
            assert!(
                paths.contains(&expected.to_string()),
                "missing {expected} in {paths:?}"
            );
        }
    }

    #[test]
    fn an_incomplete_graph_scene_is_editable_but_not_renderable() {
        let mut scene = Scene::example();
        let mut g = Graph::new_minimal();
        g.links.retain(|l| l.to.0 != 2);
        scene.fractal = Fractal::Graph(g);
        scene.validate_editable().unwrap();
        assert!(matches!(
            scene.validate(),
            Err(SceneError::Node { id: 2, .. })
        ));
    }

    #[test]
    fn set_param_changes_the_value() {
        let mut scene = Scene::example();
        scene.set_param("fractal.nodes[2].radius", 2.5).unwrap();
        assert!(matches!(
            scene.fractal.graph().node(2).unwrap().kind,
            NodeKind::Sphere { radius } if radius == 2.5
        ));
        let p = scene
            .params()
            .into_iter()
            .find(|p| p.path == "fractal.nodes[2].radius")
            .unwrap();
        assert_eq!(p.value, 2.5);
    }

    #[test]
    fn set_param_rejects_out_of_bounds_and_unknown() {
        let mut scene = Scene::example();
        assert!(matches!(
            scene.set_param("fractal.nodes[2].radius", 1e6),
            Err(SceneError::OutOfBounds { .. })
        ));
        assert!(matches!(
            scene.set_param("fractal.cube.size", 1.0),
            Err(SceneError::UnknownPath(_))
        ));
    }

    #[test]
    fn validate_reports_the_faulty_path() {
        let mut scene = Scene::example();
        scene.material.roughness = 7.0;
        match scene.validate() {
            Err(SceneError::OutOfBounds { path, .. }) => assert_eq!(path, "material.roughness"),
            other => panic!("expected OutOfBounds, got {other:?}"),
        }
    }

    #[test]
    fn auto_exposure_zero_is_rejected() {
        let mut scene = Scene::example();
        scene.look.auto_exposure = Some(0.0);
        match scene.validate() {
            Err(SceneError::OutOfBounds { path, .. }) => assert_eq!(path, "look.auto_exposure"),
            other => panic!("expected OutOfBounds, got {other:?}"),
        }
    }

    #[test]
    fn look_load_rejects_out_of_bounds() {
        let mut look = Look::neutral();
        look.saturation = 9.0;
        let text = ron::ser::to_string_pretty(&look, ron::ser::PrettyConfig::new()).unwrap();
        let path = std::env::temp_dir().join(format!("syfrack-look-{}.ron", std::process::id()));
        std::fs::write(&path, text).unwrap();
        let err = Look::load(&path).unwrap_err();
        assert!(matches!(err, SceneError::OutOfBounds { .. }), "{err:?}");
        look.saturation = 1.0;
        std::fs::write(
            &path,
            ron::ser::to_string_pretty(&look, Default::default()).unwrap(),
        )
        .unwrap();
        assert_eq!(Look::load(&path).unwrap(), look);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn zero_sun_intensity_is_rejected() {
        let mut scene = Scene::example();
        if let Light::Sun { intensity, .. } = &mut scene.lights[0] {
            *intensity = 0.0;
        }
        match scene.validate() {
            Err(SceneError::OutOfBounds { path, .. }) => {
                assert_eq!(path, "lights[0].sun.intensity")
            }
            other => panic!("expected OutOfBounds, got {other:?}"),
        }
    }

    #[test]
    fn shipped_example_ron_matches_example() {
        let shipped = Scene::from_ron(include_str!("../../../../scenes/example.ron")).unwrap();
        assert_eq!(shipped, Scene::example());
    }

    #[test]
    fn validate_requires_lights() {
        let mut scene = Scene::example();
        scene.lights.clear();
        assert!(matches!(scene.validate(), Err(SceneError::NoLights)));
        scene.lights = vec![Scene::example().lights[0].clone(); MAX_LIGHTS + 1];
        assert!(matches!(
            scene.validate(),
            Err(SceneError::TooManyLights(n)) if n == MAX_LIGHTS + 1
        ));
    }

    #[test]
    fn every_interesting_range_lies_inside_valid_range() {
        for p in Scene::example().params() {
            for (lo, hi) in p.bounds.interesting {
                assert!(
                    lo <= hi && *lo >= p.bounds.min && *hi <= p.bounds.max,
                    "{}",
                    p.path
                );
            }
        }
    }

    #[test]
    fn unknown_field_is_a_parse_error() {
        let text = Scene::example().to_ron();
        // Inject an extra key into the atmosphere record: a typo'd optional
        // field must be refused, not silently ignored.
        let injected = text.replacen(
            "fog_density:",
            "fog_densty: 0.5,
        fog_density:",
            1,
        );
        assert_ne!(injected, text, "injection point not found");
        let err = Scene::from_ron(&injected).unwrap_err();
        assert!(
            matches!(err, SceneError::Parse(ref m) if m.contains("fog_densty")),
            "{err}"
        );
    }

    #[test]
    fn every_variant_roundtrips() {
        let mut s = Scene::example();
        s.camera.mode = CameraMode::Free {
            position: Vec3::new(0.0, 1.0, 5.0),
            target: Vec3::ZERO,
            roll: 15.0,
        };
        s.lights.push(Light::Area {
            position: Vec3::new(0.0, 3.0, 0.0),
            normal: Vec3::NEG_Y,
            size: [1.0, 2.0],
            color: Color::Kelvin(3200.0),
            intensity: 10.0,
        });
        s.material.coloring = Coloring::Trap {
            slot: 0,
            ramp: Ramp {
                stops: vec![[0.1, 0.2, 0.3], [0.9, 0.8, 0.7], [0.5, 0.5, 0.5]],
                ..Ramp::default()
            },
        };
        s.look.tonemap = Tonemap::Reinhard { white: 4.0 };
        s.look.vignette = Some(Vignette {
            radius: 0.8,
            softness: 0.5,
            intensity: 0.3,
            elliptical: true,
        });
        s.look.grain = Some(Grain {
            amount: 0.1,
            size: 1.5,
            seed: 3,
        });
        s.look.auto_exposure = Some(0.18);
        s.validate().expect("variant scene validates");
        let back = Scene::from_ron(&s.to_ron()).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn ramp_stop_count_is_validated() {
        let mut s = Scene::example();
        s.material.coloring = Coloring::Trap {
            slot: 5,
            ramp: Ramp {
                stops: vec![[0.5; 3]],
                ..Ramp::default()
            },
        };
        match s.validate() {
            Err(SceneError::InvalidRamp { path, stops }) => {
                assert_eq!(path, "material.coloring.trap.ramp");
                assert_eq!(stops, 1);
            }
            other => panic!("{other:?}"),
        }
        s.material.coloring = Coloring::Trap {
            slot: 5,
            ramp: Ramp {
                stops: vec![[0.5; 3]; 17],
                ..Ramp::default()
            },
        };
        assert!(matches!(
            s.validate(),
            Err(SceneError::InvalidRamp { stops: 17, .. })
        ));
    }

    #[test]
    #[ignore = "regenerates the shipped scene files"]
    fn write_shipped_fractal_scenes() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenes");
        for name in ["mandelbulb", "mandelbox", "menger"] {
            Scene::example_for(name)
                .unwrap()
                .save(format!("{root}/{name}.ron"))
                .expect("save");
        }
    }

    #[test]
    fn shipped_fractal_scenes_match_example_for() {
        for (text, name) in [
            (
                include_str!("../../../../scenes/mandelbulb.ron"),
                "mandelbulb",
            ),
            (
                include_str!("../../../../scenes/mandelbox.ron"),
                "mandelbox",
            ),
            (include_str!("../../../../scenes/menger.ron"), "menger"),
        ] {
            let shipped = Scene::from_ron(text).expect("parse");
            assert_eq!(shipped, Scene::example_for(name).unwrap(), "{name}");
        }
    }
}
