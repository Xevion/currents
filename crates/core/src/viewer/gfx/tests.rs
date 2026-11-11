//! Draws real frames headlessly, onto targets the window's size as a surface is, through every
//! extreme the menu can reach and through changes between any two frames, failing on any
//! error the GPU reports; reads frames back to check what they show.

use assert2::{assert, check};
use rstest::rstest;
use winit::dpi::PhysicalSize;

use super::{Canvas, surface_format};
use crate::gpu::{self, Gpu};
use crate::renderer::FieldView;
use crate::settings::{Settings, limits, toml};
use crate::shaders::Sources;
use crate::viewer::testing::extremes;
use crate::viewer::ui::Painted;
use crate::viewer::ui::panels::{Info, Menu};

/// What the viewer configures a desktop surface as.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

fn canvas_with(
    physical: PhysicalSize<u32>,
    settings: &Settings,
    sources: &Sources,
    format: wgpu::TextureFormat,
) -> Canvas {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle().with_env());
    let Gpu { device, queue, .. } = pollster::block_on(gpu::request(&instance, None)).expect("a graphics adapter");
    pollster::block_on(Canvas::create(device, queue, format, physical, settings, sources))
}

fn canvas(physical: PhysicalSize<u32>, settings: &Settings) -> Canvas {
    canvas_with(physical, settings, &Sources::embedded(), FORMAT)
}

/// The menu open over a window of `physical` pixels at one pixel per point.
fn menu(physical: PhysicalSize<u32>, settings: &mut Settings) -> Painted {
    let ctx = egui::Context::default();
    crate::viewer::ui::install_theme(&ctx);
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(physical.width as f32, physical.height as f32),
        )),
        ..Default::default()
    };
    let mut menu = Menu::default();
    menu.open = true;
    // A window measures itself unseen on its first frame, so the second is the one painted;
    // the first frame's uploads, the font atlas among them, still have to reach the GPU.
    let mut textures = egui::TexturesDelta::default();
    let mut output = None;
    for _ in 0..2 {
        let out = ctx.run_ui(input.clone(), |ui| {
            menu.show(ui, settings, &Info::default());
        });
        output = Some(out);
        if let Some(out) = output.as_mut() {
            textures.append(std::mem::take(&mut out.textures_delta));
        }
    }
    let output = output.expect("ran twice");
    let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
    Painted { primitives, textures, pixels_per_point: output.pixels_per_point }
}

/// A frame as a surface would get it: drawn onto a fresh target of the window's size.
struct Frame {
    target: wgpu::Texture,
    error: Option<String>,
}

fn frame(canvas: &mut Canvas, settings: &mut Settings, time: f32, view: Option<FieldView>, overlay: bool) -> Frame {
    let device = canvas.device().clone();
    let scopes = [wgpu::ErrorFilter::Validation, wgpu::ErrorFilter::OutOfMemory, wgpu::ErrorFilter::Internal]
        .map(|filter| device.push_error_scope(filter));
    let size = canvas.target_size();
    let format = canvas.format;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("stand-in surface"),
        size: wgpu::Extent3d { width: size.width, height: size.height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let uniforms = canvas.uniforms(f64::from(time), [0.0, 0.0, 1.0, 1.0], settings.params.0);
    let painted = overlay.then(|| menu(canvas.physical, settings));
    canvas.draw(&target, &uniforms, view, painted);
    device.poll(wgpu::PollType::wait_indefinitely()).expect("the frame finishes");
    let error = scopes.into_iter().rev().find_map(|scope| pollster::block_on(scope.pop())).map(|e| format!("{e:?}"));
    Frame { target, error }
}

/// The frame's pixels as RGB, row by row.
fn pixels(canvas: &Canvas, target: &wgpu::Texture) -> Vec<[u8; 3]> {
    let device = canvas.device();
    let (w, h) = (target.width(), target.height());
    let row = (w * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(row * h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) },
        },
        target.size(),
    );
    canvas.queue.submit([enc.finish()]);
    buffer.slice(..).map_async(wgpu::MapMode::Read, |r| r.expect("the readback maps"));
    device.poll(wgpu::PollType::wait_indefinitely()).expect("the readback finishes");
    let data = buffer.slice(..).get_mapped_range().expect("the readback is mapped");
    let bgra = matches!(target.format(), wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
    (0..h)
        .flat_map(|y| (0..w).map(move |x| (y * row + x * 4) as usize))
        .map(|i| if bgra { [data[i + 2], data[i + 1], data[i]] } else { [data[i], data[i + 1], data[i + 2]] })
        .collect()
}

