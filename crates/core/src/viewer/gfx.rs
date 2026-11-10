use std::sync::Arc;

use snafu::{OptionExt, ResultExt};
use tracing::info;
use winit::dpi::PhysicalSize;
use winit::window::Window;

use super::blit::Blit;
use super::scale::RenderScale;
use super::{CreateSurfaceSnafu, Error, GpuSnafu, NoSurfaceConfigSnafu, PipelinesSnafu, Settings};
use crate::camera::Viewport;
use crate::gpu::{self, Gpu};
use crate::renderer::{FieldView, PassTimes, Renderer, Uniforms};

/// The window and everything that draws into it.
pub(crate) struct Gfx {
    pub(crate) window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Renderer,
    blit: Blit,
    scale: RenderScale,
    /// The window's size; the surface is this scaled down by `scale`.
    physical: PhysicalSize<u32>,
    /// What the renderer's colour texture holds, so a frame that would repeat it (paused with
    /// the camera still) is presented without running the passes again.
    drawn: Option<(Uniforms, Option<FieldView>)>,
}

impl Gfx {
    #[expect(
        clippy::future_not_send,
        reason = "driven on the event loop's thread: blocked on natively, spawned locally on the web"
    )]
    pub(crate) async fn create(
        window: Arc<Window>,
        instance: wgpu::Instance,
        settings: &Settings,
    ) -> Result<Self, Error> {
        let surface = instance.create_surface(window.clone()).context(CreateSurfaceSnafu)?;
        let Gpu { adapter, device, queue } = gpu::request(&instance, Some(&surface)).await.context(GpuSnafu)?;
        let scale = RenderScale::new(settings.budget, settings.field_step);
        let physical = window.inner_size();
        let size = scale.fit(physical);
        let mut config = surface.get_default_config(&adapter, size.width, size.height).context(NoSurfaceConfigSnafu)?;
        config.present_mode = wgpu::PresentMode::AutoVsync;
        surface.configure(&device, &config);

        // GL reports a shader the driver rejects as an internal error, not a validation one.
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let blit = Blit::new(&device, config.format, settings.supersampling);
        let mut renderer = Renderer::new(&device, &queue, &settings.sources);
        renderer.set_temporal(settings.temporal);
        let internal = internal.pop().await;
        if let Some(error) = internal.or(validation.pop().await) {
            return PipelinesSnafu { message: error.to_string() }.fail();
        }
        Ok(Self { window, surface, config, device, queue, renderer, blit, scale, physical, drawn: None })
    }

    /// Pixels the shaders render at, before supersampling.
    pub(crate) fn size(&self) -> PhysicalSize<u32> {
        PhysicalSize::new(self.config.width, self.config.height)
    }

    /// Output pixels per flow field texel this frame.
    pub(crate) fn field_step(&self) -> u32 {
        self.scale.field_step()
    }

    /// The window in physical pixels, which pointer positions are measured in.
    pub(crate) fn viewport(&self) -> Viewport {
        Viewport::new(self.physical.width.max(1), self.physical.height.max(1))
    }

    pub(crate) fn resize(&mut self, physical: PhysicalSize<u32>) {
        self.physical = physical;
        self.configure();
    }

    fn configure(&mut self) {
        let size = self.scale.fit(self.physical);
        if (size.width, size.height) == (self.config.width, self.config.height) {
            return;
        }
        (self.config.width, self.config.height) = (size.width, size.height);
        self.surface.configure(&self.device, &self.config);
    }

    /// Feeds the last frame's interval to the render scale, resizing the surface when it moves.
    pub(crate) fn observe_frame(&mut self, dt: f32) {
        if self.scale.observe(dt) {
            self.configure();
            info!(
                scale = self.scale.scale(),
                field_step = self.scale.field_step(),
                width = self.config.width,
                height = self.config.height,
                "render scale"
            );
        }
    }

    /// Switches blending frames with the ones before them, and redraws even a paused picture.
    pub(crate) fn toggle_temporal(&mut self) {
        let temporal = !self.renderer.temporal();
        self.renderer.set_temporal(temporal);
        self.drawn = None;
        info!(temporal, "temporal antialiasing");
    }

    /// The next surface texture, or `None` when this frame is skipped; a stale surface is reconfigured first.
    pub(crate) fn acquire(&self) -> Option<wgpu::SurfaceTexture> {
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
    pub(crate) fn draw(
        &mut self,
        frame: wgpu::SurfaceTexture,
        uniforms: &Uniforms,
        view: Option<FieldView>,
    ) -> PassTimes {
        let mut enc = self.device.create_command_encoder(&Default::default());
        if self.drawn != Some((*uniforms, view)) || !self.renderer.settled() {
            if self.renderer.render(&self.device, &self.queue, &mut enc, uniforms, view) {
                self.blit.bind(&self.device, self.renderer.color_view());
            }
            self.drawn = Some((*uniforms, view));
        }
        let view = frame.texture.create_view(&Default::default());
        self.blit.draw(&mut enc, &view);
        self.queue.submit([enc.finish()]);
        self.renderer.after_submit();
        self.window.pre_present_notify();
        self.queue.present(frame);
        self.renderer.pass_ms(&self.device, false).unwrap_or_default()
    }
}
