use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use crate::Common;
use crate::gpu::{self, Gpu};
use crate::renderer::{PassTimes, Renderer, Uniforms, draw_fullscreen};

const BLIT: &str = r#"
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let p = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4f(p * 2.0 - 1.0, 0.0, 1.0);
}
@fragment
fn fs_main(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let dim = vec2f(textureDimensions(src));
    let c = textureSampleLevel(src, smp, pos.xy * SCALE / dim, 0.0).rgb;
    if (TO_LINEAR) {
        return vec4f(pow(c, vec3f(2.2)), 1.0);
    }
    return vec4f(c, 1.0);
}
"#;

struct Gfx {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Renderer,
    blit: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
}

struct App {
    common: Common,
    instance: wgpu::Instance,
    gfx: Option<Gfx>,
    centre: [f32; 2],
    half_height: f32,
    paused: bool,
    time: f32,
    last: Instant,
    cursor: PhysicalPosition<f64>,
    dragging: bool,
    frames: u32,
    fps_at: Instant,
}

impl App {
    fn half(&self, g: &Gfx) -> [f32; 2] {
        [self.half_height * g.config.width as f32 / g.config.height as f32, self.half_height]
    }

    // Cursor position in -1..1 across the window.
    fn cursor_ndc(&self, g: &Gfx) -> [f32; 2] {
        [
            self.cursor.x as f32 / g.config.width as f32 * 2.0 - 1.0,
            self.cursor.y as f32 / g.config.height as f32 * 2.0 - 1.0,
        ]
    }

    fn redraw(&mut self) {
        let now = Instant::now();
        if !self.paused {
            self.time += (now - self.last).as_secs_f32() * self.common.speed;
        }
        self.last = now;
        let Some(g) = self.gfx.as_mut() else { return };
        let ss = self.common.ss.max(1);
        let half = [self.half_height * g.config.width as f32 / g.config.height as f32, self.half_height];
        let uniforms = Uniforms {
            resolution: [(g.config.width * ss) as f32, (g.config.height * ss) as f32],
            time: self.time,
            field_scale: ss as f32,
            view: [
                self.centre[0] - half[0],
                self.centre[1] - half[1],
                self.centre[0] + half[0],
                self.centre[1] + half[1],
            ],
            params: self.common.param_array(),
        };
        let frame = match g.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                g.surface.configure(&g.device, &g.config);
                return;
            },
            _ => return,
        };
        let mut enc = g.device.create_command_encoder(&Default::default());
        g.renderer.render(&g.device, &g.queue, &mut enc, &uniforms);
        let bind = g.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &g.blit.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(g.renderer.color_view()),
                },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&g.sampler) },
            ],
        });
        let view = frame.texture.create_view(&Default::default());
        draw_fullscreen(&mut enc, &view, &g.blit, &bind, None);
        g.queue.submit([enc.finish()]);
        g.renderer.after_submit();
        g.queue.present(frame);
        let pass = g.renderer.pass_ms(&g.device, false);

        self.frames += 1;
        let dt = (now - self.fps_at).as_secs_f32();
        if dt > 0.5 {
            let state = if self.paused { "paused" } else { "running" };
            let PassTimes { field: f, shade: s } = pass.unwrap_or_default();
            g.window.set_title(&format!(
                "currents | {:.0} fps | field {f:.1}ms shade {s:.1}ms | t={:.1} | {state}",
                self.frames as f32 / dt,
                self.time
            ));
            self.frames = 0;
            self.fps_at = now;
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        let attrs = Window::default_attributes().with_title("currents").with_inner_size(LogicalSize::new(900, 900));
        let window = Arc::new(el.create_window(attrs).expect("window"));
        let surface = self.instance.create_surface(window.clone()).expect("surface");
        let Gpu { adapter, device, queue } = gpu::request(&self.instance, Some(&surface));
        let size = window.inner_size();
        let mut config =
            surface.get_default_config(&adapter, size.width.max(1), size.height.max(1)).expect("surface config");
        config.present_mode = wgpu::PresentMode::AutoVsync;
        surface.configure(&device, &config);

        let src = BLIT
            .replace("SCALE", &format!("{:.1}", self.common.ss.max(1) as f32))
            .replace("TO_LINEAR", if config.format.is_srgb() { "true" } else { "false" });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blit"),
            source: wgpu::ShaderSource::Wgsl(src.into()),
        });
        let blit = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(config.format.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let renderer = Renderer::new(&device, &queue, &self.common.shaders);
        self.last = Instant::now();
        self.gfx = Some(Gfx { window, surface, config, device, queue, renderer, blit, sampler });
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(size) => {
                if let Some(g) = self.gfx.as_mut() {
                    g.config.width = size.width.max(1);
                    g.config.height = size.height.max(1);
                    g.surface.configure(&g.device, &g.config);
                }
            },
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::CursorMoved { position, .. } => {
                if self.dragging
                    && let Some(g) = self.gfx.as_ref()
                {
                    let half = self.half(g);
                    self.centre[0] -= (position.x - self.cursor.x) as f32 / g.config.width as f32 * 2.0 * half[0];
                    self.centre[1] -= (position.y - self.cursor.y) as f32 / g.config.height as f32 * 2.0 * half[1];
                }
                self.cursor = position;
            },
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                self.dragging = state == ElementState::Pressed
            },
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                if let Some(g) = self.gfx.as_ref() {
                    // keep the point under the cursor fixed
                    let ndc = self.cursor_ndc(g);
                    let before = self.half(g);
                    let anchor = [self.centre[0] + ndc[0] * before[0], self.centre[1] + ndc[1] * before[1]];
                    self.half_height = (self.half_height * 1.15f32.powf(-lines)).clamp(0.002, 4.0);
                    let after = self.half(g);
                    self.centre = [anchor[0] - ndc[0] * after[0], anchor[1] - ndc[1] * after[1]];
                }
            },
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed && !event.repeat => {
                match event.logical_key.as_ref() {
                    Key::Named(NamedKey::Space) => self.paused = !self.paused,
                    Key::Named(NamedKey::Escape) => el.exit(),
                    Key::Character("r") => {
                        self.centre = [0.5, 0.5];
                        self.half_height = 0.5;
                    },
                    _ => {},
                }
            },
            _ => {},
        }
    }

    fn about_to_wait(&mut self, _el: &ActiveEventLoop) {
        if let Some(g) = self.gfx.as_ref() {
            g.window.request_redraw();
        }
    }
}

pub fn run(common: Common) {
    let event_loop = EventLoop::new().expect("event loop");
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(Box::new(
        event_loop.owned_display_handle(),
    )));
    let now = Instant::now();
    let mut app = App {
        common,
        instance,
        gfx: None,
        centre: [0.5, 0.5],
        half_height: 0.5,
        paused: false,
        time: 0.0,
        last: now,
        cursor: PhysicalPosition::new(0.0, 0.0),
        dragging: false,
        frames: 0,
        fps_at: now,
    };
    event_loop.run_app(&mut app).expect("run");
}
