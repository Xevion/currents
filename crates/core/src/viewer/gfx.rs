use std::sync::{Arc, Mutex, PoisonError};

use snafu::{OptionExt, ResultExt};
use tracing::{debug, error, info, warn};
use winit::dpi::PhysicalSize;
use winit::window::Window;

use super::blit::Blit;
use super::scale::RenderScale;
use super::ui::Painted;
use super::{CreateSurfaceSnafu, Error, GpuSnafu, NoSurfaceConfigSnafu};
use crate::camera::Viewport;
use crate::gpu::{self, Gpu};
use crate::params::ParamBlock;
use crate::renderer::{FieldView, PassTimes, Renderer, Uniforms};
use crate::settings::{Settings, limits};
use crate::shaders::Sources;

/// How much wider than the frame the field pass's key texture can grow.
const KEY_HEADROOM: f32 = 1.5;

/// The first error the device reported since the canvas last tried afresh.
type Fault = Arc<Mutex<Option<String>>>;

fn record(fault: &Fault, message: String) {
    let mut slot = fault.lock().unwrap_or_else(PoisonError::into_inner);
    if slot.is_none() {
        error!(error = %message, "graphics error; drawing black under the menu until settings change");
        *slot = Some(message);
    }
}

/// Everything that draws a frame, into whatever texture it is handed.
///
/// The picture is shaded at `size()`, which a pixel budget can hold under the window's size,
/// and scaled up over the whole target. A device error stops the shading, not the frame: the
/// target is cleared black and the overlay still drawn, so the menu stays usable.
pub(crate) struct Canvas {
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// The format of the targets it draws onto.
    #[cfg_attr(not(test), expect(dead_code, reason = "tests make their stand-in surfaces in it"))]
    format: wgpu::TextureFormat,
    renderer: Renderer,
    blit: Blit,
    egui: egui_wgpu::Renderer,
    requested_ss: u32,
    scale: RenderScale,
    /// The window's size, which targets are.
    pub(crate) physical: PhysicalSize<u32>,
    /// What the renderer's colour texture holds, so a frame that would repeat it (paused with
    /// the camera still) is presented without running the passes again.
    drawn: Option<(Uniforms, Option<FieldView>)>,
    fault: Fault,
}

