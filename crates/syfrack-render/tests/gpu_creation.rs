//! End-to-end contract of the unified scene graph, not three image compositors.
use std::{sync::Arc, time::Duration};
use syfrack_core::{scene::phenomena::*, *};
use syfrack_render::{
    control::ControlSession, ExportMode, ExportSession, GpuContext, RenderConfig, Renderer,
};

fn mixed() -> Scene {
    let mut scene = Scene::example_2d();
    let mut g = CreationGraph::from_scene(&scene);
    g.nodes
        .retain(|n| matches!(n.kind, CreationKind::Form(_) | CreationKind::Output));
    let retained: Vec<_> = g.nodes.iter().map(|n| n.id).collect();
    g.links
        .retain(|l| retained.contains(&l.from.0) && retained.contains(&l.to.0));
    let out = g
        .nodes
        .iter()
        .find(|n| matches!(n.kind, CreationKind::Output))
        .unwrap()
        .id;
    g.settings.domain.resolution = [8, 8, 1];
    let point = g.add(
        CreationKind::Field(FieldOp::Vector {
            value: [0.11, 0.27, 0.0].into(),
        }),
        [0.0, 0.0],
    );
    let pattern = g.add(
        CreationKind::Pattern {
            fractal: PlanarFractal {
                julia: [0.2, 0.3],
                iterations: 24,
                ..Default::default()
            },
            channels: OrbitColoring::default(),
        },
        [250.0, 0.0],
    );
    let flame = g.add(
        CreationKind::Flame(FlameField {
            resolution: 64,
            points: 262144,
            ..Default::default()
        }),
        [250.0, 200.0],
    );
    let wave = g.add(
        CreationKind::Field(FieldOp::PlaneWave {
            wavevector: [4.0, 2.0, 0.0].into(),
            frequency: 1.0,
            phase: 0.3,
        }),
        [250.0, 400.0],
    );
    let phase = g.add(CreationKind::Field(FieldOp::Phase), [500.0, 400.0]);
    let palette = g.add(
        CreationKind::Palette(Ramp {
            stops: vec![[0.03, 0.2, 0.7], [0.8, 0.3, 0.05]],
            ..Default::default()
        }),
        [500.0, 0.0],
    );
    let addition = g.add(CreationKind::Field(FieldOp::Add), [750.0, 200.0]);
    let product = g.add(CreationKind::Field(FieldOp::Scale), [1000.0, 200.0]);
    for (a, b) in [
        ((point, 0), (pattern, 0)),
        ((point, 0), (flame, 0)),
        ((point, 0), (wave, 0)),
        ((wave, 0), (phase, 0)),
        ((pattern, 0), (palette, 0)),
        ((palette, 0), (addition, 0)),
        ((flame, 0), (addition, 1)),
        ((addition, 0), (product, 0)),
        ((phase, 0), (product, 1)),
        ((product, 0), (out, 1)),
    ] {
        g.connect(a, b).unwrap();
    }
    scene.creation = Some(g);
    scene.unify().unwrap();
    scene
}
fn finish(s: &mut ExportSession) {
    while !s.step(Duration::from_millis(20)).unwrap() {}
}
fn close(a: &[[f32; 4]], b: &[[f32; 4]], epsilon: f32) {
    assert_eq!(a.len(), b.len());
    for (i, (a, b)) in a.iter().zip(b).enumerate() {
        for c in 0..4 {
            assert!(
                (a[c] - b[c]).abs() <= epsilon * a[c].abs().max(1.0),
                "pixel {i} channel {c}: {} != {}",
                a[c],
                b[c]
            );
        }
    }
}

