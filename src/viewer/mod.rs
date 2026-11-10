mod blit;
mod camera;
mod gfx;
mod title;

use std::time::Instant;

use camera::{Camera, PIXELS_PER_LINE};
use gfx::Gfx;
use snafu::{ResultExt, Snafu};
use title::FrameCounter;
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::error::{EventLoopError, OsError};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::WindowId;

use crate::cli::Common;
use crate::gpu;
use crate::renderer::{self, PassTimes, Uniforms};

#[derive(Debug, Snafu)]
pub(crate) enum Error {
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
    #[snafu(display("could not set up the renderer"))]
    Renderer { source: renderer::Error },
}

struct App {
    common: Common,
    instance: wgpu::Instance,
    gfx: Option<Gfx>,
    camera: Camera,
    paused: bool,
    time: f32,
    last: Instant,
    cursor: PhysicalPosition<f64>,
    dragging: bool,
    title: FrameCounter,
    failure: Option<Error>,
}

fn frame_uniforms(common: &Common, camera: &Camera, time: f32, size: PhysicalSize<u32>) -> Uniforms {
    let ss = common.supersampling();
    Uniforms {
        resolution: [(size.width * ss) as f32, (size.height * ss) as f32],
        time,
        field_scale: ss as f32,
        view: camera.view(camera::Viewport::new(size.width, size.height)).to_array(),
        params: common.param_array(),
    }
}

impl App {
    fn advance_clock(&mut self, now: Instant) {
        if !self.paused {
            self.time += (now - self.last).as_secs_f32() * self.common.speed;
        }
        self.last = now;
    }

    fn update_title(&mut self, now: Instant, passes: PassTimes) {
        if let Some(title) = self.title.tick(now, passes, self.time, self.paused)
            && let Some(g) = self.gfx.as_ref()
        {
            g.set_title(&title);
        }
    }

    fn redraw(&mut self) {
        let now = Instant::now();
        self.advance_clock(now);
        let Some(g) = self.gfx.as_mut() else { return };
        let uniforms = frame_uniforms(&self.common, &self.camera, self.time, g.size());
        let Some(frame) = g.acquire() else { return };
        let passes = g.draw(frame, &uniforms);
        self.update_title(now, passes);
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        match Gfx::create(el, &self.instance, &self.common) {
            Ok(gfx) => {
                self.last = Instant::now();
                self.gfx = Some(gfx);
            },
            Err(error) => {
                self.failure = Some(error);
                el.exit();
            },
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
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed && !event.repeat => {
                match event.logical_key.as_ref() {
                    Key::Named(NamedKey::Space) => self.paused = !self.paused,
                    Key::Named(NamedKey::Escape) => el.exit(),
                    Key::Character("r") => self.camera.reset(),
                    _ => {},
                }
            },
            _ => {},
        }
    }

    fn about_to_wait(&mut self, _el: &ActiveEventLoop) {
        if let Some(g) = self.gfx.as_ref() {
            g.request_redraw();
        }
    }
}

/// Opens the interactive viewer and runs it until the window closes.
pub(crate) fn run(common: Common) -> Result<(), Error> {
    let event_loop = EventLoop::new().context(EventLoopSnafu)?;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(Box::new(
        event_loop.owned_display_handle(),
    )));
    let now = Instant::now();
    let mut app = App {
        common,
        instance,
        gfx: None,
        camera: Camera::default(),
        paused: false,
        time: 0.0,
        last: now,
        cursor: PhysicalPosition::new(0.0, 0.0),
        dragging: false,
        title: FrameCounter::new(now),
        failure: None,
    };
    event_loop.run_app(&mut app).context(RunEventLoopSnafu)?;
    app.failure.map_or(Ok(()), Err)
}
