#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::Result;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use syfrack_app_lib::display::DisplayPass;
use syfrack_app_lib::egui_layer::EguiLayer;
use syfrack_app_lib::input;
use syfrack_app_lib::library;
use syfrack_app_lib::preview::{self, Pick, Preview, Stage};
use syfrack_app_lib::state::{AppState, Banner, Tab};
use syfrack_app_lib::ui::form_panel::{FormPanel, FormView};
use syfrack_app_lib::ui::gallery_panel::GalleryPanel;
use syfrack_app_lib::ui::render_panel::RenderPanel;
use syfrack_app_lib::ui::viewport::ViewportOutput;
use syfrack_app_lib::ui::{self, Viewport};
use syfrack_app_lib::window::Gfx;
use syfrack_core::{clay_scene, CameraMode, Scene};
use syfrack_render::{GpuContext, Renderer};

/// Initial preview frame before the central panel reports its real size.
const INITIAL_FULL: (u32, u32) = (1024, 576);
/// Full-resolution frame of the preview on the « Forme » tab (16:9).
const FORM_PREVIEW_FULL: (u32, u32) = (1280, 720);
/// A panel size must stay unchanged this long before the session is rebuilt.
const RESIZE_SETTLE: Duration = Duration::from_millis(150);
/// How long a banner stays up after it was raised.
const BANNER_TTL: Duration = Duration::from_secs(8);
/// Minimum delay between two depth readbacks under a still cursor.
const DEPTH_HINT_PERIOD: Duration = Duration::from_millis(250);
/// Depth hint when the cursor points at the sky: move fast.
const DEPTH_HINT_MISS: f32 = 50.0;
/// While a final render runs, the preview steps one frame in this many.
const PREVIEW_SLOWDOWN: u64 = 4;
/// Redraw period while nothing can be presented (occluded, minimised).
const IDLE_PERIOD: Duration = Duration::from_millis(16);

struct Running {
    desktop: syfrack_app_lib::desktop::Desktop,
    last_frame: Instant,
    window: Arc<Window>,
    gfx: Gfx,
    egui: EguiLayer,
    state: AppState,
    renderer: Renderer,
    preview: Preview,
    display: DisplayPass,
    /// Full-resolution size the preview ladder was built for.
    full: (u32, u32),
    /// Candidate panel size and when it was first seen.
    pending_resize: Option<((u32, u32), Instant)>,
    /// A scene change held back until the pointer release (a new topology
    /// while a ⟳ slider is dragged: one compilation per release).
    deferred_change: bool,
    /// A size whose resize failed: not retried until the panel size changes.
    failed_resize: Option<(u32, u32)>,
    /// When the banner was last (re)raised.
    banner_at: Instant,
    /// Text of the banner the timer was started for: a new text restarts it.
    banner_seen: Option<String>,
    /// Size of the central area as reported by the last UI pass.
    available_px: (u32, u32),
    /// The last frame could not be presented: stop spinning.
    idle: bool,
    last_idle_redraw: Instant,
    /// Distance to the surface under the cursor, for the free-camera speed.
    depth_hint: f32,
    last_depth_hint: Instant,
    /// Last clicked surface point (world space) and its readback.
    last_pick: Option<(glam::Vec3, Pick)>,
    render: RenderPanel,
    gallery: GalleryPanel,
    form: FormPanel,
    /// The preview shows the clay variant of the scene.
    shown_clay: bool,
    /// Library thumbnails being rendered, each on its own thread.
    thumbnails: Vec<JoinHandle<Result<PathBuf>>>,
    frame_counter: u64,
    /// The window was asked to close over unsaved edits: the modal is up.
    confirm_quit: bool,
    /// The user confirmed: leave after this frame.
    quit_now: bool,
}

#[derive(Default)]
struct App {
    running: Option<Running>,
}