#[test]
#[ignore = "requires physical GPU"]
fn shared_fields_control_native_tiles_freeze_and_resume() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    eprintln!("adapter {:?}", r.context().info);
    let mut scene = mixed();
    let mut cfg = RenderConfig::new(16, 12, 3);
    cfg.max_bounces = 1;
    cfg.max_steps = 64;
    cfg.tile_size = 8;
    cfg.samples_per_dispatch = 1;
    eprintln!("prepare three connected sources");
    let prepared = r.prepare_phenomena(&scene).unwrap().unwrap();
    assert_eq!(prepared.flames.len(), 1);
    let atlas = &prepared.flames[0];
    let (peak, pixel) = atlas
        .values
        .iter()
        .enumerate()
        .max_by(|a, b| a.1[3].total_cmp(&b.1[3]))
        .unwrap();
    assert!(pixel[0] + pixel[1] + pixel[2] > 0.01);
    let n = atlas.resolution[0];
    let point = [
        ((peak as u32 % n) as f32 + 0.5) / n as f32 * 6.0 - 3.0,
        3.0 - ((peak as u32 / n) as f32 + 0.5) / n as f32 * 6.0,
        0.0,
    ];
    if let CreationKind::Field(FieldOp::Vector { value }) = &mut scene
        .creation
        .as_mut()
        .unwrap()
        .nodes
        .iter_mut()
        .find(|n| matches!(n.kind, CreationKind::Field(FieldOp::Vector { .. })))
        .unwrap()
        .kind
    {
        *value = point.into();
    }
    scene.unify().unwrap();
    let frozen = r.freeze_creation_sources(&scene).unwrap();
    let frozen = Scene::from_ron(&frozen.to_ron()).unwrap();
    assert!(frozen
        .creation
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .any(|n| matches!(&n.kind,CreationKind::Flame(f) if f.frozen.is_some())));
    let clock = std::time::Instant::now();
    eprintln!("fixed live control: construct");
    let mut live = ControlSession::new(r.clone(), &scene, (16, 12)).unwrap();
    eprintln!("fixed live control: initialized at {:?}", clock.elapsed());
    live.wait_complete().unwrap();
    eprintln!("fixed live control: completed at {:?}", clock.elapsed());
    let bytes = r
        .context()
        .read_buffer(live.accumulation_buffer(), 16 * 12 * 16)
        .unwrap();
    let pixels: Vec<[f32; 4]> = bytemuck::pod_collect_to_vec(&bytes);
    eprintln!("native render at {:?}", clock.elapsed());
    let native = r.render(&scene, &cfg, &mut |_| {}).unwrap();
    assert_eq!(native.samples_min, 3);
    close(&native.color.data, &pixels, 2e-4);
    let mut ablated = scene.clone();
    if let CreationKind::Flame(f) = &mut ablated
        .creation
        .as_mut()
        .unwrap()
        .nodes
        .iter_mut()
        .find(|n| matches!(n.kind, CreationKind::Flame(_)))
        .unwrap()
        .kind
    {
        f.flame.brightness = 0.0;
    }
    ablated.unify().unwrap();
    let dark = r.render(&ablated, &cfg, &mut |_| {}).unwrap();
    assert!(
        native
            .color
            .data
            .iter()
            .zip(&dark.color.data)
            .any(|(a, b)| (a[0] - b[0]).abs() + (a[1] - b[1]).abs() + (a[2] - b[2]).abs() > 0.001),
        "flame must contribute to the final image"
    );
    eprintln!("frozen render");
    let fixed = r.render(&frozen, &cfg, &mut |_| {}).unwrap();
    close(&native.color.data, &fixed.color.data, 0.0);
    eprintln!("bounded GPU export");
    let mut tiled = ExportSession::new(r.clone(), &scene, &cfg, ExportMode::Wavefront).unwrap();
    finish(&mut tiled);
    assert!(matches!(
        tiled.progress().export_workers.unwrap().cpu,
        syfrack_render::CpuExportStatus::NotRequested
    ));
    let result = tiled.readback().unwrap();
    close(&native.color.data, &result.color.data, 2e-4);
    tiled.stop_workers();
    eprintln!("canonical checkpoint resume");
    let dir =
        std::path::PathBuf::from("target").join(format!("creation-resume-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut partial = ExportSession::new(r.clone(), &scene, &cfg, ExportMode::Wavefront).unwrap();
    partial.step(Duration::from_millis(1)).unwrap();
    partial.save(&dir).unwrap();
    partial.stop_workers();
    drop(partial);
    let state = std::fs::read_to_string(dir.join("state.ron")).unwrap();
    assert!(state.contains("creation:"));
    let mut resumed = ExportSession::resume(r.clone(), &dir).unwrap();
    finish(&mut resumed);
    let final_image = resumed.readback().unwrap();
    resumed.stop_workers();
    close(&result.color.data, &final_image.color.data, 0.0);
    eprintln!("same graph in 3D");
    let mut solid = scene.clone();
    solid.dimension = Dimension::ThreeD;
    solid.lights = Scene::example().lights;
    solid.unify().unwrap();
    let image = r.render(&solid, &cfg, &mut |_| {}).unwrap();
    assert!(image.color.data.iter().flatten().all(|v| v.is_finite()));
    assert!(image.color.data.iter().any(|p| p[0] + p[1] + p[2] > 0.01));
    eprintln!("HDR field radiance and bounded surface reflectance");
    let mut energy = scene.clone();
    let g = energy.creation.as_mut().unwrap();
    let out = g
        .nodes
        .iter()
        .find(|n| matches!(n.kind, CreationKind::Output))
        .unwrap()
        .id;
    let rgb = g.add(
        CreationKind::Field(FieldOp::Vector {
            value: [32.0; 3].into(),
        }),
        [0.0; 2],
    );
    g.connect((rgb, 0), (out, 1)).unwrap();
    energy.unify().unwrap();
    let hdr = r.render(&energy, &cfg, &mut |_| {}).unwrap();
    assert!(hdr.color.data.iter().any(|p| p[0] > 31.9));
    energy.dimension = Dimension::ThreeD;
    energy.lights = Scene::example().lights;
    energy.unify().unwrap();
    let bright = r.render(&energy, &cfg, &mut |_| {}).unwrap();
    if let CreationKind::Field(FieldOp::Vector { value }) = &mut energy
        .creation
        .as_mut()
        .unwrap()
        .nodes
        .iter_mut()
        .find(|n| n.id == rgb)
        .unwrap()
        .kind
    {
        *value = [1.0; 3].into();
    }
    energy.unify().unwrap();
    let white = r.render(&energy, &cfg, &mut |_| {}).unwrap();
    close(&bright.color.data, &white.color.data, 0.0);
    eprintln!("unified graph acceptance complete");
}

#[test]
#[ignore = "requires physical GPU"]
fn pattern_and_flame_drive_the_potential_of_a_shared_quantum_state() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = mixed();
    let g = scene.creation.as_mut().unwrap();
    g.settings.domain.resolution = [16, 16, 1];
    g.settings.domain.half = [1.0; 3].into();
    let out = g
        .nodes
        .iter()
        .find(|n| matches!(n.kind, CreationKind::Output))
        .unwrap()
        .id;
    let pattern = g
        .nodes
        .iter()
        .find(|n| matches!(n.kind, CreationKind::Pattern { .. }))
        .unwrap()
        .id;
    let flame = g
        .nodes
        .iter()
        .find(|n| matches!(n.kind, CreationKind::Flame(_)))
        .unwrap()
        .id;
    let position = g.add(CreationKind::Field(FieldOp::Position), [0.0; 2]);
    g.connect((position, 0), (pattern, 0)).unwrap();
    g.connect((position, 0), (flame, 0)).unwrap();
    let product = g.add(CreationKind::Field(FieldOp::Multiply), [0.0; 2]);
    let limit = g.add(
        CreationKind::Field(FieldOp::Clamp { min: 0.0, max: 2.0 }),
        [0.0; 2],
    );
    let sim = g.add(
        CreationKind::Simulation(Simulation {
            model: SimulationModel::GrossPitaevskii {
                dispersion: 0.03,
                interaction: 0.5,
                damping: 0.0,
            },
            steps: 12,
            dt: 0.001,
            ..Default::default()
        }),
        [0.0; 2],
    );
    let sample = g.add(CreationKind::SampleState { channel: 3 }, [0.0; 2]);
    let palette = g.add(CreationKind::Palette(Ramp::default()), [0.0; 2]);
    for (a, b) in [
        ((pattern, 0), (product, 0)),
        ((flame, 1), (product, 1)),
        ((product, 0), (limit, 0)),
        ((position, 0), (sim, 0)),
        ((limit, 0), (sim, 1)),
        ((sim, 4), (sample, 0)),
        ((position, 0), (sample, 1)),
        ((sample, 0), (palette, 0)),
        ((palette, 0), (out, 1)),
    ] {
        g.connect(a, b).unwrap();
    }
    scene.unify().unwrap();
    let prepared = r.prepare_phenomena(&scene).unwrap().unwrap();
    assert_eq!(prepared.snapshots.len(), 1);
    let state = &prepared.snapshots[0];
    assert!(state.data.iter().all(|p| p.is_finite()));
    let mut independent = scene.clone();
    independent
        .creation
        .as_mut()
        .unwrap()
        .links
        .retain(|l| l.to != (sim, 1));
    independent.unify().unwrap();
    let free = r.prepare_phenomena(&independent).unwrap().unwrap();
    let delta: f32 = state
        .data
        .iter()
        .zip(&free.snapshots[0].data)
        .map(|(a, b)| (*a - *b).abs().element_sum())
        .sum();
    assert!(
        delta > 1e-5,
        "the connected generators must change the PDE, delta={delta}"
    );
    let frozen = r.freeze_creation_sources(&scene).unwrap();
    let frozen = Scene::from_ron(&frozen.to_ron()).unwrap();
    let restored = r.prepare_phenomena(&frozen).unwrap().unwrap();
    assert_eq!(state.data, restored.snapshots[0].data);
}

