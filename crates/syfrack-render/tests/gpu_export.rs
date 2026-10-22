use std::{path::PathBuf, sync::Arc, time::Duration};
use syfrack_core::{Fractal, Graph, Link, Node, NodeKind, Placement, Scene};
use syfrack_render::{
    ExportMode, ExportSession, GpuContext, RenderConfig, RenderSession, Renderer,
};

fn renderer() -> Renderer {
    Renderer::new(Arc::new(GpuContext::new().unwrap())).unwrap()
}
fn finish(s: &mut ExportSession) {
    while !s.step(Duration::from_millis(20)).unwrap() {}
}
fn warm_cpu(scene: &Scene, cfg: &RenderConfig) {
    let mut warm = cfg.clone();
    warm.width = 8;
    warm.height = 8;
    warm.samples_per_pixel = 1;
    warm.tile_size = 8;
    let mut session = ExportSession::cpu(scene, &warm).unwrap();
    finish(&mut session);
    session.stop_workers();
}
fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("syfrack-export-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
fn close<const N: usize>(a: &[[f32; N]], b: &[[f32; N]], epsilon: f32) {
    assert_eq!(a.len(), b.len());
    for (i, (a, b)) in a.iter().zip(b).enumerate() {
        for c in 0..N {
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
#[ignore = "needs GPU"]
fn automatic_large_export_never_starts_a_software_cpu_worker() {
    let mut session = ExportSession::new(
        renderer(),
        &Scene::example_2d(),
        &RenderConfig::new(513, 512, 1),
        ExportMode::Auto,
    )
    .unwrap();
    let workers = session.progress().export_workers.unwrap();
    assert!(matches!(
        workers.cpu,
        syfrack_render::CpuExportStatus::NotRequested
    ));
    finish(&mut session);
    session.stop_workers();
    let ExportSession::Tiled(tiled) = session else {
        panic!("ordinary auto exports use the bounded GPU engine")
    };
    let work = tiled.work_statistics();
    assert_eq!(work.cpu_tiles, 0);
    assert!(work.gpu_tiles > 0);
    assert_eq!(work.producers.len(), 1);
}

#[test]
#[ignore = "needs GPU"]
fn compact_tiles_match_resident_camera_rng_layers_and_material_guides() {
    let r = renderer();
    let mut scene = Scene::example();
    scene.atmosphere.fog_density = 0.025;
    let mut cfg = RenderConfig::new(263, 251, 3);
    cfg.tile_size = 16;
    cfg.denoise = true;
    cfg.max_bounces = 2;
    let mut reference = RenderSession::new(r.clone(), &scene, &cfg).unwrap();
    while !reference.step(Duration::from_millis(20)).unwrap() {}
    let expected = reference.readback().unwrap();
    let mut tiled = ExportSession::new(r, &scene, &cfg, ExportMode::Tiled).unwrap();
    finish(&mut tiled);
    if let ExportSession::Tiled(s) = &tiled {
        assert!(s.device_buffer_bytes() <= 2 * 128 * 128 * 392);
        assert!(s.device_buffer_bytes() > 2 * 64 * 64 * 392);
    }
    let actual = tiled.readback().unwrap();
    close(&expected.color.data, &actual.color.data, 1e-6);
    assert_eq!(expected.depth, actual.depth);
    close(&expected.normals, &actual.normals, 1e-6);
    assert_eq!((actual.samples_min, actual.samples_max), (3, 3));
    let (a, b) = (expected.layers.unwrap(), actual.layers.unwrap());
    close(&a.surface.data, &b.surface.data, 1e-6);
    close(&a.volume.data, &b.volume.data, 1e-6);
    assert_eq!(a.volume_variance, b.volume_variance);
    let (a, b) = (a.components.unwrap(), b.components.unwrap());
    close(&a.direct.data, &b.direct.data, 1e-6);
    assert_eq!(a.direct_variance, b.direct_variance);
    assert_eq!(a.indirect_variance, b.indirect_variance);
    let (a, n) = reference.export_guides().unwrap();
    let (b, m) = tiled.export_guides().unwrap();
    close(&a.data, &b.data, 1e-6);
    close(&n.data, &m.data, 1e-6);
    let (a, n) = reference.export_surface_guides().unwrap();
    let (b, m) = tiled.export_surface_guides().unwrap();
    close(&a.data, &b.data, 1e-6);
    close(&n.data, &m.data, 1e-6);
}

#[test]
#[ignore = "needs GPU"]
fn wavefront_preserves_hdr_layers_guides_and_checkpoint_on_clipped_tiles() {
    let r = renderer();
    let mut scene = Scene::example();
    scene.atmosphere.fog_density = 0.025;
    let mut cfg = RenderConfig::new(137, 79, 3);
    cfg.max_bounces = 2;
    cfg.denoise = true;
    cfg.tile_size = 16;
    let mut reference = ExportSession::new(r.clone(), &scene, &cfg, ExportMode::Tiled).unwrap();
    finish(&mut reference);
    let expected = reference.readback().unwrap();
    let mut wave = ExportSession::new(r.clone(), &scene, &cfg, ExportMode::Wavefront).unwrap();
    while wave.progress().samples_max == 0 {
        wave.step(Duration::ZERO).unwrap();
    }
    let dir = temp("wavefront").join("render.state");
    wave.save(&dir).unwrap();
    drop(wave);
    let mut wave = ExportSession::resume(r.clone(), &dir).unwrap();
    finish(&mut wave);
    let actual = wave.readback().unwrap();
    syfrack_render::hybrid_benchmark::compare(&expected.color, &actual.color)
        .unwrap()
        .require(syfrack_render::hybrid_benchmark::ImageTolerance::default())
        .unwrap();
    close(&expected.color.data, &actual.color.data, 1e-4);
    assert_eq!(expected.depth, actual.depth);
    close(&expected.normals, &actual.normals, 1e-5);
    let (a, b) = (expected.layers.unwrap(), actual.layers.unwrap());
    close(&a.surface.data, &b.surface.data, 1e-4);
    close(&a.volume.data, &b.volume.data, 1e-4);
    close(&a.volume_variance, &b.volume_variance, 1e-4);
    let (a, b) = (a.components.unwrap(), b.components.unwrap());
    close(&a.direct.data, &b.direct.data, 1e-4);
    close(&a.direct_variance, &b.direct_variance, 1e-4);
    close(&a.indirect_variance, &b.indirect_variance, 1e-4);
    let mut full = ExportSession::new(r, &scene, &cfg, ExportMode::Auto).unwrap();
    assert_eq!(
        full.config().integrator,
        syfrack_render::ComputeIntegrator::Wavefront
    );
    finish(&mut full);
    assert_eq!(
        actual.color,
        full.readback().unwrap().color,
        "wavefront resume changed samples"
    );
    let (a, n) = reference.export_guides().unwrap();
    let (b, m) = wave.export_guides().unwrap();
    close(&a.data, &b.data, 1e-4);
    close(&n.data, &m.data, 1e-4);
}

#[test]
#[ignore = "needs GPU"]
fn wavefront_keeps_cutout_null_events_and_dielectric_medium_state() {
    let r = renderer();
    let mut scene = Scene::example();
    let mut cfg = RenderConfig::new(17, 13, 4);
    cfg.max_bounces = 4;
    for optics in [
        syfrack_core::Optics {
            opacity: 0.0,
            ..Default::default()
        },
        syfrack_core::Optics {
            ior: 1.0,
            absorption_color: [0.5, 0.25, 1.0],
            ..Default::default()
        },
    ] {
        scene.material.optics = Some(optics);
        let mut a = ExportSession::new(r.clone(), &scene, &cfg, ExportMode::Tiled).unwrap();
        finish(&mut a);
        let mut b = ExportSession::new(r.clone(), &scene, &cfg, ExportMode::Wavefront).unwrap();
        finish(&mut b);
        close(
            &a.readback().unwrap().color.data,
            &b.readback().unwrap().color.data,
            1e-4,
        );
    }
}

#[test]
#[ignore = "needs GPU"]
fn flat_canvas_uses_global_coordinates_on_clipped_tiles() {
    let r = renderer();
    let scene = Scene::example_2d();
    let mut cfg = RenderConfig::new(41, 19, 2);
    cfg.tile_size = 16;
    let mut reference = RenderSession::new(r.clone(), &scene, &cfg).unwrap();
    while !reference.step(Duration::from_millis(50)).unwrap() {}
    let mut tiled = ExportSession::new(r, &scene, &cfg, ExportMode::Tiled).unwrap();
    finish(&mut tiled);
    close(
        &reference.readback().unwrap().color.data,
        &tiled.readback().unwrap().color.data,
        1e-6,
    );
}

#[test]
#[ignore = "needs GPU"]
fn tiled_checkpoint_resumes_exactly_and_rejects_corruption() {
    let r = renderer();
    let scene = Scene::example();
    let mut cfg = RenderConfig::new(263, 251, 3);
    cfg.tile_size = 16;
    cfg.denoise = true;
    let dir = temp("checkpoint").join("render.state");
    let mut s = ExportSession::new(r.clone(), &scene, &cfg, ExportMode::Tiled).unwrap();
    while s.progress().samples_max == 0 {
        s.step(Duration::ZERO).unwrap();
    }
    s.save(&dir).unwrap();
    drop(s);
    let mut restored = ExportSession::resume(r.clone(), &dir).unwrap();
    finish(&mut restored);
    let mut full = ExportSession::new(r.clone(), &scene, &cfg, ExportMode::Tiled).unwrap();
    finish(&mut full);
    assert_eq!(
        full.readback().unwrap().color,
        restored.readback().unwrap().color
    );
    // Image equality alone does not establish byte-identical deliverables.
    // Exercise the complete global Look and both encoders with identical scene
    // metadata, including exposure, dithering and grain after tile assembly.
    let mut look = scene.look.clone();
    look.auto_exposure = Some(0.18);
    look.grain = Some(syfrack_core::Grain {
        amount: 0.015,
        size: 1.0,
        seed: 37,
    });
    let ron = scene.to_ron();
    for (name, session) in [("full", &full), ("resumed", &restored)] {
        let output = session.readback().unwrap();
        syfrack_color::write_exr(dir.with_extension(format!("{name}.exr")), &output.color).unwrap();
        let denoised = syfrack_render::denoise::session_output(session, &output);
        let png = syfrack_color::develop_and_encode(&denoised, &look, 16);
        syfrack_color::write_png(
            dir.with_extension(format!("{name}.png")),
            &png,
            look.output,
            Some(&ron),
        )
        .unwrap();
    }
    for extension in ["exr", "png"] {
        let a = std::fs::read(dir.with_extension(format!("full.{extension}"))).unwrap();
        let b = std::fs::read(dir.with_extension(format!("resumed.{extension}"))).unwrap();
        assert!(
            a == b,
            "{extension} bytes differ: lengths {} / {}, first offset {:?}",
            a.len(),
            b.len(),
            a.iter().zip(&b).position(|(a, b)| a != b)
        );
    }
    restored.save(&dir).unwrap();
    drop(restored);
    let mut bytes = std::fs::read(dir.join("export.bin")).unwrap();
    bytes[7] ^= 1;
    std::fs::write(dir.join("export.bin"), bytes).unwrap();
    assert!(ExportSession::resume(r, &dir)
        .err()
        .unwrap()
        .to_string()
        .contains("checksum"));
}

#[test]
#[ignore = "needs GPU and a software CPU adapter"]
fn hybrid_uses_both_processors_and_commits_each_tile_once() {
    let r = renderer();
    let scene = Scene::example_2d();
    // Several GPU blocks leave real work for an asynchronously prepared CPU
    // worker. A single block can legitimately finish before CPU preparation.
    let mut cfg = RenderConfig::new(192, 128, 2);
    cfg.tile_size = 16;
    warm_cpu(&scene, &cfg);
    let mut s = ExportSession::new(r, &scene, &cfg, ExportMode::Hybrid).unwrap();
    finish(&mut s);
    let ExportSession::Tiled(tiled) = &s else {
        panic!("expected tiled export")
    };
    let (cpu, gpu) = tiled.worker_tiles();
    assert!(cpu > 0 && gpu > 0, "cpu={cpu}, gpu={gpu}");
    assert_eq!(cpu + gpu, 96);
    assert!(s
        .readback()
        .unwrap()
        .color
        .data
        .iter()
        .all(|p| p.iter().all(|c| c.is_finite())));
}

#[test]
#[ignore = "needs GPU and a software CPU adapter"]
fn hybrid_3d_uses_the_same_light_transport() {
    use syfrack_core::{Color, Coloring, Light};
    let r = renderer();
    let mut scene = Scene::example();
    scene.fractal = Fractal::Graph(Graph::new_minimal());
    scene.material.coloring = Coloring::None;
    scene.material.albedo = [0.5; 3];
    scene.material.metallic = 0.0;
    scene.material.specular = 0.0;
    scene.material.roughness = 1.0;
    scene.lights = vec![Light::Environment {
        zenith: Color::Linear([1.0; 3]),
        horizon: Color::Linear([1.0; 3]),
        ground: Color::Linear([1.0; 3]),
        intensity: 1.0,
        rotation: 0.0,
    }];
    let mut cfg = RenderConfig::new(192, 128, 4);
    cfg.tile_size = 8;
    cfg.max_bounces = 1;
    let reference = r.render(&scene, &cfg, &mut |_| {}).unwrap();
    warm_cpu(&scene, &cfg);
    let mut hybrid = ExportSession::new(r, &scene, &cfg, ExportMode::Hybrid).unwrap();
    finish(&mut hybrid);
    let ExportSession::Tiled(s) = &hybrid else {
        panic!("expected tiled export")
    };
    let (cpu, gpu) = s.worker_tiles();
    assert!(cpu > 0 && gpu > 0, "cpu={cpu}, gpu={gpu}");
    let actual = hybrid.readback().unwrap();
    let mse = actual
        .color
        .data
        .iter()
        .zip(&reference.color.data)
        .flat_map(|(a, b)| (0..3).map(move |c| (a[c] - b[c]).powi(2)))
        .sum::<f32>()
        / (cfg.width as f32 * cfg.height as f32 * 3.0);
    assert!(mse < 1e-4, "CPU/GPU transport MSE={mse}");
}

#[test]
#[ignore = "needs GPU and software CPU adapter"]
fn hybrid_benchmark_measures_both_workers_and_validates_linear_hdr() {
    use syfrack_core::{Color, Coloring, Light};
    let r = renderer();
    let mut scene = Scene::example();
    scene.fractal = Fractal::Graph(Graph::new_minimal());
    scene.material.coloring = Coloring::None;
    scene.material.albedo = [0.5; 3];
    scene.material.metallic = 0.0;
    scene.material.specular = 0.0;
    scene.material.roughness = 1.0;
    scene.lights = vec![Light::Environment {
        zenith: Color::Linear([1.0; 3]),
        horizon: Color::Linear([1.0; 3]),
        ground: Color::Linear([1.0; 3]),
        intensity: 1.0,
        rotation: 0.0,
    }];
    let mut cfg = RenderConfig::new(192, 128, 2);
    cfg.tile_size = 8;
    cfg.max_bounces = 1;
    let report = syfrack_render::hybrid_benchmark::benchmark(
        &r,
        &scene,
        &cfg,
        Default::default(),
        &mut |_| {},
    )
    .unwrap();
    report.require_quality().unwrap();
    report.require_concurrent_work().unwrap();
    assert_eq!(report.measurements.len(), 3);
    let gpu = &report.measurements[0];
    let cpu = &report.measurements[1];
    let hybrid = &report.measurements[2];
    assert_eq!((gpu.workers.cpu_tiles, gpu.workers.gpu_tiles), (0, 384));
    assert_eq!((cpu.workers.cpu_tiles, cpu.workers.gpu_tiles), (384, 0));
    assert!(hybrid.workers.cpu_tiles > 0 && hybrid.workers.gpu_tiles > 0);
    assert_eq!(hybrid.workers.cpu_tiles + hybrid.workers.gpu_tiles, 384);
    let timing = &hybrid.workers.dispatch_timing;
    assert!(timing.cpu.dispatches > 0 && timing.gpu.dispatches > 0);
    assert!(
        timing.overlap_wall_ms > 0.0,
        "workers never overlapped: {timing:?}"
    );
    assert_eq!(gpu.workers.dispatch_timing.overlap_wall_ms, 0.0);
    assert_eq!(cpu.workers.dispatch_timing.overlap_wall_ms, 0.0);
    assert!(report.hybrid_speedup_over_gpu.is_finite() && report.hybrid_speedup_over_gpu > 0.0);
    // A measured speedup is device/workload-dependent, never a flaky test assertion.
}

#[test]
#[ignore = "needs GPU and software CPU adapter"]
fn explicit_backend_resume_preserves_completed_tiles_and_producer_history() {
    use syfrack_core::{Color, Coloring, Light};
    let r = renderer();
    let mut scene = Scene::example();
    scene.fractal = Fractal::Graph(Graph::new_minimal());
    scene.material.coloring = Coloring::None;
    scene.material.albedo = [0.5; 3];
    scene.material.metallic = 0.0;
    scene.material.specular = 0.0;
    scene.material.roughness = 1.0;
    scene.lights = vec![Light::Environment {
        zenith: Color::Linear([1.0; 3]),
        horizon: Color::Linear([1.0; 3]),
        ground: Color::Linear([1.0; 3]),
        intensity: 1.0,
        rotation: 0.0,
    }];
    let mut cfg = RenderConfig::new(96, 64, 4);
    cfg.tile_size = 8;
    cfg.max_bounces = 1;
    let mut reference = ExportSession::new(r.clone(), &scene, &cfg, ExportMode::Tiled).unwrap();
    finish(&mut reference);
    let expected = reference.readback().unwrap().color;
    reference.stop_workers();
    for (from, to) in [
        (ExportMode::Cpu, ExportMode::Tiled),
        (ExportMode::Tiled, ExportMode::Cpu),
    ] {
        let dir = temp(&format!("switch-{from:?}"));
        let mut original = ExportSession::new(r.clone(), &scene, &cfg, from).unwrap();
        while original.progress().samples_max == 0 {
            original.step(Duration::ZERO).unwrap();
        }
        assert!(!original.is_done(), "test must switch with remaining work");
        original.save(&dir).unwrap();
        original.stop_workers();
        let saved = original.readback().unwrap().color;
        let state: syfrack_render::SessionState =
            ron::from_str(&std::fs::read_to_string(dir.join("state.ron")).unwrap()).unwrap();
        let mut resumed = ExportSession::resume_with_backend(r.clone(), &dir, to).unwrap();
        finish(&mut resumed);
        resumed.stop_workers();
        let actual = resumed.readback().unwrap().color;
        for tile in state.tiles.iter().filter(|t| t.done > 0) {
            for y in tile.y..tile.y + tile.h {
                for x in tile.x..tile.x + tile.w {
                    assert_eq!(
                        saved.get(x, y).map(f32::to_bits),
                        actual.get(x, y).map(f32::to_bits)
                    );
                }
            }
        }
        syfrack_render::hybrid_benchmark::compare(&expected, &actual)
            .unwrap()
            .require(Default::default())
            .unwrap();
        resumed.save(&dir).unwrap();
        let metadata = std::fs::read_to_string(dir.join("state.ron")).unwrap();
        let history = metadata
            .split_once("previous_producers:")
            .unwrap()
            .1
            .trim_start();
        assert!(
            history
                .strip_prefix('[')
                .unwrap()
                .trim_start()
                .starts_with('('),
            "producer history missing"
        );
        assert!(metadata.contains("device_type: \"Cpu\""));
        // The test owns this uniquely named directory and its checkpoint files.
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
#[ignore = "needs GPU"]
fn thousands_of_rotated_instances_use_the_same_fenced_field_as_cpu() {
    use glam::Vec3;
    let mut g = Graph::new_minimal();
    let placements = (0..2049)
        .map(|i| Placement {
            position: Vec3::new(
                (i % 47) as f32 * 2.2,
                (i / 47) as f32 * 2.1,
                (i % 7) as f32 * 0.13,
            ),
            angles: Vec3::new((i as f32 * 1.7) % 360.0 - 180.0, 13.0, -29.0),
            scale: Vec3::new(0.7, 1.1, 0.9),
        })
        .collect();
    g.nodes.push(Node {
        id: 3,
        kind: NodeKind::Instances {
            placements,
            half: Vec3::splat(1.0),
            margin: 0.01,
        },
        name: "copies".into(),
        pos: [0.0; 2],
        enabled: true,
    });
    g.links.retain(|l| l.to.0 != 2);
    g.links.extend([
        Link {
            from: (1, 0),
            to: (3, 0),
        },
        Link {
            from: (3, 0),
            to: (2, 0),
        },
    ]);
    g.validate().unwrap();
    let plan = syfrack_core::graph::eval::PreparedGraph::new(&g).unwrap();
    let points: Vec<_> = (0..159)
        .map(|i| Vec3::new((i % 47) as f32 * 2.2 - 0.2, (i / 47) as f32 * 2.1, 0.37))
        .collect();
    let expected: Vec<_> = points.iter().map(|p| plan.evaluate(*p)).collect();
    let mut scene = Scene::example();
    scene.fractal = Fractal::Graph(g);
    let actual = renderer().probe_distances(&scene, &points).unwrap();
    for (a, b) in actual.iter().zip(expected) {
        assert!((a.distance - b.distance).abs() < 2e-4, "{a:?} != {b:?}");
        assert_eq!(a.material, b.material);
    }
}