fn create(event_loop: &ActiveEventLoop) -> Result<Running> {
    let attrs = Window::default_attributes()
        .with_title("SyFrack Studio")
        .with_min_inner_size(winit::dpi::LogicalSize::new(1000.0, 640.0))
        .with_inner_size(winit::dpi::LogicalSize::new(1600.0, 900.0));
    let window = Arc::new(event_loop.create_window(attrs)?);
    let gfx = Gfx::new(window.clone())?;
    eprintln!("gpu: {} ({:?})", gfx.info.name, gfx.info.backend);
    let ctx = egui::Context::default();
    ui::theme::apply(&ctx);
    ui::floating::enable(&ctx);
    let egui = EguiLayer::new(ctx, &window, &gfx.device, gfx.config.format);
    // An unreadable scene is not fatal: start on the example and say so.
    let (scene, path, load_error) = match std::env::args().nth(1) {
        // An incomplete form graph opens (the preview waits for it); an
        // unreadable or broken file falls back to the example.
        Some(p) => {
            match syfrack_core::Scene::load(&p).and_then(|s| s.validate_editable().map(|_| s)) {
                Ok(s) => (s, Some(p.into()), None),
                Err(e) => (
                    syfrack_core::Scene::example(),
                    None,
                    Some(format!("{p} : {e:#}")),
                ),
            }
        }
        None => (syfrack_core::Scene::example_2d(), None, None),
    };
    let gpu = Arc::new(GpuContext::from_device(
        gfx.device.clone(),
        gfx.queue.clone(),
        gfx.info.clone(),
    ));
    let renderer = Renderer::new(gpu)?;
    let display = DisplayPass::new(&gfx.device)?;
    let preview = Preview::new(renderer.clone(), &scene, INITIAL_FULL)?;
    let mut state = AppState::new(scene, path);
    if let Some(text) = load_error {
        state.banner = Some(Banner::error(text));
    } else if let Some(reason) = preview.inactive_reason() {
        state.banner = Some(Banner::error(preview::incomplete_banner(reason)));
    }
    Ok(Running {
        desktop: Default::default(),
        last_frame: Instant::now(),
        window,
        gfx,
        egui,
        state,
        renderer,
        preview,
        display,
        full: INITIAL_FULL,
        pending_resize: None,
        deferred_change: false,
        failed_resize: None,
        banner_at: Instant::now(),
        banner_seen: None,
        available_px: INITIAL_FULL,
        idle: false,
        last_idle_redraw: Instant::now(),
        depth_hint: 1.0,
        last_depth_hint: Instant::now(),
        last_pick: None,
        render: RenderPanel::new(),
        gallery: GalleryPanel::new(),
        form: FormPanel::new(),
        shown_clay: false,
        thumbnails: Vec::new(),
        frame_counter: 0,
        confirm_quit: false,
        quit_now: false,
    })
}

impl Running {
    /// No render thread may outlive the window.
    fn quit(&mut self, event_loop: &ActiveEventLoop) {
        self.render.shutdown();
        for job in self.thumbnails.drain(..) {
            let _ = job.join();
        }
        event_loop.exit();
    }

    /// The scene the preview shows: the clay variant on the « Forme » tab
    /// when its switch asks for it.
    fn wants_clay(&self) -> bool {
        self.state.tab == Tab::Forme && self.form.clay
    }

    fn preview_scene(&self, clay: bool) -> Scene {
        if clay {
            clay_scene(&self.state.scene)
        } else {
            self.state.scene.clone()
        }
    }

    /// Adopts a pipeline compiled in the background, or reports its failure
    /// on the node the message names (the old pipeline keeps rendering).
    fn poll_compile(&mut self) {
        match self.preview.poll_compile() {
            None => {}
            Some(Ok(_)) => self.after_build(),
            Some(Err(e)) => {
                self.state.form.errors = preview::compile_error_node(&e)
                    .map(|n| vec![(n, e.clone())])
                    .unwrap_or_default();
                self.raise_banner(syfrack_i18n::tr_format!(
                    "preview: compilation : {e}",
                    e = e
                ));
            }
        }
    }

    /// Starts the thumbnails the library asked for and reports the finished ones.
    fn run_thumbnails(&mut self) {
        for (g, png) in self.form.library.thumbnail_jobs.drain(..) {
            let renderer = self.renderer.clone();
            let job = std::thread::Builder::new()
                .name("syfrack-library-thumbnail".into())
                .spawn(move || library::render_thumbnail(&renderer, &g, &png).map(|()| png));
            match job {
                Ok(h) => self.thumbnails.push(h),
                Err(e) => {
                    self.state.banner = Some(Banner::error(syfrack_i18n::tr_format!(
                        "vignette : {e}",
                        e = e
                    )))
                }
            }
        }
        let (done, running): (Vec<_>, Vec<_>) =
            self.thumbnails.drain(..).partition(|h| h.is_finished());
        self.thumbnails = running;
        for h in done {
            match h.join() {
                Ok(Ok(_)) => self.form.library.rescan_now(),
                Ok(Err(e)) => {
                    self.raise_banner(syfrack_i18n::tr_format!("vignette : {e:#}", e = e))
                }
                Err(_) => {
                    self.raise_banner(syfrack_i18n::tr!("vignette : le rendu a échoué").into())
                }
            }
        }
    }

