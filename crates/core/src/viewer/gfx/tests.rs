//! Draws real frames headlessly through every extreme the menu can reach, and through changes
//! between any two frames, failing on any error the GPU reports.

use assert2::{assert, check};
use rstest::rstest;
use winit::dpi::PhysicalSize;

use super::Canvas;
use crate::gpu::{self, Gpu};
use crate::renderer::FieldView;
use crate::settings::{CATALOG, Settings, limits};
use crate::shaders::Sources;
use crate::viewer::ui::Painted;
use crate::viewer::ui::panels::{Info, Menu};

/// What a desktop surface usually is.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8UnormSrgb;

fn canvas(physical: PhysicalSize<u32>, settings: &Settings) -> Canvas {
    let instance = wgpu::Instance::default();
    let Gpu { device, queue, .. } = pollster::block_on(gpu::request(&instance, None)).expect("a graphics adapter");
    Canvas::new(device, queue, FORMAT, physical, settings, &Sources::embedded())
}

/// The menu open over a window of `physical` pixels, as the overlay paints it.
fn menu(physical: PhysicalSize<u32>, ratio: f32, settings: &mut Settings) -> Painted {
    let ctx = egui::Context::default();
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(physical.width as f32, physical.height as f32),
        )),
        ..Default::default()
    };
    let mut menu = Menu::default();
    menu.open = true;
    let output = ctx.run_ui(input, |ui| {
        menu.show(ui, settings, &Info::default());
    });
    let pixels_per_point = output.pixels_per_point * ratio;
    let primitives = ctx.tessellate(output.shapes, pixels_per_point);
    Painted { primitives, textures: output.textures_delta, pixels_per_point }
}

