//! Build a portable native scene: recursive architecture, neon emitters and HDR night.
//! cargo run -p syfrack-render --example cyberpunk_scene --offline -j 2 -- renders/cyberpunk
use anyhow::Result;
use glam::Vec3;
use std::path::PathBuf;
use syfrack_color::write_exr;
use syfrack_core::{
    Atmosphere, CameraMode, Color, Coloring, Fractal, Graph, HdrImage, Light, Link, Look, Node,
    NodeKind, Scene, SurfaceVariation, Tonemap, Vignette,
};

fn node(g: &mut Graph, kind: NodeKind, name: &str) -> u32 {
    let id = g.nodes.len() as u32;
    g.nodes.push(Node {
        id,
        kind,
        name: name.into(),
        enabled: true,
        pos: [240.0 * (id % 8) as f32, 160.0 * (id / 8) as f32],
    });
    id
}
fn link(g: &mut Graph, from: u32, to: u32, port: u32) {
    g.links.push(Link {
        from: (from, 0),
        to: (to, port),
    });
}
fn chain(g: &mut Graph, from: u32, kind: NodeKind, name: &str) -> u32 {
    let id = node(g, kind, name);
    link(g, from, id, 0);
    id
}
fn tower(g: &mut Graph, center: Vec3, scale: f32, height: f32, yaw: f32) -> u32 {
    let translated = chain(
        g,
        0,
        NodeKind::Translate { offset: -center },
        "Position de la tour",
    );
    let rotated = chain(
        g,
        translated,
        NodeKind::Rotate {
            angles: Vec3::new(0.0, yaw, 0.0),
        },
        "Orientation",
    );
    let local = chain(g, rotated, NodeKind::Scale { factor: scale }, "Dimensions");
    let tiled = if height <= 1.0 {
        local
    } else {
        chain(
            g,
            local,
            NodeKind::Tile {
                period: Vec3::new(100.0, 2.0, 100.0),
            },
            "Etages recursifs",
        )
    };
    let fold = node(
        g,
        NodeKind::MengerFold {
            scale: 3.0,
            offset: Vec3::ONE,
        },
        "Structure de Menger",
    );
    let repeated = chain(
        g,
        tiled,
        NodeKind::Iterate {
            iterations: 4,
            bailout: 1_000_000.0,
            body: vec![fold],
        },
        "Quatre niveaux de detail",
    );
    let sponge = chain(g, repeated, NodeKind::Box { half: Vec3::ONE }, "Fractale");
    if height <= 1.0 {
        return sponge;
    }
    let bounds = chain(
        g,
        local,
        NodeKind::Box {
            half: Vec3::new(1.0, height, 1.0),
        },
        "Enveloppe finie",
    );
    let cut = node(g, NodeKind::Intersection, "Limiter les etages");
    link(g, sponge, cut, 0);
    link(g, bounds, cut, 1);
    cut
}
fn area(position: Vec3, normal: Vec3, size: [f32; 2], rgb: [f32; 3], intensity: f32) -> Light {
    Light::Area {
        position,
        normal: normal.normalize(),
        size,
        color: Color::Linear(rgb),
        intensity,
    }
}

