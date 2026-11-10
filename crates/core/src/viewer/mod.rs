mod blit;
mod gfx;
mod opener;
mod scale;
mod title;
mod touch;
mod ui;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use gfx::Gfx;
use opener::{Opener, on_ball};
pub use scale::Budget;
use snafu::{ResultExt, Snafu};
use title::FrameCounter;
use touch::{Gesture, Touches};
use ui::{Command, Info, Overlay, Painted, Ring};
use web_time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalPosition;
use winit::error::{EventLoopError, OsError};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, StartCause, Touch, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::camera::{Camera, PIXELS_PER_LINE};
use crate::gpu;
use crate::renderer::{FieldView, PassTimes, Uniforms};
use crate::settings::{Action, Chord, Settings};
use crate::shaders::Sources;

const TITLE: &str = "currents";

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("could not create the event loop"))]
    EventLoop { source: EventLoopError },
    #[snafu(display("the event loop failed"))]
    RunEventLoop { source: EventLoopError },
    #[snafu(display("could not create the window"))]
    CreateWindow { source: OsError },
    #[snafu(display("could not create a surface for the window"))]
    CreateSurface { source: wgpu::CreateSurfaceError },
    #[snafu(display("could not open a graphics device"))]
    Gpu { source: gpu::Error },
    #[snafu(display("the graphics adapter does not support the window's surface"))]
    NoSurfaceConfig,
    #[snafu(display("the shaders did not compile on this device: {message}"))]
    Pipelines { message: String },
}

/// Where the viewer keeps settings between runs. The viewer calls `save` a moment after the
/// last change and once more on the way out.
pub trait Store {
    fn save(&mut self, settings: &Settings) -> Result<(), String>;
    /// The file settings are saved to, as a person would type it.
    fn location(&self) -> Option<String>;
    /// Opens the file in whatever edits it.
    fn open(&self) -> Result<(), String>;
    /// Why settings are not being saved, when they are not.
    fn problem(&self) -> Option<String>;
}

/// Everything the viewer starts from.
pub struct Launch {
    pub settings: Settings,
    pub sources: Sources,
    /// The field view to start in instead of the shaded picture; D cycles through them.
    pub debug: Option<FieldView>,
    /// `None` keeps changes for this run only.
    pub store: Option<Box<dyn Store>>,
    /// Problems the settings were read with, shown once at startup and kept in the menu.
    pub issues: Vec<String>,
}

/// Page callbacks: `on_ready` after the first frame, `on_error` with the reason the viewer stopped.
#[derive(Default)]
pub struct Hooks {
    pub on_ready: Option<Box<dyn FnOnce()>>,
    pub on_error: Option<Box<dyn FnMut(String)>>,
}

/// A future the platform drives to completion: native blocks on it, the web spawns it.
pub type LocalFuture = Pin<Box<dyn Future<Output = ()>>>;

/// Carries the finished graphics setup from the startup future back to the event loop.
struct Started(Result<Box<Gfx>, Error>);

/// Seconds after the last change before settings are saved.
const SAVE_AFTER: Duration = Duration::from_secs(1);
/// Weight of the newest frame in the frame rate the menu shows.
const FPS_SMOOTHING: f32 = 0.05;

#[expect(clippy::struct_excessive_bools, reason = "independent pieces of the event loop's state")]
struct App {
    settings: Settings,
    /// The settings the canvas was last brought in line with; it follows `settings` between frames.
    drawn_with: Settings,
    sources: Sources,
    store: Option<Box<dyn Store>>,
    issues: Vec<String>,
    /// When settings last changed without being saved since.
    unsaved: Option<Instant>,
    instance: wgpu::Instance,
    spawn: Box<dyn Fn(LocalFuture)>,
    proxy: EventLoopProxy<Started>,
    #[cfg(target_arch = "wasm32")]
    canvas: Option<web_sys::HtmlCanvasElement>,
    starting: bool,
    gfx: Option<Gfx>,
    overlay: Option<Overlay>,
    /// Whether the overlay drew last frame, which is when input goes to it.
    overlay_live: bool,
    hooks: Hooks,
    camera: Camera,
    view: Option<FieldView>,
    paused: bool,
    hidden: bool,
    time: f32,
    last: Instant,
    fps: f32,
    passes: PassTimes,
    cursor: PhysicalPosition<f64>,
    modifiers: ModifiersState,
    dragging: bool,
    opener: Opener,
    /// The action waiting for the next key press to become its chord.
    capturing: Option<Action>,
    touches: Touches,
    title: FrameCounter,
    failure: Option<Error>,
}

