use anyhow::Result;
use std::cell::{RefCell, RefMut};
use std::rc::Rc;
use winit::event::WindowEvent;
use winit::window::Window;

use crate::window::Gfx;

/// egui input (egui-winit) and painting (egui-wgpu) for one window.
pub struct EguiLayer {
    state: egui_winit::State,
    renderer: Rc<RefCell<egui_wgpu::Renderer>>,
}

impl EguiLayer {
    pub fn new(
        ctx: egui::Context,
        window: &Window,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> Self {
        let state = egui_winit::State::new(
            ctx,
            egui::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        let renderer =
            egui_wgpu::Renderer::new(device, format, egui_wgpu::RendererOptions::default());
        Self {
            state,
            renderer: Rc::new(RefCell::new(renderer)),
        }
    }

    pub fn renderer_mut(&mut self) -> RefMut<'_, egui_wgpu::Renderer> {
        self.renderer.borrow_mut()
    }

    pub fn context(&self) -> egui::Context {
        self.state.egui_ctx().clone()
    }

    pub fn sibling(&self, window: &Window, id: egui::ViewportId) -> Self {
        Self {
            state: egui_winit::State::new(
                self.context(),
                id,
                window,
                Some(window.scale_factor() as f32),
                None,
                None,
            ),
            renderer: self.renderer.clone(),
        }
    }

    pub fn on_window_event(
        &mut self,
        window: &Window,
        event: &WindowEvent,
    ) -> egui_winit::EventResponse {
        self.state.on_window_event(window, event)
    }

    /// A pointer button is held (a drag under way).
    pub fn pointer_down(&self) -> bool {
        self.state.egui_ctx().input(|i| i.pointer.any_down())
    }

    /// One frame: gather input, run the UI, upload textures, paint, present.
    /// `before_ui` runs GPU work that must precede the egui pass (the preview display).
    /// Returns whether a frame was presented (false when occluded or zero-sized).
    pub fn frame(
        &mut self,
        window: &Window,
        gfx: &mut Gfx,
        before_ui: impl FnOnce(&mut wgpu::CommandEncoder),
        run_ui: impl FnOnce(&mut egui::Ui),
    ) -> Result<bool> {
        let mut raw = self.state.take_egui_input(window);
        let ctx = self.state.egui_ctx().clone();
        // egui-winit knows only this window. Keep every live viewport in the
        // input map so focus and memory survive frames in a different window.
        for request in crate::ui::floating::requests(&ctx) {
            raw.viewports
                .entry(request.id)
                .or_insert_with(|| egui::ViewportInfo {
                    parent: Some(egui::ViewportId::ROOT),
                    ..Default::default()
                });
        }
        raw.viewports.entry(egui::ViewportId::ROOT).or_default();
        let mut run_ui = Some(run_ui);
        let mut full = ctx.run_ui(raw, |ui| {
            if let Some(run_ui) = run_ui.take() {
                run_ui(ui);
            }
        });
        self.state
            .handle_platform_output(window, full.platform_output);
        let primitives = ctx.tessellate(full.shapes, full.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [gfx.config.width, gfx.config.height],
            pixels_per_point: full.pixels_per_point,
        };
        // egui panics if a TexturesDelta is dropped with unapplied deltas.
        let mut textures = std::mem::take(&mut full.textures_delta);
        for (id, deltas) in &textures.set {
            for delta in deltas {
                self.renderer
                    .borrow_mut()
                    .update_texture(&gfx.device, &gfx.queue, *id, delta);
            }
        }
        let free = std::mem::take(&mut textures.free);
        textures.clear();
        let Some(frame) = gfx.acquire() else {
            self.free_textures(&free);
            return Ok(false);
        };
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = gfx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("studio frame"),
            });
        before_ui(&mut encoder);
        let extra = self.renderer.borrow_mut().update_buffers(
            &gfx.device,
            &gfx.queue,
            &mut encoder,
            &primitives,
            &screen,
        );
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(crate::ui::theme::CLEAR),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            self.renderer
                .borrow()
                .render(&mut pass, &primitives, &screen);
        }
        gfx.queue
            .submit(extra.into_iter().chain([encoder.finish()]));
        window.pre_present_notify();
        gfx.queue.present(frame);
        self.free_textures(&free);
        Ok(true)
    }

    fn free_textures<'a>(&mut self, ids: impl IntoIterator<Item = &'a egui::TextureId>) {
        for id in ids {
            self.renderer.borrow_mut().free_texture(id);
        }
    }
}
