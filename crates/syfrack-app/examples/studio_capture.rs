//! Draw the real Studio UI with egui/wgpu into PNGs for visual review.
//! Run from the repository root: cargo run -p syfrack-app --example studio_capture -- target/gui-captures
//! The viewport uses an existing native render as a clearly labelled reference.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use syfrack_app_lib::state::{AppState, Inspector};
use syfrack_app_lib::ui::{
    self, form_panel::FormPanel, gallery_panel::GalleryPanel, render_panel::RenderPanel, Viewport,
};
use syfrack_core::Scene;
use syfrack_render::{GpuContext, Renderer};

fn main() -> Result<()> {
    syfrack_i18n::initialize();
    let dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "target/gui-captures".into()),
    );
    let workspace = std::env::args().nth(2);
    let native_render = workspace
        .as_deref()
        .is_some_and(|s| s.starts_with("native-render"));
    let render_size = if workspace.as_deref() == Some("native-render-compact") {
        [760, 660]
    } else {
        [1000, 860]
    };
    let native_explorer = workspace
        .as_deref()
        .is_some_and(|s| s.starts_with("native-explorer"));
    let native_quit = workspace.as_deref() == Some("native-quit");
    let native = native_render || native_explorer || native_quit;
    let zoom: f32 = std::env::args()
        .nth(3)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0);
    let mut edit_address = workspace.as_deref() == Some("native-explorer-edit");
    let mut native_ready = false;
    let mut custom_click = if workspace
        .as_deref()
        .is_some_and(|s| s.contains("custom") || s.contains("render-flame"))
    {
        2
    } else {
        0
    };
    let mut custom_scroll = custom_click > 0;
    std::fs::create_dir_all(&dir)?;
    let gpu = Arc::new(GpuContext::new()?);
    let engine = Renderer::new(gpu.clone())?;
    let ctx = egui::Context::default();
    ui::theme::apply(&ctx);
    ctx.set_zoom_factor(zoom);
    if native {
        ui::floating::enable(&ctx);
    }
    let mut style = (*ctx.global_style()).clone();
    style.animation_time = 0.0;
    ctx.set_global_style(style);
    let content = syfrack_app_lib::paths::content_root();
    let workshop = workspace
        .as_deref()
        .is_some_and(|s| s.starts_with("workshop-") || s.starts_with("chrome-"));
    let crown = workspace
        .as_deref()
        .is_some_and(|s| s == "chrome-2d" || s.contains("render-flame"));
    let flat = crown
        || workspace
            .as_deref()
            .is_some_and(|s| s.starts_with("workshop-2d"));
    let scene_path = std::env::args()
        .nth(4)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            content.join(if crown {
                "renders/couronne3.ron"
            } else if flat {
                "scenes/studies/soies-neon-doux-20261013/soie-de-feu-neon-doux.ron"
            } else if workshop {
                "scenes/studies/workshop-preview-20261013/corail.ron"
            } else {
                "scenes/environments/palais-rose-sur-mer.ron"
            })
        });
    let scene_path = if scene_path.exists() {
        scene_path
    } else if flat {
        content.join("scenes/showcase/breath-of-the-titans.ron")
    } else {
        content.join("scenes/mandelbulb.ron")
    };
    let mut state = AppState::new(
        Scene::load(&scene_path)?,
        if native_explorer {
            None
        } else {
            Some(scene_path)
        },
    );
    state.show_gizmos = false;
    if workspace
        .as_deref()
        .is_some_and(|s| s.starts_with("chrome-"))
    {
        state.edit(|s| s.seed = s.seed.wrapping_add(1));
    }
    let reference_path = std::env::args()
        .nth(5)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if crown {
                content.join("renders/couronne3.png")
            } else if flat {
                content.join("renders/studies/soies-neon-doux-20261013/soie-de-feu-neon-doux.png")
            } else if workshop {
                content.join("renders/studies/workshop-preview-20261013/corail.png")
            } else if content
                .join("renders/palais-rose-sur-mer-full-hd.png")
                .exists()
            {
                content.join("renders/palais-rose-sur-mer-full-hd.png")
            } else {
                content.join("docs/images/palais-rose-sur-mer.png")
            }
        });
    let reference_path = if reference_path.exists() {
        reference_path
    } else {
        content.join("docs/images/breath-of-the-titans.png")
    };
    let (w, h, rgba) = syfrack_app_lib::gallery::thumbnail(&reference_path, 1920)?;
    let reference = ctx.load_texture(
        "native-palace-reference",
        egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba),
        egui::TextureOptions::LINEAR,
    );
    let mut view = Viewport {
        texture: Some(reference.id()),
        size: (w, h),
        caption: if workshop {
            "Native render reference · 2D / 3D workshop".into()
        } else {
            "Native render reference".into()
        },
        available_px: (0, 0),
        depth_hint: 1.0,
        pick: None,
        frame_guide: None,
        output: Default::default(),
    };
    let mut gallery = GalleryPanel::new();
    let mut form = FormPanel::new();
    if workspace.is_some() {
        form.clay = false;
        form.view = ui::form_panel::FormView {
            texture: Some(reference.id()),
            size: (w, h),
            caption: "Native render reference".into(),
        };
        if let Some(n) = state
            .scene
            .fractal
            .to_graph()
            .nodes
            .iter()
            .find(|n| matches!(n.kind, syfrack_core::NodeKind::Box { .. }))
        {
            state.form.selected = vec![n.id];
        }
    }
    let mut render = RenderPanel::new();
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut painter =
        egui_wgpu::Renderer::new(&gpu.device, format, egui_wgpu::RendererOptions::default());
    let mut time = 0.0;
    let mut frame = |size: [u32; 2], page: Inspector, open_render: bool| -> Result<Vec<u8>> {
        let size = if native_render {
            render_size
        } else if native_explorer {
            [860, 650]
        } else if native_quit {
            [540, 208]
        } else {
            size
        };
        let size = size.map(|s| (s as f32 * zoom).round() as u32);
        state.inspector = if workspace.as_deref().is_some_and(|s| s.ends_with("colours")) {
            Inspector::Material
        } else {
            page
        };
        state.tab = if workspace.as_deref().is_some_and(|s| s != "explorer") && !native && !workshop
        {
            syfrack_app_lib::state::Tab::Forme
        } else {
            syfrack_app_lib::state::Tab::Atelier
        };
        if workspace.is_some() {
            form.mode = ui::form_panel::FormMode::Construction;
        }
        if open_render && (workspace.is_none() || native_render) {
            ctx.data_mut(|d| d.insert_temp(ui::render_panel::toggle_id(), true));
        }
        time += 1.0 / 60.0;
        let mut full = ctx.run_ui(
            egui::RawInput {
                viewport_id: if native && native_ready {
                    egui::ViewportId(egui::Id::new(if native_render {
                        "Rendre / Exporter"
                    } else if native_quit {
                        "quit-confirm"
                    } else {
                        "file-dialog"
                    }))
                } else {
                    egui::ViewportId::ROOT
                },
                viewports: std::iter::once((egui::ViewportId::ROOT, egui::ViewportInfo::default()))
                    .chain(ui::floating::requests(&ctx).into_iter().map(|r| {
                        (
                            r.id,
                            egui::ViewportInfo {
                                parent: Some(egui::ViewportId::ROOT),
                                ..Default::default()
                            },
                        )
                    }))
                    .collect(),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(size[0] as f32 / zoom, size[1] as f32 / zoom),
                )),
                time: Some(time),
                events: if open_render
                    && (workspace.as_deref() == Some("explorer") || native_explorer)
                {
                    vec![
                        egui::Event::ModifiersChanged(egui::Modifiers::COMMAND),
                        egui::Event::Key {
                            key: egui::Key::O,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::COMMAND,
                        },
                    ]
                } else if edit_address && native_ready {
                    edit_address = false;
                    vec![egui::Event::Key {
                        key: egui::Key::L,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::COMMAND,
                    }]
                } else if custom_click > 0 && native_ready && time > 6.0 / 60.0 {
                    if let Some((_, rect)) = ctx
                        .data(|d| {
                            d.get_temp::<(egui::ViewportId, egui::Rect)>(egui::Id::new(
                                "render-custom-header",
                            ))
                        })
                        .filter(|(id, _)| *id != egui::ViewportId::ROOT)
                    {
                        let pressed = custom_click == 2;
                        custom_click -= 1;
                        vec![
                            egui::Event::PointerMoved(rect.center()),
                            egui::Event::PointerButton {
                                pos: rect.center(),
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ]
                    } else {
                        vec![]
                    }
                } else if custom_scroll && custom_click == 0 && native_ready && time > 8.0 / 60.0 {
                    custom_scroll = false;
                    vec![
                        egui::Event::PointerMoved(egui::pos2(330.0, 630.0)),
                        egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: egui::vec2(0.0, -220.0),
                            phase: egui::TouchPhase::Move,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]
                } else {
                    vec![]
                },
                ..Default::default()
            },
            |ui| {
                if native && !native_ready {
                    ui::floating::begin_root(ui.ctx());
                }
                ui::draw(ui, &mut state, &mut view, &mut gallery, &mut form);
                render.show(ui, &mut state, &engine);
                if native_quit {
                    ui::confirm_quit(ui.ctx(), egui::Id::new("quit-confirm"));
                }
                if native && !native_ready {
                    ui::floating::finish_root(ui.ctx());
                }
            },
        );
        if native {
            native_ready = true;
        }
        let primitives = ctx.tessellate(std::mem::take(&mut full.shapes), full.pixels_per_point);
        let mut textures = std::mem::take(&mut full.textures_delta);
        for (id, deltas) in &textures.set {
            for delta in deltas {
                painter.update_texture(&gpu.device, &gpu.queue, *id, delta);
            }
        }
        let free = std::mem::take(&mut textures.free);
        textures.clear();
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: size,
            pixels_per_point: full.pixels_per_point,
        };
        let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Studio UI capture"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&Default::default());
        let row = (size[0] * 4).next_multiple_of(256);
        let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("UI capture readback"),
            size: (row * size[1]) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let extra =
            painter.update_buffers(&gpu.device, &gpu.queue, &mut encoder, &primitives, &screen);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Actual egui Studio interface"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            painter.render(&mut pass, &primitives, &screen);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(size[1]),
                },
            },
            wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
        );
        gpu.queue
            .submit(extra.into_iter().chain([encoder.finish()]));
        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
        rx.recv()??;
        let mapped = slice.get_mapped_range()?;
        let rgba: Vec<u8> = mapped
            .chunks(row as usize)
            .flat_map(|r| r[..(size[0] * 4) as usize].iter().copied())
            .collect();
        drop(mapped);
        readback.unmap();
        for id in free {
            painter.free_texture(&id);
        }
        Ok(rgba)
    };
    if let Some(workspace) = &workspace {
        let _ = frame(
            [1600, 900],
            Inspector::Scene,
            workspace == "explorer" || native,
        )?;
        for _ in 0..24 {
            std::thread::sleep(std::time::Duration::from_millis(25));
            let _ = frame([1600, 900], Inspector::Scene, false)?;
        }
        let rgba = frame([1600, 900], Inspector::Scene, false)?;
        write_png(
            &dir.join(format!("studio-{workspace}.png")),
            if native_render {
                render_size
            } else if native_explorer {
                [860, 650]
            } else if native_quit {
                [540, 208]
            } else {
                [1600, 900]
            }
            .map(|s| (s as f32 * zoom).round() as u32),
            &rgba,
        )?;
        if workspace.starts_with("chrome-") {
            let bar_height = 48;
            write_png(
                &dir.join(format!("studio-{workspace}-bar.png")),
                [1600, bar_height],
                &rgba[..1600 * bar_height as usize * 4],
            )?;
        }
        if native {
            return Ok(());
        }
        for _ in 0..4 {
            let _ = frame([1000, 640], Inspector::Scene, false)?;
        }
        let rgba = frame([1000, 640], Inspector::Scene, false)?;
        write_png(
            &dir.join(format!("studio-{workspace}-compact.png")),
            [1000, 640],
            &rgba,
        )?;
        return Ok(());
    }
    let _ = frame([1600, 900], Inspector::Texture, false)?;
    for _ in 0..100 {
        if !ui::texture_preview::is_pending(&ctx) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
        let _ = frame([1600, 900], Inspector::Texture, false)?;
    }
    for (page, name) in Inspector::ALL.into_iter().zip([
        "scene",
        "lights",
        "material",
        "texture",
        "atmosphere",
        "image",
    ]) {
        for _ in 0..12 {
            let _ = frame([1600, 900], page, false)?;
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let rgba = frame([1600, 900], page, false)?;
        write_png(&dir.join(format!("studio-{name}.png")), [1600, 900], &rgba)?;
    }
    let _ = frame([1600, 900], Inspector::Scene, true)?;
    for _ in 0..12 {
        let _ = frame([1600, 900], Inspector::Scene, false)?;
    }
    let rgba = frame([1600, 900], Inspector::Scene, false)?;
    write_png(&dir.join("studio-export.png"), [1600, 900], &rgba)?;
    let _ = frame([1000, 640], Inspector::Texture, true)?;
    let rgba = frame([1000, 640], Inspector::Texture, false)?;
    write_png(&dir.join("studio-compact.png"), [1000, 640], &rgba)?;
    drop(frame);
    Ok(())
}

fn write_png(path: &Path, size: [u32; 2], rgba: &[u8]) -> Result<()> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, size[0], size[1]);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(rgba)?;
    eprintln!("wrote {}", path.display());
    Ok(())
}
