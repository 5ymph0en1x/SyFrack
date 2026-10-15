//! Native window regression check. Creates three dialogs, moves them beyond
//! the parent, exercises the shared atlas, then closes every test window.
use anyhow::Result;
use std::sync::Arc;
use syfrack_app_lib::{
    desktop::Desktop,
    egui_layer::EguiLayer,
    ui::{floating, theme},
    window::Gfx,
};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Window, WindowId},
};

struct Running {
    window: Arc<Window>,
    gfx: Gfx,
    egui: EguiLayer,
    desktop: Desktop,
    open: [bool; 3],
    frames: u32,
    texture: egui::TextureHandle,
    verified: bool,
    last_frame: std::time::Instant,
}
#[derive(Default)]
struct App {
    running: Option<Running>,
    error: Option<anyhow::Error>,
    done: bool,
    started: Option<std::time::Instant>,
}
const TITLES: [&str; 3] = [
    "Rendre / Exporter",
    "Prendre en main le Studio",
    "Enregistrer la forme",
];
fn dialogs(ui: &mut egui::Ui, open: &mut [bool; 3], texture: egui::TextureId) {
    for (name, open) in TITLES.iter().zip(open) {
        if *open {
            floating::Window::new(name).open(open).show(ui.ctx(), |ui| {
                ui.heading(*name);
                ui.label("Fenêtre indépendante · caractères accentués · α β γ");
                ui.image((texture, egui::vec2(80.0, 80.0)));
                let _ = ui.button("Contrôle partagé");
            });
        }
    }
}
impl App {
    fn create(event_loop: &ActiveEventLoop) -> Result<Running> {
        let window = Arc::new(
            event_loop.create_window(
                Window::default_attributes()
                    .with_title("SyFrack · validation fenêtres")
                    .with_position(winit::dpi::PhysicalPosition::new(40, 40))
                    .with_inner_size(winit::dpi::LogicalSize::new(640.0, 400.0)),
            )?,
        );
        let gfx = Gfx::new(window.clone())?;
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        floating::enable(&ctx);
        let texture = ctx.load_texture(
            "shared-native-test",
            egui::ColorImage::filled([8, 8], theme::ACCENT),
            Default::default(),
        );
        let egui = EguiLayer::new(ctx, &window, &gfx.device, gfx.config.format);
        Ok(Running {
            window,
            gfx,
            egui,
            desktop: Desktop::default(),
            open: [true; 3],
            frames: 0,
            texture,
            verified: false,
            last_frame: std::time::Instant::now(),
        })
    }
    fn frame(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        let r = self.running.as_mut().unwrap();
        r.last_frame = std::time::Instant::now();
        eprintln!(
            "native frame {} · {} tool windows",
            r.frames + 1,
            r.desktop.windows.len()
        );
        let ctx = r.egui.context();
        r.egui.frame(
            &r.window,
            &mut r.gfx,
            |_| {},
            |ui| {
                floating::begin_root(ui.ctx());
                dialogs(ui, &mut r.open, r.texture.id());
                floating::finish_root(ui.ctx());
            },
        )?;
        let errors = r.desktop.sync(event_loop, &r.window, &r.gfx, &r.egui);
        anyhow::ensure!(errors.is_empty(), "{errors:?}");
        r.frames += 1;
        if r.frames == 2 {
            anyhow::ensure!(r.desktop.windows.len() == 3);
            let root = r.window.outer_position()?;
            let right = root.x + r.window.outer_size().width as i32;
            for (i, tool) in r.desktop.windows.values().enumerate() {
                tool.window
                    .set_outer_position(winit::dpi::PhysicalPosition::new(
                        right + 48 + i as i32 * 20,
                        root.y + 30,
                    ));
            }
        }
        if r.frames == 12 {
            let root = r.window.outer_position()?;
            let right = root.x + r.window.outer_size().width as i32;
            for tool in r.desktop.windows.values() {
                let p = tool.window.outer_position()?;
                anyhow::ensure!(
                    p.x > right,
                    "dialog constrained to parent: {p:?}, edge {right}"
                );
            }
            r.verified = true;
            for id in r.desktop.windows.keys() {
                floating::close(&ctx, *id);
            }
        }
        if r.frames >= 15 {
            anyhow::ensure!(r.verified && r.desktop.windows.is_empty() && r.open == [false; 3]);
            println!("PASS: three native viewports rendered with a shared atlas; all moved beyond the parent; close acknowledgements removed every tool window.");
            self.done = true;
            event_loop.exit();
        }
        r.window.request_redraw();
        Ok(())
    }
}
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.started = Some(std::time::Instant::now());
        match Self::create(event_loop) {
            Ok(r) => self.running = Some(r),
            Err(e) => {
                self.error = Some(e);
                event_loop.exit();
            }
        }
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(r) = &mut self.running else { return };
        let result = if id == r.window.id() {
            let _ = r.egui.on_window_event(&r.window, &event);
            match event {
                WindowEvent::RedrawRequested => self.frame(event_loop),
                WindowEvent::CloseRequested => {
                    event_loop.exit();
                    Ok(())
                }
                WindowEvent::Resized(s) => {
                    r.gfx.resize(s.width, s.height);
                    Ok(())
                }
                _ => Ok(()),
            }
        } else if let Some(viewport) = r.desktop.viewport(id) {
            let tool = r.desktop.windows.get_mut(&viewport).unwrap();
            let _ = tool.egui.on_window_event(&tool.window, &event);
            match event {
                WindowEvent::RedrawRequested => tool
                    .egui
                    .frame(
                        &tool.window,
                        &mut tool.gfx,
                        |_| {},
                        |ui| dialogs(ui, &mut r.open, r.texture.id()),
                    )
                    .map(|_| ()),
                WindowEvent::Resized(s) => {
                    tool.gfx.resize(s.width, s.height);
                    Ok(())
                }
                _ => Ok(()),
            }
        } else {
            Ok(())
        };
        if let Err(e) = result {
            self.error = Some(e);
            event_loop.exit();
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.started.is_some_and(|t| t.elapsed().as_secs() >= 25) {
            self.error = Some(anyhow::anyhow!(
                "native check timed out after {} root frames",
                self.running.as_ref().map_or(0, |r| r.frames)
            ));
            event_loop.exit();
            return;
        }
        event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
        if self
            .running
            .as_ref()
            .is_some_and(|r| r.last_frame.elapsed().as_millis() >= 100)
        {
            if let Err(e) = self.frame(event_loop) {
                self.error = Some(e);
                event_loop.exit();
                return;
            }
        }
        if let Some(r) = &self.running {
            r.window.request_redraw();
            for w in r.desktop.windows.values() {
                w.window.request_redraw();
            }
        }
    }
}
fn main() -> Result<()> {
    let mut app = App::default();
    EventLoop::new()?.run_app(&mut app)?;
    if let Some(e) = app.error {
        return Err(e);
    }
    anyhow::ensure!(app.done, "native verification interrupted");
    Ok(())
}
