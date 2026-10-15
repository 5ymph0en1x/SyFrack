//! Desktop dialogs share the document and egui texture atlas, but have their
//! own native viewport. The embedded path also supports offscreen UI tests.
use egui::{Context, Id, Ui, Vec2, ViewportId};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub struct Request {
    pub id: ViewportId,
    pub title: String,
    pub size: Vec2,
    pub min_size: Vec2,
    pub modal: bool,
}

#[derive(Clone, Default)]
struct Registry {
    enabled: bool,
    requests: HashMap<ViewportId, Request>,
    closed: HashSet<ViewportId>,
    modal: Option<ViewportId>,
}

fn key() -> Id {
    Id::new("native-dialog-registry")
}
fn read(ctx: &Context) -> Registry {
    ctx.data(|d| d.get_temp(key())).unwrap_or_default()
}
fn edit(ctx: &Context, f: impl FnOnce(&mut Registry)) {
    ctx.data_mut(|d| f(d.get_temp_mut_or_default::<Registry>(key())));
}
pub fn enable(ctx: &Context) {
    ctx.set_embed_viewports(false);
    edit(ctx, |r| r.enabled = true);
}
pub fn begin_root(ctx: &Context) {
    edit(ctx, |r| r.requests.clear());
}
pub fn requests(ctx: &Context) -> Vec<Request> {
    read(ctx).requests.into_values().collect()
}
pub fn close(ctx: &Context, id: ViewportId) {
    edit(ctx, |r| {
        r.closed.insert(id);
    });
}
pub fn finish_root(ctx: &Context) {
    edit(ctx, |r| {
        if !r.modal.is_some_and(|id| r.requests.contains_key(&id)) {
            r.modal = r.requests.values().find(|v| v.modal).map(|v| v.id);
        }
    });
}
pub fn blocked(ctx: &Context) -> bool {
    let r = read(ctx);
    r.enabled && r.modal.is_some_and(|id| id != ctx.viewport_id())
}
pub fn is_secondary(ctx: &Context) -> bool {
    ctx.viewport_id() != ViewportId::ROOT
}

