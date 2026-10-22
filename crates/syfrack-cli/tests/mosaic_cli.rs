use std::process::Command;

#[test]
fn adaptive_mosaic_exports_and_reconstructs_without_source() {
    let dir = std::env::temp_dir().join(format!("syfrack-mosaic-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source_path = dir.join("source.png");
    let mut source = syfrack_core::HdrImage::new(33, 21);
    for y in 0..21 {
        for x in 0..33 {
            let v = if x < 16 { 0.2 } else { ((x + y) % 2) as f32 };
            source.set(x, y, [v, v * 0.5, 0.1, 1.0]);
        }
    }
    let encoded =
        syfrack_color::encode::encode(&source, syfrack_core::OutputSpace::Srgb, 16, false);
    syfrack_color::write_png(
        &source_path,
        &encoded,
        syfrack_core::OutputSpace::Srgb,
        None,
    )
    .unwrap();
    let prefix = dir.join("adaptive");
    let build = Command::new(env!("CARGO_BIN_EXE_syfrack"))
        .args(["mosaic", "build"])
        .arg(&source_path)
        .arg("--out")
        .arg(&prefix)
        .args(["--min-size", "4", "--regions", "40"])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    for extension in ["png", "layout.png", "source.png", "mosaic.ron"] {
        assert!(prefix.with_extension(extension).is_file());
    }
    let text = std::fs::read_to_string(prefix.with_extension("mosaic.ron")).unwrap();
    let recipe: syfrack_core::mosaic::Mosaic = ron::from_str(&text).unwrap();
    assert!(recipe.tiles.len() > 1 && recipe.tiles.len() <= 40);
    recipe.validate().unwrap();
    std::fs::remove_file(&source_path).unwrap();
    let rerendered = dir.join("rerendered.png");
    let render = Command::new(env!("CARGO_BIN_EXE_syfrack"))
        .args(["mosaic", "render"])
        .arg(prefix.with_extension("mosaic.ron"))
        .arg("--out")
        .arg(&rerendered)
        .output()
        .unwrap();
    assert!(
        render.status.success(),
        "{}",
        String::from_utf8_lossy(&render.stderr)
    );
    assert_eq!(
        std::fs::read(prefix.with_extension("png")).unwrap(),
        std::fs::read(&rerendered).unwrap()
    );
    let bad = Command::new(env!("CARGO_BIN_EXE_syfrack"))
        .args(["mosaic", "build", "missing.png", "--detail", "NaN", "--out"])
        .arg(dir.join("bad"))
        .output()
        .unwrap();
    assert!(!bad.status.success());
    assert!(!dir.join("bad.png").exists());
    // A second run must not overwrite the first recipe or its source.
    let collision = Command::new(env!("CARGO_BIN_EXE_syfrack"))
        .args(["mosaic", "render"])
        .arg(prefix.with_extension("mosaic.ron"))
        .arg("--out")
        .arg(&rerendered)
        .output()
        .unwrap();
    assert!(!collision.status.success());
    std::fs::remove_dir_all(&dir).unwrap();
}
