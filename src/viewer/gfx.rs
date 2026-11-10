use std::sync::Arc;

use snafu::{OptionExt, ResultExt};
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

use super::blit::Blit;
use super::camera::Viewport;
use super::{CreateSurfaceSnafu, CreateWindowSnafu, Error, GpuSnafu, NoSurfaceConfigSnafu, RendererSnafu};
use crate::cli::Common;
use crate::gpu::{self, Gpu};
use crate::renderer::{PassTimes, Renderer, Uniforms};

/// The window and everything that draws into it.
pub(super) struct Gfx {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Renderer,
    blit: Blit,
}

impl Gfx {
    pub(super) fn create(el: &ActiveEventLoop, instance: &wgpu::Instance, common: &Common) -> Result<Self, Error> {
        let attrs = Window::default_attributes().with_title("currents").with_inner_size(LogicalSize::new(900, 900));
        let window = Arc::new(el.create_window(attrs).context(CreateWindowSnafu)?);
        let surface = instance.create_surface(window.clone()).context(CreateSurfaceSnafu)?;
        let Gpu { adapter, device, queue } = gpu::request(instance, Some(&surface)).context(GpuSnafu)?;
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .context(NoSurfaceConfigSnafu)?;
        config.present_mode = wgpu::PresentMode::AutoVsync;
        surface.configure(&device, &config);

        let blit = Blit::new(&device, config.format, common.supersampling());
        let renderer = Renderer::new(&device, &queue, &common.shaders).context(RendererSnafu)?;
        Ok(Self { window, surface, config, device, queue, renderer, blit })
    }

    pub(super) fn size(&self) -> PhysicalSize<u32> {
        PhysicalSize::new(self.config.width, self.config.height)
    }

    pub(super) fn viewport(&self) -> Viewport {
        Viewport::new(self.config.width, self.config.height)
    }

    pub(super) fn resize(&mut self, size: PhysicalSize<u32>) {
        self.config.width = size.width.max(1);
        self.config.height = size.height.max(1);
        self.surface.configure(&self.device, &self.config);
    }

    pub(super) fn request_redraw(&self) {
        self.window.request_redraw();
    }

    pub(super) fn set_title(&self, title: &str) {
        self.window.set_title(title);
    }

    /// The next surface texture, or `None` when this frame is skipped; a stale surface is reconfigured first.
    pub(super) fn acquire(&self) -> Option<wgpu::SurfaceTexture> {
        match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => Some(t),
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                None
            },
            _ => None,
        }
    }

    /// Renders a frame and presents it on `frame`; returns the pass times of the latest frame read back.
    pub(super) fn draw(&mut self, frame: wgpu::SurfaceTexture, uniforms: &Uniforms) -> PassTimes {
        let mut enc = self.device.create_command_encoder(&Default::default());
        if self.renderer.render(&self.device, &self.queue, &mut enc, uniforms) {
            self.blit.bind(&self.device, self.renderer.color_view());
        }
        let view = frame.texture.create_view(&Default::default());
        self.blit.draw(&mut enc, &view);
        self.queue.submit([enc.finish()]);
        self.renderer.after_submit();
        self.queue.present(frame);
        self.renderer.pass_ms(&self.device, false).unwrap_or_default()
    }
}