pub struct Window<'a> {
    inner: egui::Window<'a>,
    request: Request,
    open: Option<&'a mut bool>,
}
impl<'a> Window<'a> {
    pub fn new(title: &str) -> Self {
        let key = syfrack_i18n::canonical_key(title);
        let title = syfrack_i18n::text(key);
        let size = match key {
            "Rendre / Exporter" => egui::vec2(1000.0, 860.0),
            "Enregistrer la forme" => egui::vec2(520.0, 270.0),
            "Prendre en main le Studio" => egui::vec2(600.0, 650.0),
            _ => egui::vec2(860.0, 650.0),
        };
        Self {
            inner: egui::Window::new(title.to_owned()).id(Id::new(key)),
            request: Request {
                id: ViewportId(Id::new(key)),
                title: title.into(),
                size,
                min_size: egui::vec2(
                    if key == "Rendre / Exporter" {
                        760.0
                    } else {
                        420.0
                    },
                    if key == "Rendre / Exporter" {
                        600.0
                    } else {
                        220.0
                    },
                ),
                modal: false,
            },
            open: None,
        }
    }
    pub fn id(mut self, id: Id) -> Self {
        self.inner = self.inner.id(id);
        self.request.id = ViewportId(id);
        self
    }
    pub fn open(mut self, open: &'a mut bool) -> Self {
        self.open = Some(open);
        self
    }
    pub fn collapsible(mut self, v: bool) -> Self {
        self.inner = self.inner.collapsible(v);
        self
    }
    pub fn modal(mut self, modal: bool) -> Self {
        self.request.modal = modal;
        self
    }
    pub fn resizable(mut self, v: bool) -> Self {
        self.inner = self.inner.resizable(v);
        self
    }
    pub fn default_pos(mut self, v: egui::Pos2) -> Self {
        self.inner = self.inner.default_pos(v);
        self
    }
    pub fn default_width(mut self, v: f32) -> Self {
        self.inner = self.inner.default_width(v);
        self
    }
    pub fn min_width(mut self, v: f32) -> Self {
        self.inner = self.inner.min_width(v);
        self.request.min_size.x = v + 40.0;
        self
    }
    pub fn min_height(mut self, v: f32) -> Self {
        self.inner = self.inner.min_height(v);
        self.request.min_size.y = v + 40.0;
        self
    }
    pub fn max_width(mut self, v: f32) -> Self {
        self.inner = self.inner.max_width(v);
        self
    }
    pub fn max_height(mut self, v: f32) -> Self {
        self.inner = self.inner.max_height(v);
        self
    }
    pub fn vscroll(mut self, v: bool) -> Self {
        self.inner = self.inner.vscroll(v);
        self
    }
    pub fn show(mut self, ctx: &Context, body: impl FnOnce(&mut Ui)) {
        if !read(ctx).enabled {
            if let Some(open) = self.open {
                self.inner = self.inner.open(open);
            }
            self.inner.show(ctx, body);
            return;
        }
        let id = self.request.id;
        let mut closed = false;
        edit(ctx, |r| {
            closed = r.closed.remove(&id);
        });
        closed |= ctx.viewport_id() == id
            && !blocked(ctx)
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if closed {
            if let Some(open) = self.open {
                *open = false;
            }
            edit(ctx, |r| {
                r.requests.remove(&id);
            });
            return;
        }
        let builder = egui::ViewportBuilder::default()
            .with_title(format!("{} — SyFrack", self.request.title))
            .with_inner_size(self.request.size);
        edit(ctx, |r| {
            r.requests.insert(id, self.request);
        });
        if ctx.viewport_id() == ViewportId::ROOT {
            ctx.show_viewport_deferred(id, builder, |_, _| {});
        } else if ctx.viewport_id() == id {
            egui::Area::new(id.0.with("client"))
                .fixed_pos(egui::Pos2::ZERO)
                .movable(false)
                .constrain(false)
                .show(ctx, |ui| {
                    ui.set_width(ctx.content_rect().width());
                    egui::Frame::NONE
                        .fill(super::theme::PANEL)
                        .inner_margin(20)
                        .show(ui, |ui| {
                            ui.set_height((ctx.content_rect().height() - 40.0).max(1.0));
                            ui.add_enabled_ui(!blocked(ctx), |ui| {
                                egui::ScrollArea::vertical()
                                    .id_salt(id)
                                    .auto_shrink([false, false])
                                    .show(ui, body);
                            });
                        });
                });
        }
    }
}

pub struct Modal {
    id: Id,
    title: String,
}
pub struct ModalResponse {
    close: bool,
}
impl ModalResponse {
    pub fn should_close(&self) -> bool {
        self.close
    }
}
impl Modal {
    pub fn new(id: Id) -> Self {
        Self {
            id,
            title: "Confirmation".into(),
        }
    }
    pub fn title(mut self, title: &str) -> Self {
        self.title = title.into();
        self
    }
    pub fn show(self, ctx: &Context, body: impl FnOnce(&mut Ui)) -> ModalResponse {
        if !read(ctx).enabled {
            return ModalResponse {
                close: egui::Modal::new(self.id).show(ctx, body).should_close(),
            };
        }
        let mut open = true;
        let mut window = Window::new(&self.title).id(self.id).open(&mut open);
        window.request.size = egui::vec2(540.0, 208.0);
        window.request.min_size = window.request.size;
        window.request.modal = true;
        window.show(ctx, body);
        ModalResponse { close: !open }
    }
}

