use super::*;

fn fixture() -> (egui::Context, AppState) {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let raw = Arc::new(HdrImage::from_data(
        32,
        16,
        (0..512)
            .map(|i| [i as f32 / 512.0, 0.2, 0.4, 1.0])
            .collect(),
    ));
    let imported = Imported {
        image: worker::colour_image(&raw),
        raw,
        path: PathBuf::from("test-image.png"),
        settings: Settings::default(),
    };
    memory(&ctx)
        .lock()
        .unwrap()
        .adopt(&ctx, Reply::Imported(imported));
    (ctx, AppState::new(syfrack_core::Scene::example(), None))
}

#[test]
fn adjustment_history_is_independent_and_coalesces_a_slider_gesture() {
    let (ctx, state) = fixture();
    let scene = state.scene.clone();
    assert!(!has_unsaved_work(&ctx));
    {
        let binding = memory(&ctx);
        let mut panel = binding.lock().unwrap();
        let before = panel.settings.clone();
        panel.settings.image.exposure = 0.5;
        panel.track(before, true);
        let before = panel.settings.clone();
        panel.settings.image.exposure = 1.0;
        panel.track(before, true);
        assert_eq!(panel.undo.len(), 1);
    }
    assert!(has_unsaved_work(&ctx));
    assert!(history(&ctx, false, true));
    assert!(!has_unsaved_work(&ctx));
    assert_eq!(memory(&ctx).lock().unwrap().settings.image.exposure, 0.0);
    assert!(history(&ctx, true, true));
    assert_eq!(memory(&ctx).lock().unwrap().settings.image.exposure, 1.0);
    assert_eq!(state.scene, scene);
    assert!(!state.can_undo());
}

#[test]
fn a_stale_result_cannot_be_exported_and_an_old_preview_is_not_adopted() {
    let (ctx, _) = fixture();
    let binding = memory(&ctx);
    let mut panel = binding.lock().unwrap();
    let built = worker::build(
        panel.raw.as_ref().unwrap(),
        panel.settings.clone(),
        &MosaicControl::default(),
    )
    .unwrap();
    panel.adopt(&ctx, Reply::Built(built));
    assert!(panel.current());
    let before = panel.settings.clone();
    panel.settings.image.exposure = 1.0;
    panel.track(before.clone(), false);
    assert!(!panel.current());
    let old = worker::preview(
        panel.raw.as_ref().unwrap(),
        before,
        &MosaicControl::default(),
    )
    .unwrap();
    panel.adopt(&ctx, Reply::Preview(old));
    assert_ne!(panel.preview_settings.as_ref(), Some(&panel.settings));
    panel.export(&ctx);
    assert!(panel.task.is_none());
    assert!(panel.message.as_ref().unwrap().1);
    assert!(panel.product.is_some());
}

#[test]
fn dragging_on_the_image_changes_the_crop_in_one_undo_step() {
    let (ctx, state) = fixture();
    memory(&ctx).lock().unwrap().crop_mode = true;
    let mut time = 0.0;
    let mut frame = |events: Vec<egui::Event>| {
        time += 1.0 / 60.0;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 700.0))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| {
                syfrack_i18n::with_language(syfrack_i18n::Language::French, || show(ui, &state));
            },
        );
        output.textures_delta.clear();
    };
    frame(vec![]);
    frame(vec![]);
    let area: Rect = ctx
        .data(|d| d.get_temp(Id::new("mosaic-image-area")))
        .unwrap();
    let image = Rect::from_center_size(area.center(), Vec2::new(area.width(), area.width() / 2.0));
    let start = image.min + image.size() * 0.25;
    let end = image.min + image.size() * 0.75;
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frame(vec![egui::Event::PointerMoved(start), button(start, true)]);
    frame(vec![egui::Event::PointerMoved(end)]);
    frame(vec![egui::Event::PointerMoved(end), button(end, false)]);
    let binding = memory(&ctx);
    let panel = binding.lock().unwrap();
    for (value, expected) in panel
        .settings
        .image
        .crop
        .iter()
        .zip([0.25, 0.25, 0.75, 0.75])
    {
        assert!(
            (*value - expected).abs() < 0.01,
            "crop {:?}",
            panel.settings.image.crop
        );
    }
    assert_eq!(panel.undo.len(), 1);
    assert!(!state.can_undo());
}

#[test]
fn painting_a_continuous_priority_stroke_undoes_and_redoes_as_one_edit() {
    let (ctx, state) = fixture();
    {
        let binding = memory(&ctx);
        let mut panel = binding.lock().unwrap();
        let preview = worker::preview(
            panel.raw.as_ref().unwrap(),
            panel.settings.clone(),
            &MosaicControl::default(),
        )
        .unwrap();
        panel.adopt(&ctx, Reply::Preview(preview));
        panel.brush_mode = true;
    }
    let mut time = 0.0;
    let mut frame = |events| {
        time += 1.0 / 60.0;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 700.0))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| show(ui, &state),
        );
        output.textures_delta.clear();
    };
    frame(vec![]);
    frame(vec![]);
    let area: Rect = ctx
        .data(|d| d.get_temp(Id::new("mosaic-image-area")))
        .unwrap();
    let image = Rect::from_center_size(area.center(), Vec2::new(area.width(), area.width() / 2.0));
    let start = image.min + image.size() * 0.25;
    let end = image.min + image.size() * 0.75;
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frame(vec![egui::Event::PointerMoved(start), button(start, true)]);
    frame(vec![egui::Event::PointerMoved(
        (start + end.to_vec2()) / 2.0,
    )]);
    frame(vec![egui::Event::PointerMoved(end)]);
    frame(vec![button(end, false)]);
    {
        let binding = memory(&ctx);
        let panel = binding.lock().unwrap();
        assert_eq!(panel.settings.priority.len(), 1);
        assert!(panel.settings.priority[0].points.len() >= 2);
        assert_eq!(panel.undo.len(), 1);
        assert_eq!(panel.pan, Vec2::ZERO);
    }
    assert!(history(&ctx, false, true));
    assert!(memory(&ctx).lock().unwrap().settings.priority.is_empty());
    assert!(history(&ctx, true, true));
    assert_eq!(memory(&ctx).lock().unwrap().settings.priority.len(), 1);
    assert!(!state.can_undo());
}