fn frame_uniforms(settings: &Settings, camera: &Camera, time: f32, gfx: &Gfx) -> Uniforms {
    gfx.canvas.uniforms(time, camera.view(gfx.canvas.viewport()).to_array(), settings.params.0)
}

impl App {
    fn fail(&mut self, el: &ActiveEventLoop, error: Error) {
        if let Some(on_error) = self.hooks.on_error.as_mut() {
            on_error(snafu::Report::from_error(&error).to_string());
        }
        self.failure = Some(error);
        el.exit();
    }

    /// Advances the clock to `now`; returns the seconds since the last frame.
    fn advance_clock(&mut self, now: Instant) -> f32 {
        let dt = (now - self.last).as_secs_f32();
        if !self.paused {
            self.time += dt * self.settings.motion.speed;
        }
        self.last = now;
        if dt > 0.0 {
            self.fps += FPS_SMOOTHING * (dt.recip() - self.fps);
        }
        dt
    }

    fn update_title(&mut self, now: Instant, passes: PassTimes) {
        if cfg!(target_arch = "wasm32") {
            return;
        }
        if let Some(title) = self.title.tick(now, passes, self.time, self.paused, self.view)
            && let Some(g) = self.gfx.as_ref()
        {
            g.window.set_title(&title);
        }
    }

    /// Takes on `new` settings and queues a save; the canvas follows before the next frame.
    fn change(&mut self, new: Settings) {
        if new == self.settings {
            return;
        }
        self.camera.tuning = new.camera;
        self.settings = new;
        if self.store.is_some() {
            self.unsaved.get_or_insert_with(Instant::now);
        }
    }

    fn save(&mut self) {
        let (Some(store), Some(_)) = (self.store.as_mut(), self.unsaved.take()) else { return };
        if let Err(reason) = store.save(&self.settings)
            && let Some(overlay) = self.overlay.as_mut()
        {
            overlay.toast(format!("Settings were not saved: {reason}"), true);
        }
    }

    fn info(&self) -> Info {
        Info {
            fps: self.fps,
            field_ms: self.passes.field,
            shade_ms: self.passes.shade,
            location: self.store.as_ref().and_then(|s| s.location()),
            problem: self.store.as_ref().and_then(|s| s.problem()),
            issues: self.issues.clone(),
            capturing: self.capturing,
        }
    }

    fn ring(&self, now: Instant) -> Option<Ring> {
        self.opener.progress(now).map(|progress| Ring { at: [self.cursor.x, self.cursor.y], progress })
    }

    /// Runs the overlay when anything shows; returns what it painted.
    fn overlay_frame(&mut self, now: Instant) -> Option<Painted> {
        let ring = self.ring(now);
        let (Some(g), Some(overlay)) = (self.gfx.as_ref(), self.overlay.as_mut()) else { return None };
        self.overlay_live = overlay.active(now, ring);
        if !self.overlay_live {
            return None;
        }
        let info = self.info();
        let mut settings = self.settings.clone();
        let overlay = self.overlay.as_mut()?;
        let (painted, commands) = overlay.frame(&g.window, &mut settings, &info, ring, g.canvas.surface_ratio());
        self.change(settings);
        for command in commands {
            self.command(command);
        }
        Some(painted)
    }