    fn raise_banner(&mut self, text: String) {
        self.state.banner = Some(Banner::error(text));
        self.banner_at = Instant::now();
    }

    /// After a preview build that succeeded: « forme incomplète » while the
    /// preview waits, else the preview's own banners go.
    fn after_build(&mut self) {
        // The wanted topology is known not to compile: its banner stays.
        if self.preview.known_failure().is_some() {
            return;
        }
        let current = self.state.banner.as_ref().map(|b| b.text.as_str());
        match preview::banner_after_build(current, self.preview.inactive_reason()) {
            preview::BannerChange::Keep => {}
            preview::BannerChange::Clear => self.state.banner = None,
            preview::BannerChange::Raise(text) => self.raise_banner(text),
        }
    }

    /// Applies a panel size that has been stable long enough.
    fn settle_resize(&mut self) {
        let now = Instant::now();
        let target = self.available_px;
        if self.failed_resize.is_some_and(|f| f != target) {
            self.failed_resize = None;
        }
        if target.0 == 0
            || target.1 == 0
            || target == self.full
            || self.failed_resize == Some(target)
        {
            self.pending_resize = None;
            return;
        }
        match self.pending_resize {
            Some((size, since)) if size == target => {
                if now.duration_since(since) >= RESIZE_SETTLE {
                    match self.preview.resize(target) {
                        Ok(()) => {
                            self.full = target;
                            self.after_build();
                        }
                        Err(e) => {
                            self.failed_resize = Some(target);
                            self.raise_banner(format!("preview resize: {e:#}"));
                        }
                    }
                    self.pending_resize = None;
                }
            }
            _ => self.pending_resize = Some((target, now)),
        }
    }

    /// Reacts to the viewport: serves pick and depth-hint readbacks from the
    /// session as it was drawn, then rebuilds the preview after any scene edit.
    fn apply_viewport(&mut self, out: ViewportOutput) {
        let clay = self.wants_clay();
        // Switching between clay and the real look is a values-only change.
        let changed =
            self.state.take_preview_dirty() | (clay != self.shown_clay) | self.deferred_change;
        // A freshly rebuilt session has no samples yet: nothing to read back.
        let ready = self.preview.samples_min() >= 1;
        let (w, h) = self.preview.size();
        let aspect = w as f32 / h.max(1) as f32;
        if let (Some((u, v)), true, false) = (out.pick_request, ready, changed) {
            match self.preview.pick(u, v) {
                Ok(Some(pick)) => {
                    let basis = self.state.scene.camera.basis();
                    let point = input::unproject(&basis, aspect, u, v, pick.depth);
                    self.depth_hint = pick.depth;
                    self.last_pick = Some((point, pick));
                }
                Ok(None) => {
                    self.last_pick = None;
                    self.depth_hint = DEPTH_HINT_MISS;
                }
                Err(e) => self.raise_banner(format!("pick: {e:#}")),
            }
        } else if let (Some((u, v)), true, true) = (out.hover_uv, out.pointer_still, ready) {
            let free = matches!(self.state.scene.camera.mode, CameraMode::Free { .. });
            if free && self.last_depth_hint.elapsed() >= DEPTH_HINT_PERIOD {
                self.last_depth_hint = Instant::now();
                match self.preview.depth(u, v) {
                    Ok(Some(d)) => self.depth_hint = d,
                    Ok(None) => self.depth_hint = DEPTH_HINT_MISS,
                    Err(_) => {}
                }
            }
        }
        if changed {
            self.last_pick = None;
            let scene = self.preview_scene(clay);
            self.deferred_change = self.preview.defers(&scene, self.egui.pointer_down());
            if self.deferred_change {
                return;
            }
            self.shown_clay = clay;
            match self.preview.set_scene(&scene) {
                Err(e) => self.raise_banner(format!("preview: {e:#}")),
                Ok(()) => self.after_build(),
            }
        }
    }

