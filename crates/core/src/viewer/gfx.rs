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
use crate::settings::{Settings, limits};
use crate::shaders::Sources;

/// How much wider than the frame the field pass's key texture can grow.
const KEY_HEADROOM: f32 = 1.5;

/// Everything that draws a frame, into whatever texture it is handed.
pub(crate) struct Canvas {
    device: wgpu::Device,
    queue: wgpu::Queue,
    format: wgpu::TextureFormat,
    renderer: Renderer,
    blit: Blit,
    /// The supersampling factor `blit` was built for.
    blit_ss: u32,
    /// The factor the settings ask for, which a large target may not have room for.
    requested_ss: u32,
    egui: egui_wgpu::Renderer,
    scale: RenderScale,
    /// The window's size; frames are this scaled down by `scale`.
    physical: PhysicalSize<u32>,
    /// What the renderer's colour texture holds, so a frame that would repeat it (paused with
    /// the camera still) is presented without running the passes again.
    drawn: Option<(Uniforms, Option<FieldView>)>,
}

impl Canvas {
    pub(crate) fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
        physical: PhysicalSize<u32>,
        settings: &Settings,
        sources: &Sources,
    ) -> Self {
        let mut renderer = Renderer::new(&device, &queue, sources);
        renderer.set_temporal(settings.render.temporal);
        let egui = egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());
        let mut canvas = Self {
            blit: Blit::new(&device, format, 1),
            blit_ss: 1,
            requested_ss: settings.render.supersampling,
            device,
            queue,
            format,
            renderer,
            egui,
            scale: RenderScale::new(settings.render.budget(), settings.render.field_step),
            physical,
            drawn: None,
        };
        canvas.fit_blit();
        canvas
    }

    pub(crate) const fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// The longest frame side the device has room for, leaving the field pass's key, which
    /// spans the view plus margins on both sides, within the device's largest texture.
    fn max_side(&self) -> u32 {
        (self.device.limits().max_texture_dimension_2d as f32 / KEY_HEADROOM) as u32
    }

    /// Pixels the shaders render at, before supersampling: the window scaled by the budget and
    /// kept within the largest texture the device takes.
    pub(crate) fn size(&self) -> PhysicalSize<u32> {
        let size = self.scale.fit(self.physical);
        let over = size.width.max(size.height) as f32 / self.max_side() as f32;
        if over <= 1.0 {
            return size;
        }
        let shrink = |v: u32| ((v as f32 / over).floor() as u32).clamp(1, self.max_side());
        PhysicalSize::new(shrink(size.width), shrink(size.height))
    }

    /// The supersampling factor frames use: the one asked for, as far as the device's texture
    /// size allows and the shaded pixels stay within the most a budget may ask for.
    pub(crate) fn supersampling(&self) -> u32 {
        let size = self.size();
        let side_room = self.max_side() / size.width.max(size.height).max(1);
        let pixels = f64::from(size.width.max(1)) * f64::from(size.height.max(1));
        let pixel_room = (f64::from(*limits::MAX_PIXELS.end()) / pixels).sqrt().floor() as u32;
        self.requested_ss.clamp(1, side_room.min(pixel_room).max(1))
    }

    /// Rebuilds the blit when frames use a factor other than the one it was built for.
    fn fit_blit(&mut self) {
        let ss = self.supersampling();
        if ss != self.blit_ss {
            self.blit = Blit::new(&self.device, self.format, ss);
            self.blit_ss = ss;
            self.drawn = None;
        }
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
        self.fit_blit();
    }

    /// Feeds the last frame's interval to the render scale.
    pub(crate) fn observe_frame(&mut self, dt: f32) {
        if self.scale.observe(dt) {
            self.fit_blit();
            let size = self.size();
            info!(
                scale = self.scale.scale(),
                field_step = self.scale.field_step(),
                width = size.width,
                height = size.height,
                "render scale"
            );
        }
    }

    /// Frame pixels per window pixel.
    pub(crate) fn surface_ratio(&self) -> f32 {
        self.size().width as f32 / self.physical.width.max(1) as f32
    }

    /// The largest texture side the device takes, which bounds egui's font atlas.
    pub(crate) fn max_texture_side(&self) -> u32 {
        self.device.limits().max_texture_dimension_2d
    }

    /// Takes on whatever in `new` differs from `old` and the uniforms do not carry. Only call
    /// it between frames: the next frame may come out a different size.
    pub(crate) fn apply(&mut self, old: &Settings, new: &Settings) {
        let (o, n) = (&old.render, &new.render);
        if o.temporal != n.temporal {
            self.renderer.set_temporal(n.temporal);
            info!(temporal = n.temporal, "temporal antialiasing");
        }
        if (o.budget(), o.field_step) != (n.budget(), n.field_step) {
            self.scale = RenderScale::new(n.budget(), n.field_step);
        }
        self.requested_ss = n.supersampling;
        self.fit_blit();
        self.drawn = None;
    }

    /// The uniforms for a frame of the current size.
    pub(crate) fn uniforms(&self, time: f32, view: [f32; 4], params: crate::params::ParamBlock) -> Uniforms {
        let size = self.size();
        Uniforms::new([size.width, size.height], self.blit_ss, self.field_step(), time, view, params)
    }

    /// Renders a frame onto `target`, which must be `size()` large, and submits it; returns the
    /// pass times of the latest frame read back.
    pub(crate) fn draw(
        &mut self,
        target: &wgpu::TextureView,
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
        self.blit.draw(&mut enc, target);
        let mut buffers = Vec::new();
        let freed = overlay.map(|mut painted| {
            buffers = self.draw_overlay(&mut enc, target, &painted);
            let freed = std::mem::take(&mut painted.textures.free);
            // The uploads are done; a delta dropped with any left over is taken as a lost one.
            painted.textures.clear();
            freed
        });
        self.queue.submit(buffers.into_iter().chain([enc.finish()]));
        for id in freed.iter().flatten() {
            self.egui.free_texture(id);
        }
        self.renderer.after_submit();
        self.renderer.pass_ms(&self.device, false).unwrap_or_default()
    }

    /// Records the overlay over `target`; returns command buffers that must be submitted first.
    fn draw_overlay(
        &mut self,
        enc: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        painted: &Painted,
    ) -> Vec<wgpu::CommandBuffer> {
        let size = self.size();
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [size.width, size.height],
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

/// The window, its surface and the canvas that draws into it.
pub(crate) struct Gfx {
    pub(crate) window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pub(crate) canvas: Canvas,
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
        let physical = window.inner_size();
        let mut config = surface.get_default_config(&adapter, 1, 1).context(NoSurfaceConfigSnafu)?;
        config.present_mode = wgpu::PresentMode::AutoVsync;

        // GL reports a shader the driver rejects as an internal error, not a validation one.
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let canvas = Canvas::new(device, queue, config.format, physical, settings, sources);
        let internal = internal.pop().await;
        if let Some(error) = internal.or(validation.pop().await) {
            return PipelinesSnafu { message: error.to_string() }.fail();
        }
        let size = canvas.size();
        (config.width, config.height) = (size.width, size.height);
        surface.configure(canvas.device(), &config);
        Ok(Self { window, surface, config, canvas })
    }

    /// The next surface texture, or `None` when this frame is skipped. The surface takes on
    /// the canvas's size here, the one point in a frame where no texture of it is held.
    pub(crate) fn acquire(&mut self) -> Option<wgpu::SurfaceTexture> {
        let size = self.canvas.size();
        if (size.width, size.height) != (self.config.width, self.config.height) {
            (self.config.width, self.config.height) = (size.width, size.height);
            self.surface.configure(self.canvas.device(), &self.config);
        }
        match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => Some(t),
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(self.canvas.device(), &self.config);
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
        let target = frame.texture.create_view(&Default::default());
        let passes = self.canvas.draw(&target, uniforms, view, overlay);
        self.window.pre_present_notify();
        self.canvas.queue.present(frame);
        passes
    }
}

#[cfg(test)]
mod tests;