    fn command(&mut self, command: Command) {
        match command {
            Command::Close => self.set_menu(false),
            Command::ResetView => self.camera.reset(),
            Command::ResetDefaults => self.change(Settings::default()),
            Command::Capture(action) => self.capturing = Some(action),
            Command::OpenFile => {
                let result = self.store.as_ref().map(|s| s.open());
                if let (Some(Err(reason)), Some(overlay)) = (result, self.overlay.as_mut()) {
                    overlay.toast(format!("Could not open the settings file: {reason}"), true);
                }
            },
            Command::CopyPath => {
                let location = self.store.as_ref().and_then(|s| s.location());
                if let (Some(path), Some(overlay)) = (location, self.overlay.as_mut()) {
                    overlay.copy(path);
                    overlay.toast("Copied the settings file path".to_owned(), false);
                }
            },
        }
    }

    fn set_menu(&mut self, open: bool) {
        if let Some(overlay) = self.overlay.as_mut() {
            overlay.set_menu_open(open);
        }
        if !open {
            self.capturing = None;
        }
    }

    fn act(&mut self, el: &ActiveEventLoop, action: Action) {
        match action {
            Action::ToggleMenu => {
                let open = self.overlay.as_ref().is_some_and(Overlay::menu_open);
                self.set_menu(!open);
            },
            Action::Pause => self.paused = !self.paused,
            Action::ResetView => self.camera.reset(),
            Action::CycleDebug => self.view = FieldView::cycle(self.view),
            Action::ToggleTemporal => {
                let mut new = self.settings.clone();
                new.render.temporal = !new.render.temporal;
                self.change(new);
            },
            Action::Quit => el.exit(),
        }
    }

    /// A key that reached the viewer: the chord it binds, or else a hint at the menu key.
    fn key(&mut self, el: &ActiveEventLoop, code: KeyCode) {
        if Chord::is_modifier(code) {
            return;
        }
        if let Some(action) = self.settings.keys.action(&Chord::new(code, self.modifiers)) {
            return self.act(el, action);
        }
        let menu = self.settings.keys.toggle_menu.label();
        if let Some(overlay) = self.overlay.as_mut()
            && self.settings.wallpaper.hint
            && !overlay.menu_open()
        {
            overlay.toast(format!("Press {menu} for settings"), false);
        }
    }

    /// Binds the next key to the action waiting for one; Esc gives up instead.
    fn capture(&mut self, action: Action, code: KeyCode) {
        if Chord::is_modifier(code) {
            return;
        }
        self.capturing = None;
        if code == KeyCode::Escape {
            return;
        }
        let mut new = self.settings.clone();
        *new.keys.get_mut(action) = Chord::new(code, self.modifiers);
        self.change(new);
    }

    /// Schedules the next frame: at once under vsync, or when a capped frame rate allows.
    fn schedule(&self, el: &ActiveEventLoop, started: Instant) {
        let Some(g) = self.gfx.as_ref() else { return };
        if self.hidden && self.settings.wallpaper.pause_when_hidden {
            el.set_control_flow(ControlFlow::Wait);
            return;
        }
        match self.settings.render.fps_cap {
            0 => {
                el.set_control_flow(ControlFlow::Wait);
                g.window.request_redraw();
            },
            cap => el.set_control_flow(ControlFlow::WaitUntil(started + Duration::from_secs_f64(1.0 / f64::from(cap)))),
        }
    }

    fn redraw(&mut self, el: &ActiveEventLoop) {
        let now = Instant::now();
        let dt = self.advance_clock(now);
        if self.settings.wallpaper.ball_opener && self.opener.fired(now) {
            self.set_menu(true);
        }
        if self.unsaved.is_some_and(|since| now - since >= SAVE_AFTER) {
            self.save();
        }
        let Some(g) = self.gfx.as_mut() else { return };
        if self.drawn_with != self.settings {
            g.canvas.apply(&self.drawn_with, &self.settings);
            self.drawn_with.clone_from(&self.settings);
        }
        g.canvas.observe_frame(dt);
        self.camera.advance(dt, g.canvas.viewport());
        if let Some(frame) = g.acquire() {
            let painted = self.overlay_frame(now);
            let Some(g) = self.gfx.as_mut() else { return };
            let uniforms = frame_uniforms(&self.settings, &self.camera, self.time, g);
            self.passes = g.draw(frame, &uniforms, self.view, painted);
            self.update_title(now, self.passes);
            if let Some(on_ready) = self.hooks.on_ready.take() {
                on_ready();
            }
        }
        self.schedule(el, now);
    }