impl Canvas {
    /// Builds the canvas; a pipeline the device rejects leaves it faulted rather than failing.
    #[expect(
        clippy::future_not_send,
        reason = "driven on the event loop's thread: blocked on natively, spawned locally on the web"
    )]
    pub(crate) async fn create(
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
        physical: PhysicalSize<u32>,
        settings: &Settings,
        sources: &Sources,
    ) -> Self {
        let fault: Fault = Arc::default();
        // GL reports a shader the driver rejects as an internal error, not a validation one.
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let mut renderer = Renderer::new(&device, &queue, sources);
        renderer.set_temporal(settings.render.temporal);
        let blit = Blit::new(&device, format);
        let egui = egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());
        let internal = internal.pop().await;
        if let Some(e) = internal.or(validation.pop().await) {
            record(&fault, format!("the shaders did not compile on this device: {e}"));
        }
        let handler = fault.clone();
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| record(&handler, e.to_string())));
        let lost = fault.clone();
        device.set_device_lost_callback(move |reason, message| {
            record(&lost, format!("the graphics device was lost ({reason:?}): {message}"));
        });
        let canvas = Self {
            device,
            queue,
            format,
            renderer,
            blit,
            egui,
            requested_ss: settings.render.supersampling,
            scale: RenderScale::new(settings.render.budget(), settings.render.field_step),
            physical,
            drawn: None,
            fault,
        };
        let size = canvas.size();
        info!(
            window_width = physical.width,
            window_height = physical.height,
            width = size.width,
            height = size.height,
            supersampling = canvas.supersampling(),
            field_step = canvas.field_step(),
            ?format,
            "canvas ready"
        );
        canvas
    }

    pub(crate) const fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// The error that stopped shading, if one has.
    pub(crate) fn fault(&self) -> Option<String> {
        self.fault.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Stops shading as a device error would.
    #[cfg(test)]
    pub(crate) fn set_fault(&self, message: &str) {
        record(&self.fault, message.to_owned());
    }

    /// The longest frame side the device has room for, leaving the field pass's key, which
    /// spans the view plus margins on both sides, within the device's largest texture.
    pub(crate) fn max_side(&self) -> u32 {
        (self.device.limits().max_texture_dimension_2d as f32 / KEY_HEADROOM) as u32
    }

    /// The largest texture side the device takes.
    pub(crate) fn max_texture_side(&self) -> u32 {
        self.device.limits().max_texture_dimension_2d
    }

    /// The size targets are: the window, within the largest texture the device takes.
    pub(crate) fn target_size(&self) -> PhysicalSize<u32> {
        let max = self.max_texture_side();
        PhysicalSize::new(self.physical.width.clamp(1, max), self.physical.height.clamp(1, max))
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

    /// Output pixels per flow field texel this frame.
    pub(crate) fn field_step(&self) -> u32 {
        self.scale.field_step()
    }

    /// The window in physical pixels, which pointer positions are measured in.
    pub(crate) fn viewport(&self) -> Viewport {
        Viewport::new(self.physical.width.max(1), self.physical.height.max(1))
    }

    pub(crate) fn resize(&mut self, physical: PhysicalSize<u32>) {
        if physical != self.physical {
            debug!(width = physical.width, height = physical.height, "window resized");
        }
        self.physical = physical;
    }

    /// Feeds the last frame's interval and pass times to the render scale; `watched` is false
    /// while the page is hidden or unfocused, when the browser may slow frames on its own.
    pub(crate) fn observe_frame(&mut self, dt: f32, passes: PassTimes, watched: bool) {
        if !watched {
            self.scale.pause();
            return;
        }
        let gpu = passes.field + passes.shade;
        if self.scale.observe(dt, (gpu > 0.0).then_some(gpu / 1000.0)) {
            let size = self.size();
            info!(
                scale = self.scale.scale(),
                field_step = self.scale.field_step(),
                width = size.width,
                height = size.height,
                "render scale adapted to the frame rate"
            );
        }
    }

    /// Takes on whatever in `new` differs from `old` and the uniforms do not carry, and tries
    /// shading again after a fault. Only call it between frames.
    pub(crate) fn apply(&mut self, old: &Settings, new: &Settings) {
        let (o, n) = (&old.render, &new.render);
        if o.temporal != n.temporal {
            self.renderer.set_temporal(n.temporal);
        }
        let resized = (o.budget(), o.field_step) != (n.budget(), n.field_step);
        if resized {
            self.scale = RenderScale::new(n.budget(), n.field_step);
        }
        self.requested_ss = n.supersampling;
        if resized || o.supersampling != n.supersampling {
            let size = self.size();
            info!(
                width = size.width,
                height = size.height,
                supersampling = self.supersampling(),
                field_step = self.field_step(),
                budget = ?n.budget(),
                "render size"
            );
        }
        let cleared = self.fault.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(fault) = cleared {
            info!(%fault, "settings changed; shading again");
        }
        self.drawn = None;
    }

    /// The uniforms for a frame of the current size.
    pub(crate) fn uniforms(&self, time: f64, view: [f32; 4], params: ParamBlock) -> Uniforms {
        let size = self.size();
        Uniforms::new([size.width, size.height], self.supersampling(), self.field_step(), time, view, params)
    }

    /// Draws a frame over the whole of `target` and submits it; returns the pass times of the
    /// latest frame read back.
    pub(crate) fn draw(
        &mut self,
        target: &wgpu::Texture,
        uniforms: &Uniforms,
        view: Option<FieldView>,
        overlay: Option<Painted>,
    ) -> PassTimes {
        let target_view = target.create_view(&Default::default());
        let mut enc = self.device.create_command_encoder(&Default::default());
        let faulted = self.fault().is_some();
        if faulted {
            self.drawn = None;
            clear(&mut enc, &target_view);
        } else {
            if self.drawn != Some((*uniforms, view)) || !self.renderer.settled() {
                if self.renderer.render(&self.device, &self.queue, &mut enc, uniforms, view) || self.drawn.is_none() {
                    self.blit.bind(&self.device, self.renderer.color_view());
                }
                self.drawn = Some((*uniforms, view));
            }
            self.blit.draw(&mut enc, &target_view);
        }
        let mut buffers = Vec::new();
        let freed = overlay.map(|mut painted| {
            buffers = self.draw_overlay(&mut enc, target, &target_view, &painted);
            let freed = std::mem::take(&mut painted.textures.free);
            // The uploads are done; a delta dropped with any left over is taken as a lost one.
            painted.textures.clear();
            freed
        });
        self.queue.submit(buffers.into_iter().chain([enc.finish()]));
        for id in freed.iter().flatten() {
            self.egui.free_texture(id);
        }
        if faulted {
            return PassTimes::default();
        }
        self.renderer.after_submit();
        self.renderer.pass_ms(&self.device, false).unwrap_or_default()
    }

    /// Records the overlay over `target`; returns command buffers that must be submitted first.
    fn draw_overlay(
        &mut self,
        enc: &mut wgpu::CommandEncoder,
        target: &wgpu::Texture,
        view: &wgpu::TextureView,
        painted: &Painted,
    ) -> Vec<wgpu::CommandBuffer> {
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [target.width(), target.height()],
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
                    view,
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

fn clear(enc: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
    enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("black"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
        })],
        ..Default::default()
    });
}

