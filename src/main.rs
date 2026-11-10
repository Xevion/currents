mod renderer;
mod viewer;

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use renderer::{Renderer, Uniforms};

const COVER: &str = "references/stills/cover-apple-3000.jpg";

/// Live rendering of the Currents cover. With no subcommand, opens the interactive viewer
/// (scroll to zoom at the cursor, drag to pan, space to pause, R to reset, Esc to quit).
#[derive(Parser)]
struct Cli {
    #[command(flatten)]
    common: Common,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Args, Clone)]
pub struct Common {
    /// Directory holding the WGSL passes
    #[arg(long, default_value = "shaders", global = true)]
    pub shaders: PathBuf,
    /// Flow speed multiplier
    #[arg(long, default_value_t = 0.4, global = true)]
    pub speed: f32,
    /// Supersampling factor per axis
    #[arg(long, default_value_t = 2, global = true)]
    pub ss: u32,
    /// Tuning parameter override, INDEX=VALUE (repeatable)
    #[arg(short = 'p', long = "param", value_parser = parse_param, global = true)]
    pub params: Vec<(usize, f32)>,
}

impl Common {
    pub fn param_array(&self) -> [[f32; 4]; 8] {
        let mut out = [[0.0; 4]; 8];
        for &(i, v) in &self.params {
            out[i / 4][i % 4] = v;
        }
        out
    }
}

fn parse_param(s: &str) -> Result<(usize, f32), String> {
    let (i, v) = s.split_once('=').ok_or("expected INDEX=VALUE")?;
    let i: usize = i.parse().map_err(|e| format!("index: {e}"))?;
    if i >= 32 {
        return Err("index must be below 32".into());
    }
    Ok((i, v.parse().map_err(|e| format!("value: {e}"))?))
}

#[derive(Subcommand)]
enum Command {
    /// Render frames to PNG without opening a window
    Render(RenderArgs),
}

#[derive(Args)]
struct RenderArgs {
    /// Named view (full, wide, a..h, user) or x0,y0,x1,y1 in cover coordinates
    #[arg(long, default_value = "full", value_parser = parse_view, allow_hyphen_values = true)]
    view: [f32; 4],
    /// Output size as WxH; defaults to 800 pixels tall at the view's aspect
    #[arg(long, value_parser = parse_size)]
    size: Option<(u32, u32)>,
    /// Start time in seconds (before the speed multiplier)
    #[arg(long, default_value_t = 0.0)]
    time: f32,
    /// Output PNG, or a directory when rendering several frames
    #[arg(long, default_value = "out/frame.png")]
    out: PathBuf,
    /// Place the matching crop of the cover on the left
    #[arg(long = "ref")]
    reference: bool,
    #[arg(long, default_value_t = 1)]
    frames: u32,
    #[arg(long, default_value_t = 30.0)]
    fps: f32,
}

fn parse_size(s: &str) -> Result<(u32, u32), String> {
    let (w, h) = s.split_once('x').ok_or("expected WxH")?;
    Ok((w.parse().map_err(|e| format!("{e}"))?, h.parse().map_err(|e| format!("{e}"))?))
}

// Named views are rectangles in cover coordinates; the cover square is 0..1.
fn parse_view(s: &str) -> Result<[f32; 4], String> {
    let px = |x: f32, y: f32, w: f32, h: f32| [x / 1500.0, y / 1500.0, (x + w) / 1500.0, (y + h) / 1500.0];
    Ok(match s {
        "full" => [0.0, 0.0, 1.0, 1.0],
        "wide" => [-0.3889, 0.0, 1.3889, 1.0],
        "a" => px(600.0, 740.0, 420.0, 420.0),
        "b" => px(400.0, 760.0, 400.0, 400.0),
        "c" => px(900.0, 880.0, 450.0, 450.0),
        "d" => px(1000.0, 1200.0, 340.0, 300.0),
        "e" => px(280.0, 375.0, 375.0, 375.0),
        "f" => px(470.0, 1125.0, 400.0, 375.0),
        "g" => px(1100.0, 0.0, 400.0, 400.0),
        "h" => px(1125.0, 750.0, 375.0, 375.0),
        "user" => px(330.0, 560.0, 900.0, 755.0),
        _ => {
            let f: Vec<f32> =
                s.split(',').map(|x| x.parse::<f32>().map_err(|e| format!("{e}"))).collect::<Result<_, _>>()?;
            if f.len() != 4 {
                return Err("expected a view name or x0,y0,x1,y1".into());
            }
            [f[0], f[1], f[2], f[3]]
        },
    })
}

