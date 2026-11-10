use std::sync::Arc;

use snafu::{OptionExt, ResultExt};
use tracing::info;
use winit::dpi::PhysicalSize;
use winit::window::Window;

use super::blit::Blit;
use super::scale::RenderScale;
use super::ui::Painted;
use super::{CreateSurfaceSnafu, Error, GpuSnafu, NoSurfaceConfigSnafu, PipelinesSnafu};
use crate::camera::Viewport;
use crate::gpu::{self, Gpu};
use crate::renderer::{FieldView, PassTimes, Renderer, Uniforms};
use crate::settings::Settings;
use crate::shaders::Sources;

/// The window and everything that draws into it.
pub(crate) struct Gfx {
    pub(crate) window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Renderer,
    blit: Blit,
    egui: egui_wgpu::Renderer,
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
        sources: &Sources,
    ) -> Result<Self, Error> {
        let surface = instance.create_surface(window.clone()).context(CreateSurfaceSnafu)?;
        let Gpu { adapter, device, queue } = gpu::request(&instance, Some(&surface)).await.context(GpuSnafu)?;
        let scale = RenderScale::new(settings.render.budget(), settings.render.field_step);
        let physical = window.inner_size();
        let size = scale.fit(physical);
        let mut config = surface.get_default_config(&adapter, size.width, size.height).context(NoSurfaceConfigSnafu)?;
        config.present_mode = wgpu::PresentMode::AutoVsync;
        surface.configure(&device, &config);

        // GL reports a shader the driver rejects as an internal error, not a validation one.
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let blit = Blit::new(&device, config.format, settings.render.supersampling);
        let mut renderer = Renderer::new(&device, &queue, sources);
        renderer.set_temporal(settings.render.temporal);
        let egui = egui_wgpu::Renderer::new(&device, config.format, egui_wgpu::RendererOptions::default());
        let internal = internal.pop().await;
        if let Some(error) = internal.or(validation.pop().await) {
            return PipelinesSnafu { message: error.to_string() }.fail();
        }
        Ok(Self { window, surface, config, device, queue, renderer, blit, egui, scale, physical, drawn: None })
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

    /// Surface pixels per window pixel.
    pub(crate) fn surface_ratio(&self) -> f32 {
        self.config.width as f32 / self.physical.width.max(1) as f32
    }

    /// The largest texture side the device takes, which bounds egui's font atlas.
    pub(crate) fn max_texture_side(&self) -> u32 {
        self.device.limits().max_texture_dimension_2d
    }

    /// Applies what changed from `old` to `new` that the uniforms do not carry.
    pub(crate) fn apply(&mut self, old: &Settings, new: &Settings) {
        let (o, n) = (&old.render, &new.render);
        if o.temporal != n.temporal {
            self.renderer.set_temporal(n.temporal);
            info!(temporal = n.temporal, "temporal antialiasing");
        }
        if (o.budget(), o.field_step) != (n.budget(), n.field_step) {
            self.scale = RenderScale::new(n.budget(), n.field_step);
            self.configure();
        }
        if o.supersampling != n.supersampling {
            self.blit = Blit::new(&self.device, self.config.format, n.supersampling);
        }
        self.drawn = None;
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
        overlay: Option<Painted>,
    ) -> PassTimes {
        let mut enc = self.device.create_command_encoder(&Default::default());
        if self.drawn != Some((*uniforms, view)) || !self.renderer.settled() {
            let replaced = self.renderer.render(&self.device, &self.queue, &mut enc, uniforms, view);
            // A blit rebuilt for a new supersampling factor starts unbound.
            if replaced || self.drawn.is_none() {
                self.blit.bind(&self.device, self.renderer.color_view());
            }
            self.drawn = Some((*uniforms, view));
        }
        let view = frame.texture.create_view(&Default::default());
        self.blit.draw(&mut enc, &view);
        let mut buffers = Vec::new();
        let freed = overlay.map(|mut painted| {
            buffers = self.draw_overlay(&mut enc, &view, &painted);
            std::mem::take(&mut painted.textures.free)
        });
        self.queue.submit(buffers.into_iter().chain([enc.finish()]));
        for id in freed.iter().flatten() {
            self.egui.free_texture(id);
        }
        self.renderer.after_submit();
        self.window.pre_present_notify();
        self.queue.present(frame);
        self.renderer.pass_ms(&self.device, false).unwrap_or_default()
    }

    /// Records the overlay over `target`; returns command buffers that must be submitted first.
    fn draw_overlay(
        &mut self,
        enc: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        painted: &Painted,
    ) -> Vec<wgpu::CommandBuffer> {
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.config.width, self.config.height],
            pixels_per_point: painted.pixels_per_point,
        };
        for (id, deltas) in &painted.textures.set {
            for delta in deltas {
                self.egui.update_texture(&self.device, &self.queue, *id, delta);
            }
        }
        let buffers = self.egui.update_buffers(&self.device, &self.queue, enc, &painted.primitives, &screen);
        let mut pass = enc
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("overlay"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            })
            .forget_lifetime();
        self.egui.render(&mut pass, &painted.primitives, &screen);
        buffers
    }
}
