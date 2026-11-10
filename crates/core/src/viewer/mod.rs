mod blit;
mod gfx;
mod scale;
mod title;
mod touch;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use gfx::Gfx;
pub use scale::Budget;
use snafu::{ResultExt, Snafu};
use title::FrameCounter;
use touch::{Gesture, Touches};
use web_time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::error::{EventLoopError, OsError};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, Touch, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use crate::camera::{Camera, PIXELS_PER_LINE};
use crate::gpu;
use crate::params::ParamBlock;
use crate::renderer::{FieldView, PassTimes, Uniforms};
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

/// What the viewer renders and how.
#[derive(Clone)]
pub struct Settings {
    pub sources: Sources,
    /// Flow speed multiplier.
    pub speed: f32,
    /// Supersampling factor per axis, at least 1.
    pub supersampling: u32,
    pub params: ParamBlock,
    /// Pixel budget for the surface; `None` renders every window pixel.
    pub budget: Option<Budget>,
    /// The field view to start in instead of the shaded picture; D cycles through them.
    pub debug: Option<FieldView>,
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

struct App {
    settings: Settings,
    instance: wgpu::Instance,
    spawn: Box<dyn Fn(LocalFuture)>,
    proxy: EventLoopProxy<Started>,
    #[cfg(target_arch = "wasm32")]
    canvas: Option<web_sys::HtmlCanvasElement>,
    starting: bool,
    gfx: Option<Gfx>,
    hooks: Hooks,
    camera: Camera,
    view: Option<FieldView>,
    paused: bool,
    time: f32,
    last: Instant,
    cursor: PhysicalPosition<f64>,
    dragging: bool,
    touches: Touches,
    title: FrameCounter,
    failure: Option<Error>,
}

fn frame_uniforms(settings: &Settings, camera: &Camera, time: f32, gfx: &Gfx) -> Uniforms {
    let ss = settings.supersampling;
    let size: PhysicalSize<u32> = gfx.size();
    Uniforms {
        resolution: [(size.width * ss) as f32, (size.height * ss) as f32],
        time,
        field_scale: ss as f32,
        view: camera.view(gfx.viewport()).to_array(),
        params: settings.params,
    }
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
            self.time += dt * self.settings.speed;
        }
        self.last = now;
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

    fn redraw(&mut self) {
        let now = Instant::now();
        let dt = self.advance_clock(now);
        let Some(g) = self.gfx.as_mut() else { return };
        g.observe_frame(dt);
        let uniforms = frame_uniforms(&self.settings, &self.camera, self.time, g);
        if let Some(frame) = g.acquire() {
            let passes = g.draw(frame, &uniforms, self.view);
            self.update_title(now, passes);
            if let Some(on_ready) = self.hooks.on_ready.take() {
                on_ready();
            }
        }
        if let Some(g) = self.gfx.as_ref() {
            g.window.request_redraw();
        }
    }

    fn touch(&mut self, touch: Touch) {
        let Some(g) = self.gfx.as_ref() else { return };
        let viewport = g.viewport();
        match self.touches.event(touch.phase, touch.id, [touch.location.x, touch.location.y]) {
            Some(Gesture::Pan(delta)) => self.camera.pan(delta, viewport),
            Some(Gesture::Pinch { centre, pan, factor }) => {
                self.camera.pan(pan, viewport);
                self.camera.scale_extent(factor, centre, viewport);
            },
            None => {},
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
        let (instance, settings, proxy) = (self.instance.clone(), self.settings.clone(), self.proxy.clone());
        (self.spawn)(Box::pin(async move {
            let gfx = Gfx::create(window, instance, &settings).await.map(Box::new);
            if proxy.send_event(Started(gfx)).is_err() {
                tracing::warn!("event loop closed before the viewer started");
            }
        }));
    }

    fn user_event(&mut self, el: &ActiveEventLoop, event: Started) {
        match event.0 {
            Ok(gfx) => {
                gfx.window.request_redraw();
                self.last = Instant::now();
                self.gfx = Some(*gfx);
            },
            Err(error) => self.fail(el, error),
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(size) => {
                if let Some(g) = self.gfx.as_mut() {
                    g.resize(size);
                }
            },
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::CursorMoved { position, .. } => {
                if self.dragging
                    && let Some(g) = self.gfx.as_ref()
                {
                    self.camera.pan([position.x - self.cursor.x, position.y - self.cursor.y], g.viewport());
                }
                self.cursor = position;
            },
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                self.dragging = state == ElementState::Pressed;
            },
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / PIXELS_PER_LINE,
                };
                if let Some(g) = self.gfx.as_ref() {
                    self.camera.zoom(lines, [self.cursor.x, self.cursor.y], g.viewport());
                }
            },
            WindowEvent::Touch(touch) => self.touch(touch),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed && !event.repeat => {
                match event.logical_key.as_ref() {
                    Key::Named(NamedKey::Space) => self.paused = !self.paused,
                    Key::Named(NamedKey::Escape) if cfg!(not(target_arch = "wasm32")) => el.exit(),
                    Key::Character("r") => self.camera.reset(),
                    Key::Character("d") => self.view = FieldView::cycle(self.view),
                    _ => {},
                }
            },
            _ => {},
        }
    }
}

fn app(event_loop: &EventLoop<Started>, settings: Settings, hooks: Hooks, spawn: Box<dyn Fn(LocalFuture)>) -> App {
    #[cfg(not(target_arch = "wasm32"))]
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(Box::new(
        event_loop.owned_display_handle(),
    )));
    #[cfg(target_arch = "wasm32")]
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let now = Instant::now();
    App {
        view: settings.debug,
        settings,
        instance,
        spawn,
        proxy: event_loop.create_proxy(),
        #[cfg(target_arch = "wasm32")]
        canvas: None,
        starting: false,
        gfx: None,
        hooks,
        camera: Camera::default(),
        paused: false,
        time: 0.0,
        last: now,
        cursor: PhysicalPosition::new(0.0, 0.0),
        dragging: false,
        touches: Touches::default(),
        title: FrameCounter::new(now),
        failure: None,
    }
}

/// Opens the interactive viewer and runs it until the window closes.
#[cfg(not(target_arch = "wasm32"))]
pub fn run(settings: Settings) -> Result<(), Error> {
    let event_loop = EventLoop::with_user_event().build().context(EventLoopSnafu)?;
    let mut app = app(&event_loop, settings, Hooks::default(), Box::new(pollster::block_on));
    event_loop.run_app(&mut app).context(RunEventLoopSnafu)?;
    app.failure.map_or(Ok(()), Err)
}

/// Starts the viewer on `canvas` under the browser's event loop and returns at once.
#[cfg(target_arch = "wasm32")]
pub fn spawn(settings: Settings, canvas: web_sys::HtmlCanvasElement, hooks: Hooks) -> Result<(), Error> {
    use winit::platform::web::EventLoopExtWebSys;

    let event_loop = EventLoop::with_user_event().build().context(EventLoopSnafu)?;
    let mut app = app(&event_loop, settings, hooks, Box::new(wasm_bindgen_futures::spawn_local));
    app.canvas = Some(canvas);
    event_loop.spawn_app(app);
    Ok(())
}