    fn touch(&mut self, touch: Touch) {
        let Some(g) = self.gfx.as_ref() else { return };
        let viewport = g.canvas.viewport();
        self.camera.hold(matches!(touch.phase, TouchPhase::Started | TouchPhase::Moved));
        match self.touches.event(touch.phase, touch.id, [touch.location.x, touch.location.y]) {
            Some(Gesture::Pan(delta)) => self.camera.pan(delta, viewport),
            Some(Gesture::Pinch { centre, pan, factor }) => {
                self.camera.pan(pan, viewport);
                self.camera.scale_extent(factor, centre, viewport);
            },
            None => {},
        }
    }

    fn on_ball(&self) -> bool {
        self.gfx
            .as_ref()
            .is_some_and(|g| on_ball(self.camera.cover_point([self.cursor.x, self.cursor.y], g.canvas.viewport())))
    }

    /// Pointer and touch input, once egui has passed on it.
    fn pointer(&mut self, event: &WindowEvent, consumed: bool) {
        let locked = self.settings.camera.locked;
        match *event {
            WindowEvent::CursorMoved { position, .. } => {
                if self.dragging
                    && let Some(g) = self.gfx.as_ref()
                {
                    self.camera.pan([position.x - self.cursor.x, position.y - self.cursor.y], g.canvas.viewport());
                }
                self.cursor = position;
                let on = self.on_ball();
                self.opener.moved(on);
            },
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                let owned = self.overlay.as_ref().is_some_and(Overlay::owns_pointer);
                self.dragging = state == ElementState::Pressed && !consumed && !owned && !locked;
                self.camera.hold(self.dragging);
            },
            WindowEvent::MouseInput { state, button: MouseButton::Right, .. } => match state {
                ElementState::Pressed if !consumed && self.settings.wallpaper.ball_opener => {
                    let on = self.on_ball();
                    self.opener.press(Instant::now(), on);
                },
                _ => self.opener.release(),
            },
            WindowEvent::MouseWheel { delta, .. } if !consumed && !locked => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / PIXELS_PER_LINE,
                };
                if let Some(g) = self.gfx.as_ref() {
                    self.camera.zoom(lines, [self.cursor.x, self.cursor.y], g.canvas.viewport());
                }
            },
            WindowEvent::Touch(touch) if !consumed && !locked => self.touch(touch),
            _ => {},
        }
    }
}