/// The format the surface is configured in: one without sRGB encoding where the surface
/// offers it, since the picture and egui both write display-ready values.
fn surface_format(formats: &[wgpu::TextureFormat]) -> Option<wgpu::TextureFormat> {
    formats.iter().copied().find(|f| !f.is_srgb()).or_else(|| formats.first().copied())
}

/// Frames in a row that may fail to come before it is worth a warning.
const FAILED_WARN_AFTER: u32 = 3;

/// The window, its surface and the canvas that draws into it.
pub(crate) struct Gfx {
    pub(crate) window: Arc<Window>,
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pub(crate) canvas: Canvas,
    /// Frames in a row the surface has not given; reset by the next it does.
    pub(crate) failed: u32,
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
        let caps = surface.get_capabilities(&adapter);
        let mut config = surface.get_default_config(&adapter, 1, 1).context(NoSurfaceConfigSnafu)?;
        config.format = surface_format(&caps.formats).unwrap_or(config.format);
        config.present_mode = wgpu::PresentMode::AutoVsync;
        let canvas = Canvas::create(device, queue, config.format, window.inner_size(), settings, sources).await;
        let size = canvas.target_size();
        (config.width, config.height) = (size.width, size.height);
        surface.configure(canvas.device(), &config);
        info!(
            width = config.width,
            height = config.height,
            format = ?config.format,
            present_mode = ?config.present_mode,
            offered = ?caps.formats,
            "surface ready"
        );
        Ok(Self { window, instance, surface, config, canvas, failed: 0 })
    }

    /// Brings the surface to the window's size. A surface the window's size is the only kind
    /// every platform presents; a smaller picture is scaled up onto it instead.
    fn fit_surface(&mut self, physical: PhysicalSize<u32>) {
        self.canvas.resize(physical);
        let size = self.canvas.target_size();
        if (size.width, size.height) != (self.config.width, self.config.height) {
            (self.config.width, self.config.height) = (size.width, size.height);
            self.surface.configure(self.canvas.device(), &self.config);
            debug!(width = size.width, height = size.height, "surface resized");
        }
    }

    /// The next surface texture, or `None` when this frame is skipped. A surface that has gone
    /// out of date is refitted to the window as it is now and asked again once.
    pub(crate) fn acquire(&mut self) -> Option<wgpu::SurfaceTexture> {
        let physical = self.canvas.physical;
        self.fit_surface(physical);
        for attempt in 0..2 {
            match self.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(t) => {
                    self.recovered();
                    return Some(t);
                },
                wgpu::CurrentSurfaceTexture::Suboptimal(t) => {
                    debug!("surface suboptimal; refitting after this frame");
                    self.config.width = 0;
                    self.recovered();
                    return Some(t);
                },
                wgpu::CurrentSurfaceTexture::Outdated => {
                    debug!(attempt, "surface out of date; refitting to the window");
                    self.config.width = 0;
                    let physical = self.window.inner_size();
                    self.fit_surface(physical);
                },
                wgpu::CurrentSurfaceTexture::Lost => {
                    warn!(attempt, "surface lost; making a new one");
                    match self.instance.create_surface(self.window.clone()) {
                        Ok(surface) => {
                            self.surface = surface;
                            self.surface.configure(self.canvas.device(), &self.config);
                        },
                        Err(e) => error!(error = %e, "could not make a new surface"),
                    }
                },
                wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => break,
                wgpu::CurrentSurfaceTexture::Validation => {
                    error!("the surface reported a validation error");
                    break;
                },
            }
        }
        self.failed += 1;
        if self.failed == FAILED_WARN_AFTER || self.failed.is_power_of_two() && self.failed > FAILED_WARN_AFTER {
            warn!(
                frames = self.failed,
                width = self.config.width,
                height = self.config.height,
                "the surface has not given a frame"
            );
        }
        None
    }

    fn recovered(&mut self) {
        if self.failed >= FAILED_WARN_AFTER {
            info!(frames = self.failed, "the surface gives frames again");
        }
        self.failed = 0;
    }

    /// Renders a frame and presents it on `frame`; returns the pass times of the latest frame read back.
    pub(crate) fn draw(
        &mut self,
        frame: wgpu::SurfaceTexture,
        uniforms: &Uniforms,
        view: Option<FieldView>,
        overlay: Option<Painted>,
    ) -> PassTimes {
        let passes = self.canvas.draw(&frame.texture, uniforms, view, overlay);
        self.window.pre_present_notify();
        self.canvas.queue.present(frame);
        passes
    }
}

#[cfg(test)]
mod tests;
