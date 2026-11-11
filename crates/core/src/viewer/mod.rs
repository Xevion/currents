mod blit;
#[cfg(all(test, target_os = "linux"))]
mod e2e;
mod gfx;
mod opener;
mod pacing;
mod scale;
#[cfg(test)]
pub(crate) mod testing;
mod title;
mod touch;
mod ui;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use gfx::Gfx;
use opener::{Opener, on_ball};
pub use pacing::Conditions;
use pacing::{Pacing, pacing};
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
use crate::settings::{self, Action, Chord, Settings};
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
    pub placement: Placement,
    /// Runs once the window is up and drawing, with the handle that changes the viewer, so the
    /// host can do what the window toolkit cannot, like X11 hints or watching for fullscreen apps.
    pub on_window: Option<OnWindow>,
}

/// What a host runs once the viewer's window is up.
pub type OnWindow = Box<dyn FnOnce(Arc<Window>, Remote)>;

/// Where the viewer's window goes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Placement {
    /// An ordinary window.
    #[default]
    Window,
    /// Behind every other window on one monitor, as X11 desktops place a wallpaper; the monitor
    /// by name or index, the primary one without either.
    Desktop { monitor: Option<String> },
    /// Inside another program's X11 window, as xwinwrap gives one.
    Embed(u32),
}

/// What a page shares settings with; the menu offers Copy link only when there is one.
pub type Share = Box<dyn FnMut(&Settings)>;

/// Page callbacks: `on_ready` after the first frame, `on_error` with why the viewer stopped.
#[derive(Default)]
pub struct Hooks {
    pub on_ready: Option<Box<dyn FnOnce()>>,
    pub on_error: Option<Box<dyn FnMut(String)>>,
    /// Takes the settings the menu's Copy link button shares.
    pub share: Option<Share>,
}

/// A future the platform drives to completion: native blocks on it, the web spawns it.
pub type LocalFuture = Pin<Box<dyn Future<Output = ()>>>;

/// What reaches the event loop from outside it.
enum Event {
    /// The finished graphics setup, from the startup future.
    Started(Result<Box<Gfx>, Error>),
    /// One setting to change by dotted path; `None` switches an optional one off.
    Assign(String, Option<settings::toml::Value>),
    /// The host holding the viewer still, as a wallpaper host does while nothing shows it.
    Hold(bool),
    /// What the platform reports about the viewer's surroundings.
    Conditions(Conditions),
}

/// Changes a running viewer from outside its event loop, as a page or wallpaper host does.
#[derive(Clone)]
pub struct Remote(EventLoopProxy<Event>);

impl Remote {
    /// Changes the setting at dotted `path`, as in `motion.speed`; `None` switches an optional
    /// one off. A value the setting cannot take is logged and changes nothing.
    pub fn set(&self, path: impl Into<String>, value: Option<settings::toml::Value>) {
        self.send(Event::Assign(path.into(), value));
    }

    /// Stops drawing while `held`, picking up where it left off once released.
    pub fn hold(&self, held: bool) {
        self.send(Event::Hold(held));
    }

    /// Tells the viewer what the platform sees around it, which the pause settings act on.
    pub fn conditions(&self, conditions: Conditions) {
        self.send(Event::Conditions(conditions));
    }

    fn send(&self, event: Event) {
        if self.0.send_event(event).is_err() {
            tracing::debug!("the viewer has stopped; dropped a request");
        }
    }
}

/// Seconds to wait before asking again for a frame the surface did not give.
const RETRY_SECS: f64 = 0.05;
/// Seconds after the last change before settings are saved.
const SAVE_AFTER: Duration = Duration::from_secs(1);
/// Weight of the newest frame in the frame rate the menu shows.
const FPS_SMOOTHING: f32 = 0.05;
/// How far in logical pixels a held finger may drift and still be holding, not panning.
const TOUCH_SLOP: f64 = 10.0;