/// Shared confirmation hierarchy: question, consequences, then right-aligned
/// actions. The native footer follows the bottom edge when the window grows.
pub fn confirmation(
    ctx: &Context,
    id: Id,
    title: &str,
    accept: &str,
    cancel: &str,
    body: impl FnOnce(&mut Ui),
) -> Option<bool> {
    let mut answer = None;
    let response = Modal::new(id).title(title).show(ctx, |ui| {
        if !is_secondary(ctx) {
            ui.set_width(500.0);
        }
        ui.heading(title);
        ui.add_space(6.0);
        body(ui);
        ui.add_space(12.0);
        if is_secondary(ctx) {
            let footer = ctx.content_rect().bottom() - 20.0 - 54.0;
            ui.add_space((footer - ui.cursor().min.y).max(0.0));
        }
        ui.separator();
        ui.add_space(6.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let go = ui.add(
                egui::Button::new(egui::RichText::new(accept).color(super::theme::ERROR))
                    .fill(egui::Color32::from_rgb(64, 39, 43))
                    .min_size(egui::vec2(100.0, 32.0)),
            );
            #[cfg(test)]
            ui.data_mut(|d| d.insert_temp(Id::new("confirm-discard-button"), go.rect));
            if go.clicked() {
                answer = Some(true);
            }
            if ui
                .add(egui::Button::new(cancel).min_size(egui::vec2(100.0, 32.0)))
                .clicked()
            {
                answer = Some(false);
            }
        });
    });
    if response.should_close() {
        Some(false)
    } else {
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(ctx: &Context, viewport: ViewportId, body: impl FnOnce()) {
        let mut raw = egui::RawInput {
            viewport_id: viewport,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(720.0, 840.0),
            )),
            ..Default::default()
        };
        for r in requests(ctx) {
            raw.viewports.insert(
                r.id,
                egui::ViewportInfo {
                    parent: Some(ViewportId::ROOT),
                    ..Default::default()
                },
            );
        }
        let mut body = Some(body);
        ctx.run_ui(raw, |_| {
            if let Some(body) = body.take() {
                body();
            }
        })
        .textures_delta
        .clear();
    }
    #[test]
    fn content_runs_only_in_its_own_viewport_and_os_close_acknowledges_once() {
        let ctx = Context::default();
        enable(&ctx);
        let id = ViewportId(Id::new("Rendre / Exporter"));
        let mut open = true;
        let mut calls = 0;
        frame(&ctx, ViewportId::ROOT, || {
            begin_root(&ctx);
            Window::new("Rendre / Exporter")
                .open(&mut open)
                .show(&ctx, |_| calls += 1);
            finish_root(&ctx);
        });
        assert_eq!(calls, 0);
        assert_eq!(requests(&ctx).len(), 1);
        frame(&ctx, id, || {
            Window::new("Rendre / Exporter")
                .open(&mut open)
                .show(&ctx, |_| calls += 1)
        });
        assert_eq!(calls, 1);
        close(&ctx, id);
        frame(&ctx, ViewportId::ROOT, || {
            begin_root(&ctx);
            Window::new("Rendre / Exporter")
                .open(&mut open)
                .show(&ctx, |_| calls += 1);
            finish_root(&ctx);
        });
        assert!(!open);
        assert!(requests(&ctx).is_empty());
        open = true;
        frame(&ctx, ViewportId::ROOT, || {
            Window::new("Rendre / Exporter")
                .open(&mut open)
                .show(&ctx, |_| {})
        });
        assert!(open, "a fresh reopening is not consumed by a stale close");
    }
    #[test]
    fn confirmation_blocks_other_viewports_until_cancelled() {
        let ctx = Context::default();
        enable(&ctx);
        let id = Id::new("test-confirmation");
        frame(&ctx, ViewportId::ROOT, || {
            begin_root(&ctx);
            assert!(!Modal::new(id).show(&ctx, |_| {}).should_close());
            finish_root(&ctx);
            assert!(blocked(&ctx));
        });
        frame(&ctx, ViewportId(id), || assert!(!blocked(&ctx)));
        close(&ctx, ViewportId(id));
        frame(&ctx, ViewportId::ROOT, || {
            begin_root(&ctx);
            assert!(Modal::new(id)
                .show(&ctx, |_| panic!("closed content must not run"))
                .should_close());
            finish_root(&ctx);
            assert!(!blocked(&ctx));
        });
    }
}
