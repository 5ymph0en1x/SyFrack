//! A neutral copy of a scene for judging the form alone (spec §5.4).
use crate::scene::*;
use glam::Vec3;

/// `scene` with a grey matte material, a fixed sun and sky, no fog, a deep
/// depth of field and a plain look; the form graph and the camera framing
/// are kept.
pub fn clay_scene(scene: &Scene) -> Scene {
    if scene.dimension == Dimension::TwoD {
        return scene.clone();
    }
    let mut s = scene.clone();
    if let Some(g) = &mut s.creation {
        let output = g
            .nodes
            .iter()
            .find(|n| matches!(n.kind, CreationKind::Output))
            .map(|n| n.id);
        g.links
            .retain(|l| Some(l.to.0) != output || matches!(l.to.1, 0 | 4 | 6));
        g.settings.film = None;
        g.settings.dispersion = None;
    }
    let finish_count = scene
        .fractal
        .graph()
        .nodes
        .iter()
        .filter_map(|n| {
            if let NodeKind::MaterialTag { index } = n.kind {
                Some(index as usize)
            } else {
                None
            }
        })
        .max()
        .unwrap_or(0)
        .max(
            scene
                .atmosphere
                .ground
                .as_ref()
                .map_or(0, |g| g.finish as usize),
        )
        .min(material::MAX_FINISHES);
    s.material = Material {
        albedo: [0.6; 3],
        roughness: 0.6,
        metallic: 0.0,
        specular: 0.04,
        coloring: Coloring::None,
        texture: None,
        optics: None,
        surface: None,
        finishes: (0..finish_count)
            .map(|_| Finish {
                name: "Argile".into(),
                albedo: [0.6; 3],
                roughness: 0.6,
                ..Finish::default()
            })
            .collect(),
    };
    s.lights = vec![
        Light::Environment {
            zenith: Color::Linear([1.0; 3]),
            horizon: Color::Linear([0.9; 3]),
            ground: Color::Linear([0.4; 3]),
            intensity: 1.0,
            rotation: 0.0,
        },
        Light::Sun {
            direction: Vec3::new(-0.4, 0.8, 0.4).normalize(),
            color: Color::Kelvin(5500.0),
            intensity: 3.0,
            angular_size: 2.0,
        },
    ];
    s.atmosphere.fog_density = 0.0;
    s.atmosphere.seascape = None;
    s.atmosphere.portal = None;
    s.atmosphere.nebula = None;
    s.camera.f_number = 64.0;
    let mut look = Scene::example().look;
    look.auto_exposure = Some(0.18);
    look.tonemap = Tonemap::Agx {
        slope: 1.0,
        power: 1.0,
        offset: 0.0,
    };
    look.vignette = None;
    look.grain = None;
    look.saturation = 1.0;
    look.vibrance = 0.0;
    look.shadows = Zone::neutral();
    look.midtones = Zone::neutral();
    look.highlights = Zone::neutral();
    s.look = look;
    s
}

/// The clay scene a form thumbnail is rendered from: the example scene
/// orbiting `g` at 2.5 times its estimated radius (at least 0.5), focused
/// on it (`syfrack graph thumbnail` and the studio's library).
pub fn thumbnail_scene(g: &crate::Graph) -> Scene {
    let distance = 2.5 * g.estimate_bounding_radius().max(0.5);
    let mut scene = Scene::example();
    scene.camera.mode = CameraMode::Orbit {
        point: Vec3::ZERO,
        distance,
        azimuth: 30.0,
        elevation: 20.0,
        roll: 0.0,
    };
    scene.camera.focus_distance = distance;
    scene.fractal = Fractal::Graph(g.clone());
    clay_scene(&scene)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_thumbnail_scene_frames_the_form_in_clay() {
        let g = crate::Graph::bundled("mandelbox").unwrap();
        let s = super::thumbnail_scene(&g);
        s.validate().unwrap();
        assert_eq!(s.material.coloring, crate::Coloring::None);
        let d = 2.5 * g.estimate_bounding_radius().max(0.5);
        assert!(
            matches!(s.camera.mode, crate::CameraMode::Orbit { distance, .. } if distance == d)
        );
        assert_eq!(s.camera.focus_distance, d);
        assert_eq!(s.fractal, crate::Fractal::Graph(g));
    }

    #[test]
    fn clay_scene_has_the_fixed_values_and_validates() {
        let mut original = crate::Scene::example();
        original.material.texture = Some(crate::FractalTexture::default());
        let s = super::clay_scene(&original);
        assert!(s.material.texture.is_none());
        assert_eq!(s.material.albedo, [0.6; 3]);
        assert_eq!(s.lights.len(), 2);
        assert_eq!(s.atmosphere.fog_density, 0.0);
        assert_eq!(s.camera.f_number, 64.0);
        assert!(s.look.vignette.is_none() && s.look.grain.is_none());
        s.validate().unwrap();
    }
}