fn srgb_to_lin(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn lin_to_srgb(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5) as u8
}

// Box downsample in linear light.
fn downsample(src: &[u8], w: u32, ss: u32, ow: u32, oh: u32) -> image::RgbImage {
    let lut: Vec<f32> = (0..=255u8).map(srgb_to_lin).collect();
    let n = (ss * ss) as f32;
    image::RgbImage::from_fn(ow, oh, |x, y| {
        let mut acc = [0.0f32; 3];
        for sy in 0..ss {
            for sx in 0..ss {
                let i = (((y * ss + sy) * w + x * ss + sx) * 4) as usize;
                for c in 0..3 {
                    acc[c] += lut[src[i + c] as usize];
                }
            }
        }
        image::Rgb(acc.map(|a| lin_to_srgb(a / n)))
    })
}

fn render(common: &Common, args: &RenderArgs) {
    let v = args.view;
    let aspect = (v[2] - v[0]) / (v[3] - v[1]);
    let (ow, oh) = args.size.unwrap_or(((800.0 * aspect).round() as u32, 800));
    let ss = common.ss.max(1);
    let (w, h) = (ow * ss, oh * ss);

    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .expect("adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&renderer::device_desc(&adapter))).expect("device");
    let mut renderer = Renderer::new(&device, &queue, &common.shaders);
    let mut pass_sum = [0.0f32; 2];

    let row = (w * 4).next_multiple_of(256);
    let rbuf = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (row * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    for frame in 0..args.frames {
        let time = (args.time + frame as f32 / args.fps) * common.speed;
        let out_path = if args.frames > 1 { args.out.join(format!("{frame:04}.png")) } else { args.out.clone() };
        let uniforms = Uniforms {
            resolution: [w as f32, h as f32],
            time,
            field_scale: ss as f32,
            view: v,
            params: common.param_array(),
        };

        let t0 = std::time::Instant::now();
        let mut enc = device.create_command_encoder(&Default::default());
        renderer.render(&device, &queue, &mut enc, &uniforms);
        enc.copy_texture_to_buffer(
            renderer.color().as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &rbuf,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        queue.submit([enc.finish()]);
        renderer.after_submit();
        let slice = rbuf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let gpu_ms = t0.elapsed().as_secs_f32() * 1000.0;
        let pass = renderer.pass_ms(&device, true).unwrap_or_default();
        pass_sum = [pass_sum[0] + pass[0], pass_sum[1] + pass[1]];

        let image = {
            let data = slice.get_mapped_range().unwrap();
            let tight: Vec<u8> =
                (0..h).flat_map(|y| data[(y * row) as usize..(y * row + w * 4) as usize].iter().copied()).collect();
            downsample(&tight, w, ss, ow, oh)
        };
        rbuf.unmap();

        if let Some(dir) = out_path.parent() {
            std::fs::create_dir_all(dir).unwrap();
        }
        if args.reference {
            let cover = image::open(COVER).expect("cover").to_rgb8();
            let cs = cover.width() as f32;
            let (cx, cy) = ((v[0] * cs).max(0.0) as u32, (v[1] * cs).max(0.0) as u32);
            let (cw, ch) = (((v[2] - v[0]) * cs) as u32, ((v[3] - v[1]) * cs) as u32);
            let crop =
                image::imageops::crop_imm(&cover, cx, cy, cw.min(cover.width() - cx), ch.min(cover.height() - cy))
                    .to_image();
            let crop = image::imageops::resize(&crop, ow, oh, image::imageops::FilterType::CatmullRom);
            let mut both = image::RgbImage::new(ow * 2 + 8, oh);
            image::imageops::replace(&mut both, &crop, 0, 0);
            image::imageops::replace(&mut both, &image, (ow + 8) as i64, 0);
            both.save(&out_path).unwrap();
        } else {
            image.save(&out_path).unwrap();
        }
        if args.frames == 1 || frame + 1 == args.frames {
            let n = args.frames as f32;
            println!(
                "{} {}x{} (ss {}) wall {:.0}ms | field {:.2}ms shade {:.2}ms (mean of {})",
                out_path.display(),
                ow,
                oh,
                ss,
                gpu_ms,
                pass_sum[0] / n,
                pass_sum[1] / n,
                args.frames
            );
        }
    }
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Render(args)) => render(&cli.common, &args),
        None => viewer::run(cli.common),
    }
}