    fn redraw(&mut self) {
        self.last_frame = Instant::now();
        self.settle_resize();
        self.frame_counter += 1;
        self.poll_compile();
        self.run_thumbnails();
        let render_running = self.render.is_running();
        // The gallery needs no preview: the GPU goes to a running final render.
        let preview_due = matches!(self.state.tab, Tab::Atelier | Tab::Forme)
            && (!render_running || self.frame_counter.is_multiple_of(PREVIEW_SLOWDOWN));
        if !self.idle && preview_due {
            if let Err(e) = self.preview.tick(Duration::from_millis(12)) {
                eprintln!("preview error: {e:#}");
                self.raise_banner(format!("preview: {e:#}"));
            }
        }
        if let Err(e) = self.renderer.context().check() {
            self.raise_banner(format!("{e:#}"));
        }
        let text = self.state.banner.as_ref().map(|b| b.text.clone());
        if text.is_some() && text != self.banner_seen {
            self.banner_at = Instant::now();
        }
        self.banner_seen = text;
        if self.state.banner.is_some() && self.banner_at.elapsed() >= BANNER_TTL {
            self.state.banner = None;
        }

        let size = self.preview.size();
        let texture =
            self.display
                .ensure_target(&self.gfx.device, &mut self.egui.renderer_mut(), size);
        let spp = self.preview.samples_min();
        let active = self.preview.session().is_some();
        let stage = match self.preview.stage() {
            Stage::Quarter => "quart",
            Stage::Half => "demi",
        };
        let compiling = if self.preview.compiling() {
            syfrack_i18n::tr!(" · compilation…")
        } else {
            ""
        };
        let caption = if !active && self.preview.compiling() {
            syfrack_i18n::tr!("compilation…").to_string()
        } else if !active {
            syfrack_i18n::text(preview::INCOMPLETE).to_owned()
        } else if render_running {
            syfrack_i18n::tr_format!(
                "{stage} · {spp} spp{compiling} · rendu en cours · aperçu ralenti",
                compiling = compiling,
                spp = spp,
                stage = stage
            )
        } else {
            format!("{stage} · {spp} spp{compiling}")
        };
        self.form.view = FormView {
            texture: active.then_some(texture),
            size,
            caption: caption.clone(),
        };
        self.form.compile_ms = self.preview.last_compile_ms().map(|ms| ms as f32);
        let mut view = Viewport {
            texture: active.then_some(texture),
            size,
            caption,
            available_px: self.available_px,
            depth_hint: self.depth_hint,
            pick: self.last_pick.clone(),
            frame_guide: self.render.frame_guide(),
            output: Default::default(),
        };

        let device = self.gfx.device.clone();
        let queue = self.gfx.queue.clone();
        let gain = self.preview.exposure_gain();
        let Running {
            window,
            gfx,
            egui,
            state,
            preview,
            display,
            render,
            gallery,
            form,
            renderer,
            confirm_quit,
            quit_now,
            ..
        } = self;
        let look = state.scene.look.clone();
        let result = egui.frame(
            window,
            gfx,
            |encoder| {
                // Nothing to show while the preview waits for a complete form.
                if let Some(session) = preview.session() {
                    display.encode(
                        &device,
                        &queue,
                        encoder,
                        session.accumulation_buffer(),
                        size,
                        &look,
                        gain,
                    );
                }
            },
            |ui| {
                ui::floating::begin_root(ui.ctx());
                ui::draw(ui, state, &mut view, gallery, form);
                render.show(ui, state, renderer);
                if *confirm_quit {
                    match ui::confirm_quit(ui.ctx(), egui::Id::new("quit-confirm")) {
                        Some(true) => {
                            *confirm_quit = false;
                            *quit_now = true;
                        }
                        Some(false) => *confirm_quit = false,
                        None => {}
                    }
                }
                ui::floating::finish_root(ui.ctx());
            },
        );
        // The « Forme » tab shows the preview in a 16:9 box: its quarter
        // stage is about the box's size.
        self.available_px = if self.state.tab == Tab::Forme {
            FORM_PREVIEW_FULL
        } else {
            view.available_px
        };
        self.apply_viewport(view.output);
        match result {
            Ok(presented) => self.idle = !presented,
            Err(e) => {
                eprintln!("frame error: {e:#}");
                self.idle = true;
            }
        }
        if self.idle {
            self.last_idle_redraw = Instant::now();
        } else {
            self.window.request_redraw();
        }
    }