/// Mean colour of the pixels in the `[x0, y0, x1, y1]` share of a `w`-wide frame.
fn mean(pixels: &[[u8; 3]], w: u32, share: [f32; 4]) -> [f32; 3] {
    let h = pixels.len() as u32 / w;
    let (x0, x1) = ((share[0] * w as f32) as u32, (share[2] * w as f32) as u32);
    let (y0, y1) = ((share[1] * h as f32) as u32, (share[3] * h as f32) as u32);
    let mut sum = [0.0; 3];
    let mut n = 0.0;
    for y in y0..y1 {
        for x in x0..x1 {
            let p = pixels[(y * w + x) as usize];
            for c in 0..3 {
                sum[c] += f32::from(p[c]);
            }
            n += 1.0;
        }
    }
    sum.map(|s| s / n)
}

fn assert_clean(frame: &Frame, what: &str) {
    assert!(frame.error.is_none(), "{what}: {}", frame.error.clone().unwrap_or_default());
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
            Self::Smallest => r.max_pixels = Some(*limits::MAX_PIXELS.start()),
            Self::AdaptiveFloor => {
                r.max_pixels = Some(3_686_400.0);
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
    let shaded = size.width.max(size.height) * canvas.supersampling();
    check!(shaded <= canvas.max_side(), "{size:?} at {}x outgrows {}", canvas.supersampling(), canvas.max_side());
    let target = canvas.target_size();
    check!(target.width <= canvas.max_texture_side() && target.height <= canvas.max_texture_side());
    check!(size.width <= target.width.max(1) && size.height <= target.height.max(1));

    for i in 0..3 {
        let frame = frame(&mut canvas, &mut settings, i as f32 / 30.0, None, true);
        assert_clean(&frame, &format!("frame {i}"));
        check!(canvas.fault().is_none());
    }
}

#[rstest]
#[case::plain(wgpu::TextureFormat::Bgra8Unorm)]
#[case::srgb(wgpu::TextureFormat::Bgra8UnormSrgb)]
#[case::rgba(wgpu::TextureFormat::Rgba8Unorm)]
fn frames_draw_in_any_surface_format(#[case] format: wgpu::TextureFormat) {
    let mut settings = Settings::default();
    let mut canvas = canvas_with(PhysicalSize::new(640, 480), &settings, &Sources::embedded(), format);
    for i in 0..2 {
        assert_clean(&frame(&mut canvas, &mut settings, i as f32, None, true), &format!("{format:?}"));
    }
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
        assert_clean(&frame(&mut canvas, &mut settings, i as f32 / 30.0, view, i % 2 == 0), &format!("after {name}"));
    }
}

#[test]
fn a_window_shrunk_to_nothing_still_draws() {
    let mut settings = Settings::default();
    let mut canvas = canvas(PhysicalSize::new(800, 600), &settings);
    canvas.resize(PhysicalSize::new(0, 0));
    check!(canvas.size().width >= 1 && canvas.target_size().width >= 1);
    assert_clean(&frame(&mut canvas, &mut settings, 0.0, None, true), "zero-sized window");
}

#[rstest]
#[case::smallest_cap(Some(*limits::MAX_PIXELS.start()), 1.0)]
#[case::scale_floor(None, *limits::RENDER_SCALE.start())]
fn a_picture_shaded_smaller_fills_the_whole_window(#[case] cap: Option<f32>, #[case] scale: f32) {
    let physical = PhysicalSize::new(900, 700);
    let mut full = Settings::default();
    full.render.temporal = false;
    let mut small = full.clone();
    small.render.max_pixels = cap;
    small.render.render_scale = scale;

    let mut whole = canvas(physical, &full);
    let mut scaled = canvas(physical, &small);
    check!(scaled.size().width < physical.width / 2, "the picture is shaded well under the window's size");
    let expected = frame(&mut whole, &mut full, 4.0, None, false);
    let got = frame(&mut scaled, &mut small, 4.0, None, false);
    assert_clean(&got, "scaled");
    check!(got.target.width() == physical.width && got.target.height() == physical.height);

    let (a, b) = (pixels(&whole, &expected.target), pixels(&scaled, &got.target));
    for qy in 0..4 {
        for qx in 0..4 {
            let share = [qx as f32 / 4.0, qy as f32 / 4.0, (qx + 1) as f32 / 4.0, (qy + 1) as f32 / 4.0];
            let (ma, mb) = (mean(&a, physical.width, share), mean(&b, physical.width, share));
            let off = (0..3).map(|c| (ma[c] - mb[c]).abs()).fold(0.0, f32::max);
            // Stripes finer than the smaller picture's pixels average out darker, which moves a
            // block's mean by up to about 14 levels; a misplaced, cropped or stretched picture
            // moves it by tens more.
            check!(off < 25.0, "block ({qx}, {qy}) differs by {off}: {ma:?} against {mb:?}");
        }
    }
}