#[expect(clippy::struct_excessive_bools, reason = "independent pieces of the event loop's state")]
struct App {
    settings: Settings,
    /// The settings the canvas was last brought in line with; it follows `settings` between frames.
    drawn_with: Settings,
    sources: Sources,
    store: Option<Box<dyn Store>>,
    issues: Vec<String>,
    #[cfg_attr(target_arch = "wasm32", expect(dead_code, reason = "a page places its own canvas"))]
    placement: Placement,
    on_window: Option<OnWindow>,
    /// When settings last changed without being saved since.
    unsaved: Option<Instant>,
    instance: wgpu::Instance,
    spawn: Box<dyn Fn(LocalFuture)>,
    proxy: EventLoopProxy<Event>,
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
    /// Whether the host is holding the viewer still.
    held: bool,
    conditions: Conditions,
    /// Whether the window has keyboard focus; browsers slow the frames of a page without it.
    focused: bool,
    /// Whether the first touch has been told how to open the menu.
    touch_hinted: bool,
    time: f64,
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
    /// When the viewer was set up.
    started_at: Instant,
    /// Frames presented so far.
    presented: u64,
    /// The device fault last shown to the person, so each one is announced once.
    announced_fault: Option<String>,
    failure: Option<Error>,
}

fn frame_uniforms(settings: &Settings, camera: &Camera, time: f64, gfx: &Gfx) -> Uniforms {
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
            self.time += f64::from(dt) * f64::from(self.settings.motion.speed);
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
        tracing::debug!(changed = ?settings::toml::changed(&self.settings, &new), "settings changed");
        self.camera.tuning = new.camera;
        self.settings = new;
        if self.store.is_some() {
            self.unsaved.get_or_insert_with(Instant::now);
        }
    }

    fn assign(&mut self, path: &str, value: Option<settings::toml::Value>) {
        match settings::toml::assign(&self.settings, path, value) {
            Ok((new, issues)) => {
                for issue in &issues {
                    tracing::warn!(%issue, "setting held to its bounds");
                }
                tracing::info!(path, changed = ?settings::toml::changed(&self.settings, &new), "setting assigned");
                self.change(new);
            },
            Err(issue) => tracing::warn!(%issue, "setting not changed"),
        }
    }

    fn hold(&mut self, held: bool) {
        if held == self.held {
            return;
        }
        tracing::info!(held, "host hold");
        let was = self.pacing();
        self.held = held;
        self.resume_if_woken(was);
    }

    fn observe(&mut self, conditions: Conditions) {
        if conditions == self.conditions {
            return;
        }
        tracing::info!(?conditions, "surroundings changed");
        let was = self.pacing();
        self.conditions = conditions;
        self.resume_if_woken(was);
    }

    fn pacing(&self) -> Pacing {
        let attended = self.overlay_live || self.overlay.as_ref().is_some_and(Overlay::menu_open);
        pacing(&self.settings, self.conditions, self.hidden, self.held, attended)
    }

    /// Starts frames again after an idle spell, the clock picking up where it stopped instead
    /// of leaping over the pause.
    fn resume_if_woken(&mut self, was: Pacing) {
        if was == Pacing::Idle
            && self.pacing() != Pacing::Idle
            && let Some(g) = self.gfx.as_ref()
        {
            self.last = Instant::now();
            g.window.request_redraw();
        }
    }

    fn save(&mut self) {
        let (Some(store), Some(_)) = (self.store.as_mut(), self.unsaved.take()) else { return };
        if let Err(reason) = store.save(&self.settings) {
            tracing::error!(%reason, "settings were not saved");
            if let Some(overlay) = self.overlay.as_mut() {
                overlay.toast(format!("Settings were not saved: {reason}"), true);
            }
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
            can_share: self.hooks.share.is_some(),
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
        let (painted, commands) = overlay.frame(&g.window, &mut settings, &info, ring);
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
            Command::CopyLink => {
                if let Some(share) = self.hooks.share.as_mut() {
                    share(&self.settings);
                    if let Some(overlay) = self.overlay.as_mut() {
                        overlay.toast("Copied a link to these settings".to_owned(), false);
                    }
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
        if let Some(overlay) = self.overlay.as_mut()
            && overlay.menu_open() != open
        {
            tracing::debug!(open, "menu");
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
        let chord = Chord::new(code, self.modifiers);
        tracing::info!(action = %action, chord = %chord, "key bound");
        *new.keys.get_mut(action) = chord;
        self.change(new);
    }

    /// Schedules the next frame: at once under vsync, when a frame rate limit allows, or after
    /// a short wait when the surface gave no frame, so a surface that keeps refusing never
    /// spins the loop.
    fn schedule(&self, el: &ActiveEventLoop, started: Instant, presented: bool) {
        let Some(g) = self.gfx.as_ref() else { return };
        let cap = match self.pacing() {
            Pacing::Idle => {
                el.set_control_flow(ControlFlow::Wait);
                return;
            },
            Pacing::Run(cap) => cap,
        };
        let wait = if presented { cap.map(|cap| 1.0 / f64::from(cap)) } else { Some(RETRY_SECS) };
        match wait {
            None => {
                el.set_control_flow(ControlFlow::Wait);
                g.window.request_redraw();
            },
            Some(secs) => el.set_control_flow(ControlFlow::WaitUntil(started + Duration::from_secs_f64(secs))),
        }
    }

    /// Tells the person once about a fault that stopped the shading.
    fn announce_fault(&mut self) {
        let fault = self.gfx.as_ref().and_then(|g| g.canvas.fault());
        if fault == self.announced_fault {
            return;
        }
        if let (Some(fault), Some(overlay)) = (&fault, self.overlay.as_mut()) {
            overlay.toast(format!("Drawing stopped: {fault}. Change a setting to try again."), true);
        }
        self.announced_fault = fault;
    }

    fn redraw(&mut self, el: &ActiveEventLoop) {
        let now = Instant::now();
        let dt = self.advance_clock(now);
        // A touch hold opens the menu whatever the ball setting says, as a phone has no other way to.
        if (self.settings.wallpaper.ball_opener || self.touches.any()) && self.opener.fired(now) {
            self.touches.forget();
            self.camera.hold(false);
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
        g.canvas.observe_frame(dt, self.passes, self.focused && !self.hidden);
        self.camera.advance(dt, g.canvas.viewport());
        let frame = g.acquire();
        let presented = frame.is_some();
        if let Some(frame) = frame {
            self.announce_fault();
            let painted = self.overlay_frame(now);
            let Some(g) = self.gfx.as_mut() else { return };
            let uniforms = frame_uniforms(&self.settings, &self.camera, self.time, g);
            self.passes = g.draw(frame, &uniforms, self.view, painted);
            self.presented += 1;
            if self.presented == 1 {
                tracing::info!(elapsed_ms = self.started_at.elapsed().as_millis(), "first frame presented");
            }
            self.update_title(now, self.passes);
            if let Some(on_ready) = self.hooks.on_ready.take() {
                on_ready();
            }
        }
        self.schedule(el, now, presented);
    }

    /// Follows every finger, so one that lifts over the menu is never left counted as down,
    /// but moves the camera or starts a hold only for touches the overlay passed on.
    fn touch(&mut self, touch: Touch, consumed: bool) {
        let Some(g) = self.gfx.as_ref() else { return };
        let viewport = g.canvas.viewport();
        let slop = TOUCH_SLOP * g.window.scale_factor();
        let on_menu = self.overlay.as_ref().is_some_and(|o| o.covers([touch.location.x, touch.location.y]));
        if (consumed || on_menu) && touch.phase == TouchPhase::Started {
            return;
        }
        if touch.phase == TouchPhase::Started && !self.touch_hinted && self.settings.wallpaper.hint {
            self.touch_hinted = true;
            if let Some(overlay) = self.overlay.as_mut() {
                overlay.toast("Press and hold for settings".to_owned(), false);
            }
        }
        let gesture = self.touches.event(touch.phase, touch.id, [touch.location.x, touch.location.y]);
        self.camera.hold(self.touches.any());
        match self.touches.still(slop) {
            Some(at) => {
                self.cursor = PhysicalPosition::new(at[0], at[1]);
                if touch.phase == TouchPhase::Started {
                    self.opener.touch(Instant::now());
                }
            },
            None => self.opener.release(),
        }
        if consumed || self.settings.camera.locked {
            return;
        }
        match gesture {
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
                let Some(g) = self.gfx.as_ref() else { return };
                // Pixel deltas arrive in physical pixels, so a high density screen would zoom further per notch.
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => (p.y / g.window.scale_factor()) as f32 / PIXELS_PER_LINE,
                };
                self.camera.zoom(lines, [self.cursor.x, self.cursor.y], g.canvas.viewport());
            },
            WindowEvent::Touch(touch) => self.touch(touch, consumed),
            _ => {},
        }
    }
}

impl ApplicationHandler<Event> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.starting {
            return;
        }
        self.starting = true;
        let attrs = Window::default_attributes().with_title(TITLE);
        #[cfg(not(target_arch = "wasm32"))]
        let attrs = placed(attrs.with_inner_size(winit::dpi::LogicalSize::new(900, 900)), &self.placement, el);
        #[cfg(target_arch = "wasm32")]
        let attrs = {
            use winit::platform::web::WindowAttributesExtWebSys;
            // The page blocks only the defaults the viewer needs gone, so browser shortcuts keep working.
            attrs.with_canvas(self.canvas.take()).with_prevent_default(false)
        };
        let window = match el.create_window(attrs).context(CreateWindowSnafu) {
            Ok(window) => Arc::new(window),
            Err(error) => return self.fail(el, error),
        };
        let (instance, settings, sources, proxy) =
            (self.instance.clone(), self.settings.clone(), self.sources.clone(), self.proxy.clone());
        (self.spawn)(Box::pin(async move {
            let gfx = Gfx::create(window, instance, &settings, &sources).await.map(Box::new);
            if proxy.send_event(Event::Started(gfx)).is_err() {
                tracing::warn!("event loop closed before the viewer started");
            }
        }));
    }

    fn user_event(&mut self, el: &ActiveEventLoop, event: Event) {
        match event {
            Event::Assign(path, value) => {
                self.assign(&path, value);
                if let Some(g) = self.gfx.as_ref() {
                    g.window.request_redraw();
                }
            },
            Event::Hold(held) => self.hold(held),
            Event::Conditions(conditions) => self.observe(conditions),
            Event::Started(Ok(gfx)) => {
                gfx.window.request_redraw();
                let mut overlay = Overlay::new(&gfx.window, gfx.canvas.max_texture_side());
                if !self.issues.is_empty() {
                    let n = self.issues.len();
                    // Native settings come from the file; a page's come from its link.
                    let source = if self.store.is_some() { "settings file" } else { "link" };
                    let what = if n == 1 { "1 setting".to_owned() } else { format!("{n} settings") };
                    overlay.toast(format!("{what} in the {source} could not be used; see the menu"), true);
                }
                self.overlay = Some(overlay);
                self.last = Instant::now();
                if let Some(on_window) = self.on_window.take() {
                    on_window(Arc::clone(&gfx.window), Remote(self.proxy.clone()));
                }
                self.gfx = Some(*gfx);
            },
            Event::Started(Err(error)) => self.fail(el, error),
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
        if let WindowEvent::KeyboardInput { event: key, is_synthetic, .. } = &event {
            tracing::debug!(key = ?key.physical_key, state = ?key.state, repeat = key.repeat, synthetic = is_synthetic, "key");
        }
        // Input while paused draws a frame, so a menu it opens shows and then keeps frames coming.
        let input =
            matches!(event, WindowEvent::KeyboardInput { .. } | WindowEvent::MouseInput { .. } | WindowEvent::Touch(_));
        if input
            && self.pacing() == Pacing::Idle
            && let Some(g) = self.gfx.as_ref()
        {
            g.window.request_redraw();
        }
        if let WindowEvent::KeyboardInput { event: key, is_synthetic: false, .. } = &event
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
                let was = self.pacing();
                self.hidden = hidden;
                self.resume_if_woken(was);
            },
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::Focused(focused) => self.focused = focused,
            WindowEvent::RedrawRequested => self.redraw(el),
            // A synthetic press is a key already held as the window gained focus, not a new one;
            // acting on it would run a toggle twice when focus and the press arrive together.
            WindowEvent::KeyboardInput { event, is_synthetic: false, .. }
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

/// `attrs` for a window at `placement`.
#[cfg(not(target_arch = "wasm32"))]
fn placed(
    attrs: winit::window::WindowAttributes,
    placement: &Placement,
    el: &ActiveEventLoop,
) -> winit::window::WindowAttributes {
    #[cfg(target_os = "linux")]
    {
        use winit::platform::x11::{WindowAttributesExtX11, WindowType};
        match placement {
            Placement::Window => attrs,
            Placement::Desktop { monitor } => {
                let attrs = attrs
                    .with_decorations(false)
                    .with_window_level(winit::window::WindowLevel::AlwaysOnBottom)
                    .with_x11_window_type(vec![WindowType::Desktop]);
                match pick_monitor(el, monitor.as_deref()) {
                    Some(m) => {
                        tracing::info!(monitor = ?m.name(), size = ?m.size(), position = ?m.position(), "desktop placement");
                        attrs.with_position(m.position()).with_inner_size(m.size())
                    },
                    None => attrs,
                }
            },
            Placement::Embed(parent) => {
                tracing::info!(parent = format_args!("{parent:#x}"), "embedded placement");
                attrs.with_embed_parent_window(*parent)
            },
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = el;
        if *placement != Placement::Window {
            tracing::warn!(?placement, "only X11 places the viewer on the desktop; opening a window");
        }
        attrs
    }
}

/// The monitor named `wanted`, or at index `wanted`, else the primary one.
#[cfg(target_os = "linux")]
fn pick_monitor(el: &ActiveEventLoop, wanted: Option<&str>) -> Option<winit::monitor::MonitorHandle> {
    let all: Vec<_> = el.available_monitors().collect();
    let names: Vec<_> = all.iter().map(winit::monitor::MonitorHandle::name).collect();
    let found = wanted.and_then(|w| {
        all.iter().find(|m| m.name().as_deref() == Some(w)).or_else(|| w.parse::<usize>().ok().and_then(|i| all.get(i)))
    });
    if let (Some(w), None) = (wanted, found) {
        tracing::warn!(wanted = w, available = ?names, "no such monitor; using the primary one");
    }
    found.cloned().or_else(|| el.primary_monitor()).or_else(|| all.first().cloned())
}

fn app(event_loop: &EventLoop<Event>, launch: Launch, hooks: Hooks, spawn: Box<dyn Fn(LocalFuture)>) -> App {
    #[cfg(not(target_arch = "wasm32"))]
    // `WGPU_BACKEND` and the other wgpu variables pick a backend or relax checks, as under Wine.
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(
        event_loop.owned_display_handle(),
    )));
    #[cfg(target_arch = "wasm32")]
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let now = Instant::now();
    let Launch { settings, sources, debug, store, issues, placement, on_window } = launch;
    App {
        camera: Camera::new(settings.camera),
        view: debug,
        drawn_with: settings.clone(),
        settings,
        sources,
        store,
        issues,
        placement,
        on_window,
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
        held: false,
        conditions: Conditions::default(),
        focused: true,
        touch_hinted: false,
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
        presented: 0,
        announced_fault: None,
        started_at: now,
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

/// Starts the viewer on `canvas` under the browser's event loop and returns at once, with the
/// handle that changes it while it runs.
#[cfg(target_arch = "wasm32")]
pub fn spawn(launch: Launch, canvas: web_sys::HtmlCanvasElement, hooks: Hooks) -> Result<Remote, Error> {
    use winit::platform::web::EventLoopExtWebSys;

    let event_loop = EventLoop::with_user_event().build().context(EventLoopSnafu)?;
    let remote = Remote(event_loop.create_proxy());
    let mut app = app(&event_loop, launch, hooks, Box::new(wasm_bindgen_futures::spawn_local));
    app.canvas = Some(canvas);
    event_loop.spawn_app(app);
    Ok(remote)
}