/// Draws one frame onto a fresh target of the size the canvas asks for, as a surface would be
/// configured; returns any error the GPU raised while doing it.
fn frame(canvas: &mut Canvas, settings: &mut Settings, time: f32, view: Option<FieldView>) -> Option<String> {
    let device = canvas.device().clone();
    let scopes = [wgpu::ErrorFilter::Validation, wgpu::ErrorFilter::OutOfMemory, wgpu::ErrorFilter::Internal]
        .map(|filter| device.push_error_scope(filter));
    let size = canvas.size();
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("stand-in surface"),
        size: wgpu::Extent3d { width: size.width, height: size.height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let uniforms = canvas.uniforms(time, [0.0, 0.0, 1.0, 1.0], settings.params.0);
    let overlay = menu(canvas.physical, canvas.surface_ratio(), settings);
    canvas.draw(&target.create_view(&Default::default()), &uniforms, view, Some(overlay));
    device.poll(wgpu::PollType::wait_indefinitely()).expect("the frame finishes");
    scopes.into_iter().rev().find_map(|scope| pollster::block_on(scope.pop())).map(|e| format!("{e:?}"))
}

#[derive(Clone, Copy, Debug)]
enum Budget {
    Unlimited,
    /// The smallest cap the menu offers, held fixed.
    Smallest,
    /// The default web budget, adapting with the scale at its floor.
    AdaptiveFloor,
}

impl Budget {
    fn apply(self, settings: &mut Settings) {
        let r = &mut settings.render;
        match self {
            Self::Unlimited => {},
            Self::Smallest => r.max_pixels = 100_000.0,
            Self::AdaptiveFloor => {
                r.max_pixels = 3_686_400.0;
                r.render_scale = *limits::RENDER_SCALE.start();
                r.adaptive = true;
            },
        }
    }
}

#[rstest]
fn a_frame_draws_cleanly_at_every_extreme(
    #[values((900, 900), (3840, 2160), (1, 1), (40_000, 200))] window: (u32, u32),
    #[values(1, 4)] supersampling: u32,
    #[values(1, 4)] field_step: u32,
    #[values(Budget::Unlimited, Budget::Smallest, Budget::AdaptiveFloor)] budget: Budget,
) {
    let physical = PhysicalSize::new(window.0, window.1);
    let mut settings = Settings::default();
    settings.render.supersampling = supersampling;
    settings.render.field_step = field_step;
    budget.apply(&mut settings);
    let mut canvas = canvas(physical, &settings);

    let size = canvas.size();
    let max = canvas.max_side();
    let shaded = size.width.max(size.height) * canvas.supersampling();
    check!(shaded <= max, "{size:?} at {}x outgrows {max}", canvas.supersampling());
    check!(f64::from(size.width) * f64::from(size.height) >= 1.0);

    for i in 0..3 {
        let error = frame(&mut canvas, &mut settings, i as f32 / 30.0, None);
        assert!(error.is_none(), "frame {i}: {}", error.unwrap_or_default());
    }
}

/// Settings as far as the menu takes each of them, one change at a time.
type Step = (String, Box<dyn Fn(&mut Settings)>);

fn extremes() -> Vec<Step> {
    let mut steps: Vec<Step> = vec![
        ("supersampling up".into(), Box::new(|s| s.render.supersampling = *limits::SUPERSAMPLING.end())),
        ("field step up".into(), Box::new(|s| s.render.field_step = *limits::FIELD_STEP.end())),
        ("temporal off".into(), Box::new(|s| s.render.temporal = false)),
        ("pixel cap at the top".into(), Box::new(|s| s.render.max_pixels = *limits::MAX_PIXELS.end())),
        ("pixel cap at the bottom".into(), Box::new(|s| s.render.max_pixels = 100_000.0)),
        ("render scale floor".into(), Box::new(|s| s.render.render_scale = *limits::RENDER_SCALE.start())),
        ("adaptive on".into(), Box::new(|s| s.render.adaptive = true)),
        ("pixel cap off".into(), Box::new(|s| s.render.max_pixels = 0.0)),
        ("supersampling down".into(), Box::new(|s| s.render.supersampling = 1)),
        ("temporal on".into(), Box::new(|s| s.render.temporal = true)),
        ("speed top".into(), Box::new(|s| s.motion.speed = *limits::SPEED.end())),
    ];
    for spec in &CATALOG {
        let slot = spec.slot;
        steps.push((format!("{} at {}", spec.key, spec.min), Box::new(move |s| *s.params.get_mut(slot) = spec.min)));
        steps.push((format!("{} at {}", spec.key, spec.max), Box::new(move |s| *s.params.get_mut(slot) = spec.max)));
    }
    steps.push(("everything back".into(), Box::new(|s| *s = Settings::default())));
    steps
}

#[rstest]
fn settings_can_change_between_any_two_frames(
    #[values((1280, 720), (3840, 2160))] window: (u32, u32),
    #[values(None, Some(FieldView::Velocity))] view: Option<FieldView>,
) {
    let physical = PhysicalSize::new(window.0, window.1);
    let mut settings = Settings::default();
    let mut canvas = canvas(physical, &settings);
    for (i, (name, step)) in extremes().iter().enumerate() {
        let mut next = settings.clone();
        step(&mut next);
        check!(next.clone().clamp().is_empty(), "{name} leaves the bounds a file is held to");
        canvas.apply(&settings, &next);
        settings = next;
        if i % 7 == 3 {
            canvas.resize(PhysicalSize::new(physical.width / 2 + i as u32, physical.height / 3));
        }
        let error = frame(&mut canvas, &mut settings, i as f32 / 30.0, view);
        assert!(error.is_none(), "after {name}: {}", error.unwrap_or_default());
    }
}

#[test]
fn a_window_shrunk_to_nothing_still_draws() {
    let mut settings = Settings::default();
    let mut canvas = canvas(PhysicalSize::new(800, 600), &settings);
    canvas.resize(PhysicalSize::new(0, 0));
    check!(canvas.size().width >= 1 && canvas.size().height >= 1);
    let error = frame(&mut canvas, &mut settings, 0.0, None);
    assert!(error.is_none(), "{}", error.unwrap_or_default());
}