/// The file the menu wrote that froze the viewer at launch.
const FROZE_AT_LAUNCH: &str = "[params]\nobstacle_scale = 0.21\nglare_gain = 0.3\nband_edge_tone = 0.06\nband_width_jitter = -0.05\nspray_mottle = 0.045\n\n[render]\nsupersampling = 1\nadaptive = false\nmax_pixels = 800000.0\nfield_step = 1\nfps_cap = 0\n";

#[rstest]
fn the_settings_file_that_froze_launch_draws_the_picture(
    #[values((900, 900), (1920, 1080), (2560, 1440), (5120, 1440))] window: (u32, u32),
) {
    let (mut settings, issues) = toml::load(FROZE_AT_LAUNCH).expect("the file loads");
    check!(issues.is_empty(), "{issues:?}");
    let physical = PhysicalSize::new(window.0, window.1);
    let mut canvas = canvas(physical, &settings);
    check!(canvas.target_size() == physical);
    for i in 0..5 {
        let frame = frame(&mut canvas, &mut settings, i as f32 / 60.0, None, i == 4);
        assert_clean(&frame, &format!("frame {i}"));
        if i == 3 {
            let lit = mean(&pixels(&canvas, &frame.target), physical.width, [0.0, 0.0, 1.0, 1.0]);
            check!(lit.iter().sum::<f32>() > 30.0, "the picture is there, not black: {lit:?}");
        }
    }
}

#[test]
fn a_fault_draws_black_under_a_working_menu_until_settings_change() {
    let physical = PhysicalSize::new(1000, 800);
    let mut settings = Settings::default();
    let mut canvas = canvas(physical, &settings);
    canvas.set_fault("a test fault");

    let faulted = frame(&mut canvas, &mut settings, 1.0, None, true);
    assert_clean(&faulted, "faulted frame");
    let px = pixels(&canvas, &faulted.target);
    let far_corner = mean(&px, physical.width, [0.8, 0.8, 1.0, 1.0]);
    check!(far_corner.iter().all(|&c| c < 0.5), "outside the menu is black: {far_corner:?}");
    let menu_body = mean(&px, physical.width, [0.03, 0.03, 0.3, 0.2]);
    check!(menu_body.iter().sum::<f32>() > 10.0, "the menu draws on top: {menu_body:?}");

    let mut next = settings.clone();
    next.motion.speed = 1.0;
    canvas.apply(&settings, &next);
    check!(canvas.fault().is_none());
    let recovered = frame(&mut canvas, &mut next, 1.0, None, false);
    let lit = mean(&pixels(&canvas, &recovered.target), physical.width, [0.0, 0.0, 1.0, 1.0]);
    check!(lit.iter().sum::<f32>() > 30.0, "the picture is back: {lit:?}");
}

#[test]
fn shaders_that_do_not_compile_leave_the_viewer_drawing_its_menu() {
    let mut sources = Sources::embedded();
    sources.shade = "this is not wgsl".to_owned();
    let mut settings = Settings::default();
    let mut canvas = canvas_with(PhysicalSize::new(800, 600), &settings, &sources, FORMAT);
    let fault = canvas.fault();
    check!(fault.as_deref().is_some_and(|f| f.contains("did not compile")), "{fault:?}");
    for i in 0..3 {
        assert_clean(&frame(&mut canvas, &mut settings, i as f32, None, true), &format!("frame {i}"));
    }
}

#[test]
fn a_device_error_outside_any_scope_faults_the_canvas_instead_of_panicking() {
    let canvas = canvas(PhysicalSize::new(320, 240), &Settings::default());
    // A texture with no texels is a validation error, which by default would panic.
    let _ = canvas.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("invalid"),
        size: wgpu::Extent3d { width: 0, height: 0, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    canvas.device().poll(wgpu::PollType::wait_indefinitely()).expect("the device settles");
    check!(canvas.fault().is_some());
}

#[rstest]
#[case::prefers_plain(&[wgpu::TextureFormat::Bgra8UnormSrgb, wgpu::TextureFormat::Bgra8Unorm], Some(wgpu::TextureFormat::Bgra8Unorm))]
#[case::only_srgb(&[wgpu::TextureFormat::Rgba8UnormSrgb], Some(wgpu::TextureFormat::Rgba8UnormSrgb))]
#[case::none(&[], None)]
fn the_surface_takes_a_plain_format_when_offered(
    #[case] offered: &[wgpu::TextureFormat],
    #[case] chosen: Option<wgpu::TextureFormat>,
) {
    check!(surface_format(offered) == chosen);
}
