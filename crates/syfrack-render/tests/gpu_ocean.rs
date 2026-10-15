//! Native ocean regressions. Run with --ignored --test-threads=1 on a GPU host.
use glam::Vec3;
use std::sync::Arc;
use syfrack_core::{
    Atmosphere, CameraMode, Color, Fractal, Graph, Light, OceanSurface, Scene, Seascape,
};
use syfrack_render::{GpuContext, RenderConfig, RenderOutput, Renderer};

fn sea_scene() -> Scene {
    let mut s = Scene::example();
    s.fractal = Fractal::Graph(Graph::new_minimal());
    s.camera.mode = CameraMode::Free {
        position: Vec3::new(8.0, 5.0, 4.0),
        target: Vec3::new(8.0, 0.0, 0.0),
        roll: 0.0,
    };
    s.camera.focal_mm = 500.0;
    s.camera.f_number = 0.0;
    s.atmosphere = Atmosphere::none();
    s.atmosphere.seascape = Some(Seascape {
        sea_level: 0.0,
        intensity: 0.0,
        surface: Some(OceanSurface {
            wave_height: 0.0,
            foam_amount: 0.0,
            absorption: [0.0; 3],
            scattering: 0.0,
            ..Default::default()
        }),
        ..Default::default()
    });
    s.lights = vec![Light::Area {
        position: Vec3::new(8.0, -2.0, 0.0),
        normal: Vec3::Y,
        size: [40.0; 2],
        color: Color::Linear([1.0; 3]),
        intensity: 12.0,
    }];
    s
}

fn ocean(s: &mut Scene) -> &mut OceanSurface {
    s.atmosphere
        .seascape
        .as_mut()
        .unwrap()
        .surface
        .as_mut()
        .unwrap()
}

fn render(r: &Renderer, s: &Scene, samples: u32) -> RenderOutput {
    s.validate().unwrap();
    let mut cfg = RenderConfig::new(24, 16, samples);
    cfg.max_bounces = 2;
    cfg.tile_size = 8;
    cfg.samples_per_dispatch = 1;
    r.render(s, &cfg, &mut |_| {}).unwrap()
}

fn mean(out: &RenderOutput) -> [f32; 3] {
    let mut sum = [0.0; 3];
    for p in &out.color.data {
        for c in 0..3 {
            sum[c] += p[c] / out.color.data.len() as f32;
        }
    }
    sum
}

#[test]
#[ignore = "needs GPU"]
fn waves_are_geometry_and_submerged_emitters_follow_beer_lambert() {
    let r = Renderer::new(Arc::new(GpuContext::new().expect("GPU"))).unwrap();
    let mut s = sea_scene();
    let clear = render(&r, &s, 64);
    assert!(clear.depth.iter().all(|d| *d > 6.0 && *d < 7.0));
    assert!(clear.normals.iter().all(|n| (n[1] - 1.0).abs() < 1e-6));
    let clear_mean = mean(&clear);
    eprintln!("clear submerged emitter: {clear_mean:?}");
    assert!(
        clear_mean[0] > 5.5 && clear_mean[0] < 7.0,
        "Snell/Fresnel transmission, not an opaque water tint"
    );

    ocean(&mut s).absorption = [0.5, 0.1, 0.02];
    let tinted = mean(&render(&r, &s, 64));
    let ratios = std::array::from_fn::<_, 3, _>(|c| tinted[c] / clear_mean[c]);
    eprintln!("absorbing emitter: {tinted:?}, ratios {ratios:?}");
    assert!(ratios[0] < 0.35 && ratios[1] > 0.7 && ratios[2] > 0.94);
    assert!(ratios[0] < ratios[1] && ratios[1] < ratios[2]);

    ocean(&mut s).absorption = [0.0; 3];
    ocean(&mut s).wave_height = 0.8;
    ocean(&mut s).wavelength = 2.0;
    let waves = render(&r, &s, 1);
    assert!(
        waves.normals.iter().any(|n| n[1] < 0.97),
        "wave normals must be displaced"
    );
    let changed = waves
        .depth
        .iter()
        .zip(&clear.depth)
        .filter(|(a, b)| (*a - *b).abs() > 0.01)
        .count();
    assert!(
        changed > waves.depth.len() / 2,
        "waves change first-hit depth, not just shading"
    );
    ocean(&mut s).time = 2.0;
    let moved = render(&r, &s, 1);
    assert_ne!(waves.depth, moved.depth);
    assert!(moved.depth.iter().all(|d| *d > 0.0));
    for out in [&clear, &waves, &moved] {
        assert!(out
            .color
            .data
            .iter()
            .all(|p| p[..3].iter().all(|v| v.is_finite() && *v >= 0.0)));
    }
}

