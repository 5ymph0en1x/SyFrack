use std::process::Command;
use syfrack_core::{Coloring, Fractal, Graph, Scene};

fn tmp() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("syfrack-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn version_reports_the_cli_package_version_without_a_subcommand() {
    for flag in ["--version", "-V"] {
        let output = Command::new(env!("CARGO_BIN_EXE_syfrack"))
            .args(["--lang", "fr", flag])
            .output()
            .unwrap();
        assert!(output.status.success(), "{flag}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            format!("syfrack {}", env!("CARGO_PKG_VERSION"))
        );
    }
}

#[test]
fn parity_command_requires_a_bounded_nonempty_corpus_before_creating_a_device() {
    let output = Command::new(env!("CARGO_BIN_EXE_syfrack"))
        .args(["parity", "--points", "0", "--out", "unused-parity.ron"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("16..=1000000"));
    let help = Command::new(env!("CARGO_BIN_EXE_syfrack"))
        .args(["parity", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("--report-only"));
}

#[test]
fn example_scene_command_writes_a_valid_scene() {
    let out = tmp().join("example.ron");
    let status = Command::new(env!("CARGO_BIN_EXE_syfrack"))
        .args(["example-scene", "--out"])
        .arg(&out)
        .status()
        .unwrap();
    assert!(status.success());
    let scene = Scene::load(&out).unwrap();
    // Example scenes are written as graph scenes.
    assert_eq!(scene, Scene::example());
}

#[test]
#[ignore = "needs GPU"]
fn render_writes_exr_png_and_twin_ron_with_matching_scene() {
    let dir = tmp();
    let scene_path = dir.join("scene.ron");
    let mut scene = Scene::example();
    scene.seed = 99;
    scene.save(&scene_path).unwrap();

    let out = dir.join("render");
    let status = Command::new(env!("CARGO_BIN_EXE_syfrack"))
        .arg("render")
        .arg(&scene_path)
        .args(["--width", "64", "--height", "48", "--samples", "4", "--out"])
        .arg(&out)
        .status()
        .unwrap();
    assert!(status.success());

    let exr = syfrack_color::read_exr(dir.join("render.exr")).unwrap();
    assert_eq!((exr.width, exr.height), (64, 48));
    assert!(exr.data.iter().any(|p| p[0] > 0.0), "image is not black");

    let text = syfrack_color::read_png_text(dir.join("render.png"), syfrack_color::SCENE_KEYWORD)
        .unwrap()
        .expect("png carries the scene");
    assert_eq!(Scene::from_ron(&text).unwrap(), scene);
    assert_eq!(Scene::load(dir.join("render.ron")).unwrap(), scene);
}

#[test]
#[ignore = "needs GPU"]
fn grade_reprocesses_an_exr_with_a_different_look() {
    let dir = tmp();
    let scene_path = dir.join("scene.ron");
    Scene::example().save(&scene_path).unwrap();
    let out = dir.join("g");
    assert!(Command::new(env!("CARGO_BIN_EXE_syfrack"))
        .arg("render")
        .arg(&scene_path)
        .args(["--width", "32", "--height", "32", "--samples", "2", "--out"])
        .arg(&out)
        .status()
        .unwrap()
        .success());

    let mut dark = Scene::example();
    dark.look.exposure_ev = -3.0;
    let dark_path = dir.join("dark.ron");
    dark.save(&dark_path).unwrap();
    let graded = dir.join("dark.png");
    assert!(Command::new(env!("CARGO_BIN_EXE_syfrack"))
        .arg("grade")
        .arg(dir.join("g.exr"))
        .arg("--scene")
        .arg(&dark_path)
        .args(["--bits", "8", "--out"])
        .arg(&graded)
        .status()
        .unwrap()
        .success());
    let text = syfrack_color::read_png_text(&graded, syfrack_color::SCENE_KEYWORD)
        .unwrap()
        .unwrap();
    assert_eq!(Scene::from_ron(&text).unwrap().look.exposure_ev, -3.0);
}

#[test]
fn grade_rejects_bad_bits() {
    let out = Command::new(env!("CARGO_BIN_EXE_syfrack"))
        .args([
            "grade",
            "missing.exr",
            "--bits",
            "12",
            "--look",
            "missing.ron",
            "--out",
        ])
        .arg(tmp().join("bad.png"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--bits"));
}

#[test]
fn render_rejects_negative_time_budget() {
    let dir = tmp();
    let scene_path = dir.join("tb.ron");
    Scene::example().save(&scene_path).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_syfrack"))
        .arg("render")
        .arg(&scene_path)
        .args(["--time-budget", "-1", "--out"])
        .arg(dir.join("tb"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--time-budget"));
}

#[test]
#[ignore = "needs GPU"]
fn render_keeps_dotted_output_names() {
    let dir = tmp();
    let scene_path = dir.join("dot.ron");
    Scene::example().save(&scene_path).unwrap();
    assert!(Command::new(env!("CARGO_BIN_EXE_syfrack"))
        .arg("render")
        .arg(&scene_path)
        .args(["--width", "16", "--height", "16", "--samples", "1", "--out"])
        .arg(dir.join("run1.5"))
        .status()
        .unwrap()
        .success());
    for ext in ["exr", "png", "ron"] {
        assert!(
            dir.join(format!("run1.5.{ext}")).exists(),
            "missing run1.5.{ext}"
        );
    }
}

#[test]
fn render_rejects_zero_width() {
    let dir = tmp();
    let scene_path = dir.join("zw.ron");
    Scene::example().save(&scene_path).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_syfrack"))
        .arg("render")
        .arg(&scene_path)
        .args(["--width", "0", "--out"])
        .arg(dir.join("zw"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("width"));
}

#[test]
#[ignore = "needs GPU"]
fn render_rejects_zero_samples() {
    let dir = tmp();
    let scene_path = dir.join("zs.ron");
    Scene::example().save(&scene_path).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_syfrack"))
        .arg("render")
        .arg(&scene_path)
        .args(["--samples", "0", "--out"])
        .arg(dir.join("zs"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("samples"));
}

#[test]
fn example_scene_kind_writes_the_matching_fractal() {
    let out = tmp().join("box.ron");
    let status = Command::new(env!("CARGO_BIN_EXE_syfrack"))
        .args(["example-scene", "--kind", "mandelbox", "--out"])
        .arg(&out)
        .status()
        .unwrap();
    assert!(status.success());
    let scene = Scene::load(&out).unwrap();
    // Example scenes are graph scenes with trap-slot colourings.
    assert!(matches!(scene.fractal, Fractal::Graph(_)));
    assert!(matches!(
        scene.material.coloring,
        Coloring::Trap { slot: 0, .. }
    ));
    assert_eq!(scene, Scene::example_for("mandelbox").unwrap());
}

#[test]
fn example_scene_rejects_unknown_kind() {
    let out = Command::new(env!("CARGO_BIN_EXE_syfrack"))
        .args(["example-scene", "--kind", "cube", "--out", "x.ron"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cube"));
}

#[test]
fn info_rejects_a_missing_state_dir() {
    let out = Command::new(env!("CARGO_BIN_EXE_syfrack"))
        .arg("info")
        .arg(tmp().join("nope"))
        .output()
        .unwrap();
    assert!(!out.status.success());
}

#[test]
#[ignore = "needs GPU"]
fn interrupted_render_resumes_to_the_same_image() {
    let dir = tmp();
    let scene_path = dir.join("scene.ron");
    Scene::example().save(&scene_path).unwrap();
    let common = [
        "--width",
        "64",
        "--height",
        "48",
        "--samples",
        "24",
        "--tile",
        "32",
    ];

    let full = dir.join("full");
    assert!(Command::new(env!("CARGO_BIN_EXE_syfrack"))
        .arg("render")
        .arg(&scene_path)
        .args(common)
        .arg("--out")
        .arg(&full)
        .status()
        .unwrap()
        .success());

    let part = dir.join("part");
    let state = dir.join("part.state");
    let status = Command::new(env!("CARGO_BIN_EXE_syfrack"))
        .arg("render")
        .arg(&scene_path)
        .args(common)
        .args(["--time-budget", "0", "--checkpoint-every", "1000"])
        .arg("--state")
        .arg(&state)
        .arg("--out")
        .arg(&part)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(
        state.join("state.ron").exists(),
        "state must be saved when the budget interrupts"
    );

    let info = Command::new(env!("CARGO_BIN_EXE_syfrack"))
        .arg("info")
        .arg(&state)
        .output()
        .unwrap();
    assert!(info.status.success());
    let text = String::from_utf8_lossy(&info.stdout);
    assert!(text.contains("64x48") && text.contains("/ 24"), "{text}");

    let resumed = dir.join("resumed");
    assert!(Command::new(env!("CARGO_BIN_EXE_syfrack"))
        .arg("resume")
        .arg(&state)
        .arg("--out")
        .arg(&resumed)
        .status()
        .unwrap()
        .success());
    let a = syfrack_color::read_exr(dir.join("full.exr")).unwrap();
    let b = syfrack_color::read_exr(dir.join("resumed.exr")).unwrap();
    assert_eq!(a.data, b.data, "resumed render must be bit-identical");
}

#[test]
#[ignore = "needs GPU"]
fn preview_every_writes_a_preview_png() {
    let dir = tmp();
    let scene_path = dir.join("scene.ron");
    Scene::example().save(&scene_path).unwrap();
    let out = dir.join("p");
    assert!(Command::new(env!("CARGO_BIN_EXE_syfrack"))
        .arg("render")
        .arg(&scene_path)
        .args([
            "--width",
            "64",
            "--height",
            "48",
            "--samples",
            "64",
            "--preview-every",
            "0",
            "--out"
        ])
        .arg(&out)
        .status()
        .unwrap()
        .success());
    assert!(dir.join("p.preview.png").exists());
}

fn stderr_of(args: &[&std::ffi::OsStr]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_syfrack"))
        .args(args)
        .output()
        .unwrap();
    assert!(!out.status.success());
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn render_rejects_huge_time_budget() {
    let err = stderr_of(&[
        "render".as_ref(),
        "s.ron".as_ref(),
        "--out".as_ref(),
        "x".as_ref(),
        "--time-budget".as_ref(),
        "1e300".as_ref(),
    ]);
    assert!(err.contains("--time-budget"), "{err}");
}

#[test]
fn render_rejects_negative_checkpoint_every() {
    let err = stderr_of(&[
        "render".as_ref(),
        "s.ron".as_ref(),
        "--out".as_ref(),
        "x".as_ref(),
        "--checkpoint-every".as_ref(),
        "-1".as_ref(),
    ]);
    assert!(err.contains("--checkpoint-every"), "{err}");
}

#[test]
fn render_refuses_out_colliding_with_state_file() {
    let dir = tmp();
    let state = dir.join("s");
    let out = state.join("state");
    let err = stderr_of(&[
        "render".as_ref(),
        "s.ron".as_ref(),
        "--state".as_ref(),
        state.as_os_str(),
        "--out".as_ref(),
        out.as_os_str(),
    ]);
    assert!(err.contains("state.ron"), "{err}");
}

#[test]
#[ignore = "needs GPU"]
fn exhibition_scene_renders_a_low_resolution_proof() {
    let scene = Scene::load(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scenes/exhibition.ron"
    ))
    .unwrap();
    scene.validate().unwrap();
    assert!(matches!(scene.fractal, syfrack_core::Fractal::Graph(_)));
    assert!(
        scene.atmosphere.fog_density > 0.0,
        "the exhibition scene has fog"
    );
    assert!(
        scene.camera.f_number > 0.0,
        "the exhibition scene has depth of field"
    );
    assert!(scene.lights.len() >= 3);
    let dir = tmp();
    let out = dir.join("proof");
    assert!(Command::new(env!("CARGO_BIN_EXE_syfrack"))
        .arg("render")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scenes/exhibition.ron"
        ))
        .args([
            "--width",
            "192",
            "--height",
            "108",
            "--samples",
            "16",
            "--out"
        ])
        .arg(&out)
        .status()
        .unwrap()
        .success());
    let img = syfrack_color::read_exr(dir.join("proof.exr")).unwrap();
    assert!(img.data.iter().all(|p| p.iter().all(|c| c.is_finite())));
    let mean = img.data.iter().map(|p| p[1]).sum::<f32>() / img.data.len() as f32;
    assert!(
        mean > 0.02 && mean < 5.0,
        "implausible exposure: mean {mean}"
    );
    println!("proof mean {mean}");
}

#[test]
fn render_refuses_a_state_dir_holding_foreign_files() {
    let state = tmp().join("foreign-state");
    std::fs::create_dir_all(&state).unwrap();
    let keep = state.join("important.txt");
    std::fs::write(&keep, "keep me").unwrap();
    let out = tmp().join("foreign-out");
    let err = stderr_of(&[
        "render".as_ref(),
        "s.ron".as_ref(),
        "--state".as_ref(),
        state.as_os_str(),
        "--out".as_ref(),
        out.as_os_str(),
        "--time-budget".as_ref(),
        "0".as_ref(),
    ]);
    assert!(err.contains("not a SyFrack state directory"), "{err}");
    assert_eq!(std::fs::read_to_string(&keep).unwrap(), "keep me");
}

#[test]
fn render_hints_at_resume_when_the_state_dir_holds_a_state() {
    let state = tmp().join("used-state");
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(state.join("state.ron"), "x").unwrap();
    let out = tmp().join("used-out");
    let err = stderr_of(&[
        "render".as_ref(),
        "s.ron".as_ref(),
        "--state".as_ref(),
        state.as_os_str(),
        "--out".as_ref(),
        out.as_os_str(),
    ]);
    assert!(err.contains("syfrack resume"), "{err}");
    assert_eq!(
        std::fs::read_to_string(state.join("state.ron")).unwrap(),
        "x"
    );
}

#[test]
#[ignore = "needs GPU"]
fn completed_render_saves_a_finished_state() {
    let dir = tmp();
    let scene_path = dir.join("done-scene.ron");
    Scene::example().save(&scene_path).unwrap();
    let state = dir.join("done.state");
    let out = Command::new(env!("CARGO_BIN_EXE_syfrack"))
        .arg("render")
        .arg(&scene_path)
        .args([
            "--width",
            "64",
            "--height",
            "48",
            "--samples",
            "6",
            "--tile",
            "32",
        ])
        .arg("--state")
        .arg(&state)
        .arg("--out")
        .arg(dir.join("done"))
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("render complete; state saved"));
    let info = Command::new(env!("CARGO_BIN_EXE_syfrack"))
        .arg("info")
        .arg(&state)
        .output()
        .unwrap();
    assert!(info.status.success());
    let text = String::from_utf8_lossy(&info.stdout);
    assert!(text.contains("min 6, max 6 / 6"), "{text}");
}

#[test]
fn render_refuses_out_inside_the_state_dir() {
    let state = tmp().join("inside-state");
    let out = state.join("final");
    let err = stderr_of(&[
        "render".as_ref(),
        "s.ron".as_ref(),
        "--state".as_ref(),
        state.as_os_str(),
        "--out".as_ref(),
        out.as_os_str(),
    ]);
    assert!(err.contains("inside the state directory"), "{err}");
}

#[test]
#[ignore = "needs GPU"]
fn outputs_are_written_even_when_the_final_state_save_fails() {
    let dir = tmp();
    let scene_path = dir.join("n1-scene.ron");
    Scene::example().save(&scene_path).unwrap();
    let state = dir.join("n1.state");
    let out = dir.join("n1");
    assert!(Command::new(env!("CARGO_BIN_EXE_syfrack"))
        .arg("render")
        .arg(&scene_path)
        .args([
            "--width",
            "64",
            "--height",
            "48",
            "--samples",
            "6",
            "--tile",
            "32"
        ])
        .args(["--time-budget", "0"])
        .arg("--state")
        .arg(&state)
        .arg("--out")
        .arg(&out)
        .status()
        .unwrap()
        .success());
    std::fs::write(state.join("desktop.ini"), "stray").unwrap();
    let _ = std::fs::remove_file(dir.join("n1-done.exr"));
    let res = Command::new(env!("CARGO_BIN_EXE_syfrack"))
        .arg("resume")
        .arg(&state)
        .arg("--out")
        .arg(dir.join("n1-done"))
        .output()
        .unwrap();
    assert!(
        res.status.success(),
        "{}",
        String::from_utf8_lossy(&res.stderr)
    );
    assert!(String::from_utf8_lossy(&res.stderr).contains("warning: the outputs were written"));
    assert!(dir.join("n1-done.exr").exists() && dir.join("n1-done.png").exists());
    assert!(state.join("desktop.ini").exists());
}

fn syfrack() -> Command {
    Command::new(env!("CARGO_BIN_EXE_syfrack"))
}

#[test]
fn graph_validate_reports_errors_and_compile_writes_wgsl() {
    let dir = tmp().join("graph-cli");
    std::fs::create_dir_all(&dir).unwrap();
    let bad = dir.join("bad.ron");
    let mut scene = Scene::example();
    let mut g = Graph::new_minimal();
    g.links.clear();
    scene.fractal = Fractal::Graph(g);
    std::fs::write(&bad, scene.to_ron()).unwrap();
    let out = syfrack()
        .args(["--lang", "fr", "graph", "validate"])
        .arg(&bad)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Sortie"));
    let wgsl = dir.join("de.wgsl");
    let out = syfrack()
        .args(["graph", "compile", "../../scenes/mandelbox.ron", "--out"])
        .arg(&wgsl)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(std::fs::read_to_string(&wgsl)
        .unwrap()
        .contains("fn de_sample"));
}

#[test]
fn graph_upgrade_refuses_a_pre_plan4_scene() {
    let dir = tmp().join("graph-upgrade-old");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("old.ron");
    let text = Scene::example().to_ron();
    let (a, b) = (
        text.find("fractal:").unwrap(),
        text.find("camera:").unwrap(),
    );
    let old = format!(
        "{}fractal: Mandelbox(scale: 2.0, min_radius: 0.5, fold_limit: 1.0, iterations: 12),\n    {}",
        &text[..a],
        &text[b..]
    );
    std::fs::write(&path, &old).unwrap();
    let out = syfrack()
        .args(["graph", "upgrade"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains(syfrack_core::scene::PRE_PLAN4), "{err}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), old, "untouched");
    assert!(!dir.join("old.ron.bak").exists());
}

#[test]
fn graph_upgrade_rewrites_a_current_scene_only_with_force_and_keeps_backups() {
    let dir = tmp().join("graph-upgrade");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("scene.ron");
    let scene = Scene::example_for("mandelbox").unwrap();
    std::fs::write(&path, format!("// a comment\n{}", scene.to_ron())).unwrap();
    let out = syfrack()
        .args(["graph", "upgrade"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "an upgrade of a current scene without --force must be refused"
    );
    assert!(!dir.join("scene.ron.bak").exists());
    for backup in ["scene.ron.bak", "scene.ron.bak.1"] {
        let out = syfrack()
            .args(["graph", "upgrade", "--force"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(dir.join(backup).exists(), "{backup}");
    }
    assert!(std::fs::read_to_string(dir.join("scene.ron.bak"))
        .unwrap()
        .starts_with("// a comment"));
    assert_eq!(Scene::load(&path).unwrap(), scene);
}