    fn redraw_tool(&mut self, id: egui::ViewportId) {
        let Some(mut tool) = self.desktop.windows.remove(&id) else {
            return;
        };
        if !tool.minimized {
            let result = tool.egui.frame(
                &tool.window,
                &mut tool.gfx,
                |_| {},
                |ui| {
                    ui::dialogs(ui, &mut self.state, &mut self.gallery, &mut self.form);
                    self.render.show(ui, &mut self.state, &self.renderer);
                    if self.confirm_quit {
                        match ui::confirm_quit(ui.ctx(), egui::Id::new("quit-confirm")) {
                            Some(true) => {
                                self.confirm_quit = false;
                                self.quit_now = true;
                            }
                            Some(false) => self.confirm_quit = false,
                            None => {}
                        }
                    }
                },
            );
            if let Err(e) = result {
                self.raise_banner(syfrack_i18n::tr_format!("fenêtre : {e:#}", e = e));
            }
        }
        self.desktop.windows.insert(id, tool);
        self.window.request_redraw();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.is_some() {
            return;
        }
        match create(event_loop) {
            Ok(r) => self.running = Some(r),
            Err(e) => {
                eprintln!("startup failed: {e:#}");
                startup_error(&syfrack_i18n::tr_format!(
                    "Impossible de démarrer SyFrack Studio.\n\n{e:#}",
                    e = e
                ));
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(r) = &mut self.running else { return };
        if id != r.window.id() {
            let Some(viewport) = r.desktop.viewport(id) else {
                return;
            };
            let tool = r.desktop.windows.get_mut(&viewport).unwrap();
            let response = tool.egui.on_window_event(&tool.window, &event);
            match event {
                WindowEvent::CloseRequested => {
                    ui::floating::close(&tool.egui.context(), viewport);
                    r.window.request_redraw();
                }
                WindowEvent::Resized(size) => {
                    tool.minimized = size.width == 0 || size.height == 0;
                    tool.gfx.resize(size.width, size.height);
                    tool.window.request_redraw();
                }
                WindowEvent::RedrawRequested => {
                    r.redraw_tool(viewport);
                    if r.quit_now {
                        r.quit(event_loop);
                    }
                }
                _ => {
                    if response.repaint {
                        tool.window.request_redraw();
                    }
                }
            }
            return;
        }
        let response = r.egui.on_window_event(&r.window, &event);
        match event {
            WindowEvent::CloseRequested => {
                if r.state.has_unsaved_workspaces() {
                    // Ask first; the modal is drawn by the next frame.
                    r.confirm_quit = true;
                    r.idle = false;
                    r.window.request_redraw();
                } else {
                    r.quit(event_loop);
                }
            }
            WindowEvent::Resized(size) => r.gfx.resize(size.width, size.height),
            WindowEvent::RedrawRequested => {
                r.redraw();
                for error in r.desktop.sync(event_loop, &r.window, &r.gfx, &r.egui) {
                    r.raise_banner(error);
                }
                if r.quit_now {
                    r.quit(event_loop);
                }
            }
            _ => {}
        }
        if response.repaint {
            r.window.request_redraw();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(r) = &mut self.running else { return };
        // Windows may stop delivering WM_PAINT to a parent completely covered
        // by its tool windows. Keep document actions, progress and close
        // acknowledgements alive even when no root redraw event arrives.
        if r.last_frame.elapsed() >= Duration::from_millis(100) {
            r.redraw();
            for error in r.desktop.sync(event_loop, &r.window, &r.gfx, &r.egui) {
                r.raise_banner(error);
            }
            if r.quit_now {
                r.quit(event_loop);
                return;
            }
        }
        for tool in r.desktop.windows.values() {
            if !tool.minimized {
                tool.window.request_redraw();
            }
        }
        if r.idle {
            // Nothing to present: wake at a low rate instead of spinning.
            let due = r.last_idle_redraw + IDLE_PERIOD;
            if Instant::now() >= due {
                r.window.request_redraw();
                event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + IDLE_PERIOD));
            } else {
                event_loop.set_control_flow(ControlFlow::WaitUntil(due));
            }
        } else {
            event_loop.set_control_flow(ControlFlow::Poll);
            r.window.request_redraw();
        }
    }
}

fn run() -> Result<()> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App::default();
    event_loop.run_app(&mut app)?;
    Ok(())
}

fn main() {
    syfrack_i18n::initialize();
    if let Err(e) = run() {
        startup_error(&format!("SyFrack Studio\n\n{e:#}"));
    }
}

/// A GUI executable has no console to report a GPU/startup failure into.
#[cfg(target_os = "windows")]
fn startup_error(message: &str) {
    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(
            hwnd: *mut std::ffi::c_void,
            text: *const u16,
            caption: *const u16,
            kind: u32,
        ) -> i32;
    }
    let text: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
    let caption: Vec<u16> = "SyFrack Studio".encode_utf16().chain(Some(0)).collect();
    // Null owner is intentional: startup may fail before any window exists.
    unsafe {
        MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), 0x10);
    }
}

#[cfg(not(target_os = "windows"))]
fn startup_error(message: &str) {
    eprintln!("{message}");
}