fn main() -> Result<()> {
    let dir = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("renders/cyberpunk"));
    std::fs::create_dir_all(&dir)?;
    let mut sky = HdrImage::new(1024, 512);
    let key = Vec3::new(-0.4, 0.8, 0.5).normalize();
    let rim = Vec3::new(0.9, 0.25, -0.3).normalize();
    for y in 0..sky.height {
        let theta = std::f32::consts::PI * (y as f32 + 0.5) / sky.height as f32;
        for x in 0..sky.width {
            let phi = std::f32::consts::TAU * (x as f32 + 0.5) / sky.width as f32;
            let d = Vec3::new(
                theta.sin() * phi.cos(),
                theta.cos(),
                theta.sin() * phi.sin(),
            );
            let mut rgb = Vec3::new(0.012, 0.018, 0.04) * (0.6 + d.y.max(0.0));
            if d.dot(key) > 0.975 {
                rgb += Vec3::new(0.18, 0.65, 1.2);
            }
            if d.dot(rim) > 0.985 {
                rgb += Vec3::new(1.1, 0.018, 0.3);
            }
            // Distant rectangular windows form a dim, original city panorama.
            let block = x / 29;
            let top = 202 + (block * 17 % 46);
            if y > top && y < 282 && x % 29 > 3 && x % 29 < 25 {
                rgb *= 0.3;
                if x % 6 < 1 && y % 9 < 2 && (block + y / 9) % 3 != 0 {
                    rgb += if block % 3 == 0 {
                        Vec3::new(0.065, 0.004, 0.017)
                    } else {
                        Vec3::new(0.004, 0.025, 0.05)
                    };
                }
            }
            sky.set(x, y, [rgb.x, rgb.y, rgb.z, 1.0]);
        }
    }
    write_exr(dir.join("neon-night.exr"), &sky)?;

    let mut g = Graph::new_minimal();
    g.nodes.clear();
    g.links.clear();
    node(&mut g, NodeKind::Input, "Monde");
    let output = node(&mut g, NodeKind::Output, "Cathedrale du neon");
    let mut combined = chain(
        &mut g,
        0,
        NodeKind::Plane {
            normal: Vec3::Y,
            offset: -0.04,
        },
        "Parvis reflechissant",
    );
    let hero = tower(&mut g, Vec3::new(0.0, 1.7, -0.8), 0.6, 1.0, 18.0);
    // One shared recursive field supplies all distant buildings. Their finite
    // envelopes are cheap boxes, avoiding three copies of the fractal evaluator.
    let city_point = chain(
        &mut g,
        0,
        NodeKind::Tile {
            period: Vec3::new(4.0, 2.0, 4.0),
        },
        "Ville modulaire",
    );
    let city_fold = node(
        &mut g,
        NodeKind::MengerFold {
            scale: 3.0,
            offset: Vec3::ONE,
        },
        "Menger des tours",
    );
    let city_iter = chain(
        &mut g,
        city_point,
        NodeKind::Iterate {
            iterations: 4,
            bailout: 1_000_000.0,
            body: vec![city_fold],
        },
        "Architecture recursive",
    );
    let city_fractal = chain(
        &mut g,
        city_iter,
        NodeKind::Box { half: Vec3::ONE },
        "Facades ajourees",
    );
    let mut city_envelope = None;
    for center in [
        Vec3::new(-4.0, 3.0, -4.0),
        Vec3::new(4.0, 3.0, -8.0),
        Vec3::new(0.0, 3.0, -8.0),
    ] {
        let local = chain(
            &mut g,
            0,
            NodeKind::Translate { offset: -center },
            "Emplacement dans la ville",
        );
        let bounds = chain(
            &mut g,
            local,
            NodeKind::Box {
                half: Vec3::new(1.0, 3.0, 1.0),
            },
            "Enveloppe de tour",
        );
        city_envelope = Some(if let Some(previous) = city_envelope {
            let union = node(&mut g, NodeKind::Union, "Silhouette urbaine");
            link(&mut g, previous, union, 0);
            link(&mut g, bounds, union, 1);
            union
        } else {
            bounds
        });
    }
    let city = node(&mut g, NodeKind::Intersection, "Ville finie");
    link(&mut g, city_fractal, city, 0);
    link(&mut g, city_envelope.unwrap(), city, 1);
    for shape in [hero, city] {
        let union = node(&mut g, NodeKind::Union, "Architecture et parvis");
        link(&mut g, combined, union, 0);
        link(&mut g, shape, union, 1);
        combined = union;
    }
    link(&mut g, combined, output, 0);
    g.safety = 0.85;
    g.bounding_radius_hint = 30.0;
    let mut scene = Scene::example();
    scene.fractal = Fractal::Graph(g);
    scene.camera.mode = CameraMode::Free {
        position: Vec3::new(5.8, 2.5, 8.8),
        target: Vec3::new(0.0, 1.8, -1.4),
        roll: 0.0,
    };
    scene.camera.focal_mm = 32.0;
    scene.camera.f_number = 0.0;
    scene.material.albedo = [0.45, 0.5, 0.56];
    scene.material.metallic = 0.65;
    scene.material.roughness = 0.2;
    scene.material.coloring = Coloring::None;
    scene.material.texture = None;
    scene.material.surface = Some(SurfaceVariation {
        slot: 5,
        roughness: [0.3, 0.11],
        metallic: [0.55, 0.8],
    });
    scene.atmosphere = Atmosphere::none();
    scene.atmosphere.fog_density = 0.012;
    scene.atmosphere.fog_color = [0.68, 0.8, 0.95];
    scene.atmosphere.fog_height = 0.0;
    scene.atmosphere.fog_falloff = 0.22;
    scene.atmosphere.anisotropy = 0.3;
    let cyan = [0.006, 0.65, 1.0];
    let pink = [1.0, 0.006, 0.23];
    scene.lights = vec![
        Light::Hdri {
            path: PathBuf::from("neon-night.exr"),
            intensity: 1.0,
            rotation: 0.0,
        },
        area(
            Vec3::new(-2.0, 1.7, -0.4),
            Vec3::new(0.7, 0.0, 0.7),
            [0.08, 3.4],
            cyan,
            28.0,
        ),
        area(
            Vec3::new(2.0, 1.7, -0.4),
            Vec3::new(-0.7, 0.0, 0.7),
            [0.08, 3.4],
            pink,
            28.0,
        ),
        area(
            Vec3::new(-2.2, 0.015, 1.7),
            Vec3::Y,
            [0.055, 7.0],
            cyan,
            18.0,
        ),
        area(
            Vec3::new(2.2, 0.015, 1.7),
            Vec3::Y,
            [0.055, 7.0],
            pink,
            18.0,
        ),
        area(
            Vec3::new(0.0, 1.8, -4.6),
            Vec3::Z,
            [1.4, 0.15],
            [1.0, 0.19, 0.016],
            40.0,
        ),
        area(
            Vec3::new(-4.0, 3.0, -2.97),
            Vec3::Z,
            [0.25, 4.2],
            cyan,
            18.0,
        ),
        area(
            Vec3::new(0.0, 6.8, 0.5),
            Vec3::NEG_Y,
            [4.0, 3.0],
            [0.07, 0.35, 1.0],
            1.2,
        ),
    ];
    scene.look = Look::neutral();
    scene.look.exposure_ev = 0.7;
    scene.look.saturation = 1.12;
    scene.look.vibrance = 0.2;
    scene.look.tonemap = Tonemap::Agx {
        slope: 1.0,
        power: 1.12,
        offset: 0.0,
    };
    scene.look.vignette = Some(Vignette {
        radius: 0.9,
        softness: 0.8,
        intensity: 0.18,
        elliptical: true,
    });
    scene.validate()?;
    let path = dir.join("cathedrale-neon.ron");
    scene.save(&path)?;
    println!("Native scene: {}", path.display());
    Ok(())
}
