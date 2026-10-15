//! Native, independently movable tool windows on the same GPU/context.
use crate::{egui_layer::EguiLayer, ui::floating, window::Gfx};
use egui::ViewportId;
use std::{collections::HashMap, sync::Arc};
use winit::{
    dpi::{LogicalSize, PhysicalPosition},
    event_loop::ActiveEventLoop,
    window::{Window, WindowId},
};

pub struct ToolWindow {
    pub window: Arc<Window>,
    pub gfx: Gfx,
    pub egui: EguiLayer,
    pub minimized: bool,
    title: String,
}

#[derive(Default)]
pub struct Desktop {
    pub windows: HashMap<ViewportId, ToolWindow>,
    positions: HashMap<ViewportId, PhysicalPosition<i32>>,
    failed: std::collections::HashSet<ViewportId>,
}

impl Desktop {
    pub fn viewport(&self, id: WindowId) -> Option<ViewportId> {
        self.windows
            .iter()
            .find(|(_, w)| w.window.id() == id)
            .map(|(id, _)| *id)
    }
    pub fn sync(
        &mut self,
        event_loop: &ActiveEventLoop,
        root: &Window,
        gfx: &Gfx,
        egui: &EguiLayer,
    ) -> Vec<String> {
        let ctx = egui.context();
        let requests = floating::requests(&ctx);
        self.windows.retain(|id, tool| {
            let retain = requests.iter().any(|r| r.id == *id);
            if !retain {
                if let Ok(pos) = tool.window.outer_position() {
                    self.positions.insert(*id, pos);
                }
            }
            retain
        });
        self.failed
            .retain(|id| requests.iter().any(|r| r.id == *id));
        let mut errors = Vec::new();
        for request in requests {
            if let Some(tool) = self.windows.get_mut(&request.id) {
                if tool.title != request.title {
                    tool.window.set_title(&request.title);
                    tool.title.clone_from(&request.title);
                    tool.window.request_redraw();
                }
                continue;
            }
            if self.failed.contains(&request.id) {
                continue;
            }
            let result = (|| -> anyhow::Result<ToolWindow> {
                // UI sizes are in egui points, before the user's interface zoom.
                let zoom = ctx.zoom_factor();
                let mut size = request.size * zoom;
                let mut min_size = request.min_size * zoom;
                if let Some(monitor) = root
                    .current_monitor()
                    .or_else(|| event_loop.primary_monitor())
                {
                    let screen = monitor.size().to_logical::<f32>(monitor.scale_factor());
                    let available = egui::vec2(
                        (screen.width - 32.0).max(320.0),
                        (screen.height - 96.0).max(320.0),
                    );
                    size = size.min(available);
                    min_size = min_size.min(available);
                }
                let mut attrs = Window::default_attributes()
                    .with_title(format!("{} — SyFrack", request.title))
                    .with_resizable(true)
                    .with_min_inner_size(LogicalSize::new(min_size.x, min_size.y))
                    .with_inner_size(LogicalSize::new(size.x, size.y));
                // Remember desktop coordinates, including negative coordinates
                // on a monitor to the left. Only recover an unplugged monitor.
                if let Some(pos) = self.positions.get(&request.id).copied() {
                    if root.available_monitors().any(|m| {
                        let p = m.position();
                        let s = m.size();
                        pos.x >= p.x
                            && pos.y >= p.y
                            && pos.x < p.x + s.width as i32
                            && pos.y < p.y + s.height as i32
                    }) {
                        attrs = attrs.with_position(pos);
                    }
                }
                let window = Arc::new(event_loop.create_window(attrs)?);
                let layer = egui.sibling(&window, request.id);
                let surface = gfx.sibling(window.clone())?;
                Ok(ToolWindow {
                    window,
                    egui: layer,
                    gfx: surface,
                    minimized: false,
                    title: request.title.clone(),
                })
            })();
            match result {
                Ok(tool) => {
                    tool.window.request_redraw();
                    self.windows.insert(request.id, tool);
                }
                Err(e) => {
                    self.failed.insert(request.id);
                    floating::close(&ctx, request.id);
                    errors.push(format!("{} : {e:#}", request.title));
                }
            }
        }
        errors
    }
}