#[test]
#[ignore = "needs GPU"]
fn transparent_water_does_not_block_seabed_sunlight() {
    let r = Renderer::new(Arc::new(GpuContext::new().expect("GPU"))).unwrap();
    let mut s = sea_scene();
    s.lights = vec![Light::Sun {
        direction: Vec3::Y,
        color: Color::Linear([1.0; 3]),
        intensity: 2.0,
        angular_size: 1.0,
    }];
    let lit = mean(&render(&r, &s, 64));
    eprintln!("sunlit seabed through ocean: {lit:?}");
    assert!(lit.iter().all(|c| *c > 0.03));
    ocean(&mut s).absorption = [1.0; 3];
    let dark = mean(&render(&r, &s, 64));
    assert!(dark.iter().zip(lit).all(|(d, l)| *d < l * 0.1));
}

#[test]
#[ignore = "needs GPU"]
fn shore_foam_follows_geometry_and_internal_reflection_stays_finite() {
    let r = Renderer::new(Arc::new(GpuContext::new().expect("GPU"))).unwrap();
    let mut s = sea_scene();
    s.camera.mode = CameraMode::Free {
        position: Vec3::new(1.3, 5.0, 4.0),
        target: Vec3::new(1.3, 0.0, 0.0),
        roll: 0.0,
    };
    s.lights = vec![Light::Sun {
        direction: Vec3::Y,
        color: Color::Linear([1.0; 3]),
        intensity: 3.0,
        angular_size: 1.0,
    }];
    ocean(&mut s).seabed_color = [0.0; 3];
    let bare = mean(&render(&r, &s, 64))[0];
    ocean(&mut s).foam_amount = 1.0;
    ocean(&mut s).shore_width = 0.8;
    let shore = mean(&render(&r, &s, 64))[0];
    s.camera.mode = sea_scene().camera.mode;
    let offshore = mean(&render(&r, &s, 64))[0];
    eprintln!("foam shore {shore}, bare {bare}, offshore {offshore}");
    assert!(shore > 0.1 && shore > bare * 5.0);
    assert!(
        offshore < shore * 0.15,
        "foam cannot follow a bounding box or blanket open water"
    );

    s = sea_scene();
    // Cover the long grazing reflected path as well as its GGX tails.
    if let Light::Area { size, .. } = &mut s.lights[0] {
        *size = [100.0; 2];
    }
    s.camera.mode = CameraMode::Free {
        position: Vec3::new(8.0, -1.0, 0.0),
        target: Vec3::new(14.0, 0.0, 0.0),
        roll: 0.0,
    };
    let tir = render(&r, &s, 64);
    eprintln!("total internal reflection: {:?}", mean(&tir));
    assert!(mean(&tir)[0] > 10.0);
    assert!(tir.normals.iter().all(|n| n[1] < -0.99));
    assert!(tir
        .color
        .data
        .iter()
        .all(|p| p[..3].iter().all(|c| c.is_finite() && *c >= 0.0)));
}

#[test]
#[ignore = "needs GPU"]
fn grazing_wave_intersections_do_not_leave_sky_holes() {
    let r = Renderer::new(Arc::new(GpuContext::new().expect("GPU"))).unwrap();
    let mut s = sea_scene();
    s.camera.mode = CameraMode::Free {
        position: Vec3::new(8.0, 3.0, 4.0),
        target: Vec3::new(8.0, 0.0, -42.0),
        roll: 0.0,
    };
    for (height, wavelength) in [(0.8, 8.0), (4.0, 1.0)] {
        ocean(&mut s).wave_height = height;
        ocean(&mut s).wavelength = wavelength;
        let out = render(&r, &s, 1);
        let holes = out.depth.iter().filter(|d| **d < 0.0).count();
        eprintln!("grazing height {height}, wavelength {wavelength}, missed {holes}");
        assert_eq!(holes, 0);
        assert!(out.normals.iter().all(|n| n[1] > 0.0));
    }
}