impl ApplicationHandler<Started> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.starting {
            return;
        }
        self.starting = true;
        let attrs = Window::default_attributes().with_title(TITLE);
        #[cfg(not(target_arch = "wasm32"))]
        let attrs = attrs.with_inner_size(winit::dpi::LogicalSize::new(900, 900));
        #[cfg(target_arch = "wasm32")]
        let attrs = {
            use winit::platform::web::WindowAttributesExtWebSys;
            attrs.with_canvas(self.canvas.take()).with_prevent_default(true)
        };
        let window = match el.create_window(attrs).context(CreateWindowSnafu) {
            Ok(window) => Arc::new(window),
            Err(error) => return self.fail(el, error),
        };
        let (instance, settings, sources, proxy) =
            (self.instance.clone(), self.settings.clone(), self.sources.clone(), self.proxy.clone());
        (self.spawn)(Box::pin(async move {
            let gfx = Gfx::create(window, instance, &settings, &sources).await.map(Box::new);
            if proxy.send_event(Started(gfx)).is_err() {
                tracing::warn!("event loop closed before the viewer started");
            }
        }));
    }

    fn user_event(&mut self, el: &ActiveEventLoop, event: Started) {
        match event.0 {
            Ok(gfx) => {
                gfx.window.request_redraw();
                let mut overlay = Overlay::new(&gfx.window, gfx.canvas.max_texture_side());
                if !self.issues.is_empty() {
                    let n = self.issues.len();
                    overlay.toast(format!("The settings file had {n} problem(s); see the menu"), true);
                }
                self.overlay = Some(overlay);
                self.last = Instant::now();
                self.gfx = Some(*gfx);
            },
            Err(error) => self.fail(el, error),
        }
    }

    fn new_events(&mut self, _el: &ActiveEventLoop, cause: StartCause) {
        if matches!(cause, StartCause::ResumeTimeReached { .. })
            && let Some(g) = self.gfx.as_ref()
        {
            g.window.request_redraw();
        }
    }

    fn exiting(&mut self, _el: &ActiveEventLoop) {
        self.save();
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if let WindowEvent::KeyboardInput { event: key, .. } = &event
            && key.state == ElementState::Pressed
            && let (Some(action), PhysicalKey::Code(code)) = (self.capturing, key.physical_key)
        {
            return self.capture(action, code);
        }
        let consumed = match (self.gfx.as_ref(), self.overlay.as_mut()) {
            (Some(g), Some(overlay)) => overlay.on_event(&g.window, &event, self.overlay_live),
            _ => false,
        };
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(size) => {
                if let Some(g) = self.gfx.as_mut() {
                    g.canvas.resize(size);
                }
            },
            WindowEvent::Occluded(hidden) => {
                self.hidden = hidden;
                if !hidden && let Some(g) = self.gfx.as_ref() {
                    g.window.request_redraw();
                }
            },
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::RedrawRequested => self.redraw(el),
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !event.repeat && !consumed =>
            {
                if let PhysicalKey::Code(code) = event.physical_key {
                    self.key(el, code);
                }
            },
            pointer => self.pointer(&pointer, consumed),
        }
    }
}

fn app(event_loop: &EventLoop<Started>, launch: Launch, hooks: Hooks, spawn: Box<dyn Fn(LocalFuture)>) -> App {
    #[cfg(not(target_arch = "wasm32"))]
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(Box::new(
        event_loop.owned_display_handle(),
    )));
    #[cfg(target_arch = "wasm32")]
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let now = Instant::now();
    let Launch { settings, sources, debug, store, issues } = launch;
    App {
        camera: Camera::new(settings.camera),
        view: debug,
        drawn_with: settings.clone(),
        settings,
        sources,
        store,
        issues,
        unsaved: None,
        instance,
        spawn,
        proxy: event_loop.create_proxy(),
        #[cfg(target_arch = "wasm32")]
        canvas: None,
        starting: false,
        gfx: None,
        overlay: None,
        overlay_live: false,
        hooks,
        paused: false,
        hidden: false,
        time: 0.0,
        last: now,
        fps: 0.0,
        passes: PassTimes::default(),
        cursor: PhysicalPosition::new(0.0, 0.0),
        modifiers: ModifiersState::empty(),
        dragging: false,
        opener: Opener::default(),
        capturing: None,
        touches: Touches::default(),
        title: FrameCounter::new(now),
        failure: None,
    }
}

/// Opens the interactive viewer and runs it until the window closes.
#[cfg(not(target_arch = "wasm32"))]
pub fn run(launch: Launch) -> Result<(), Error> {
    let event_loop = EventLoop::with_user_event().build().context(EventLoopSnafu)?;
    let mut app = app(&event_loop, launch, Hooks::default(), Box::new(pollster::block_on));
    event_loop.run_app(&mut app).context(RunEventLoopSnafu)?;
    app.failure.map_or(Ok(()), Err)
}

/// Starts the viewer on `canvas` under the browser's event loop and returns at once.
#[cfg(target_arch = "wasm32")]
pub fn spawn(launch: Launch, canvas: web_sys::HtmlCanvasElement, hooks: Hooks) -> Result<(), Error> {
    use winit::platform::web::EventLoopExtWebSys;

    let event_loop = EventLoop::with_user_event().build().context(EventLoopSnafu)?;
    let mut app = app(&event_loop, launch, hooks, Box::new(wasm_bindgen_futures::spawn_local));
    app.canvas = Some(canvas);
    event_loop.spawn_app(app);
    Ok(())
}