#[test]
#[ignore = "requires physical GPU"]
fn collapsed_flame_carries_integer_moments_past_u32_and_preserves_hue() {
    let r = Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap();
    let mut scene = mixed();
    let g = scene.creation.as_mut().unwrap();
    let f = g
        .nodes
        .iter_mut()
        .find_map(|n| {
            if let CreationKind::Flame(f) = &mut n.kind {
                Some(f)
            } else {
                None
            }
        })
        .unwrap();
    f.points = 1_048_576;
    f.resolution = 32;
    f.flame.transforms = vec![FlameTransform {
        affine: [0.0; 6],
        terms: vec![FlameTerm {
            amount: 1.0,
            variation: FlameVariation::Linear,
        }],
        ..Default::default()
    }];
    f.flame.final_transform = None;
    f.flame.background = [0.0; 3];
    f.flame.brightness = 1.0;
    f.flame.density_log = 0.0;
    f.flame.palette = Ramp {
        stops: vec![[0.125, 0.5, 0.875]; 2],
        ..Default::default()
    };
    scene.unify().unwrap();
    let prepared = r.prepare_phenomena(&scene).unwrap().unwrap();
    let atlas = &prepared.flames[0];
    let peak = atlas.values.iter().map(|p| p[3]).fold(0.0f32, f32::max);
    assert!(peak > 100.0, "lost u32 carry: {peak}");
    for p in atlas.values.iter().filter(|p| p[3] > 1.0) {
        for (c, expected) in [0.125, 0.5, 0.875].into_iter().enumerate() {
            assert!((p[c] / p[3] - expected).abs() < 2e-5, "hue changed: {p:?}");
        }
    }
}
