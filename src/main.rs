use std::path::PathBuf;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    resolution: [f32; 2],
    time: f32,
    _pad: f32,
    view: [f32; 4],
    params: [[f32; 4]; 8],
}

const COVER: &str = "references/stills/cover-apple-3000.jpg";

// Named views in cover coords (x0, y0, x1, y1); the cover square is 0..1.
fn named_view(name: &str) -> Option<[f32; 4]> {
    let px = |x: f32, y: f32, w: f32, h: f32| [x / 1500.0, y / 1500.0, (x + w) / 1500.0, (y + h) / 1500.0];
    Some(match name {
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
        _ => return None,
    })
}

struct Args {
    size: Option<(u32, u32)>,
    view: [f32; 4],
    time: f32,
    out: PathBuf,
    shader: PathBuf,
    params: [[f32; 4]; 8],
    ss: u32,
    reference: bool,
    frames: u32,
    fps: f32,
    speed: f32,
}

fn parse_args() -> Args {
    let mut a = Args {
        size: None,
        view: [0.0, 0.0, 1.0, 1.0],
        time: 0.0,
        out: PathBuf::from("out/frame.png"),
        shader: PathBuf::from("shaders"),
        params: [[0.0; 4]; 8],
        ss: 2,
        reference: false,
        frames: 1,
        fps: 30.0,
        speed: 0.4,
    };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let mut v = || it.next().unwrap_or_else(|| panic!("missing value for {k}"));
        match k.as_str() {
            "--size" => {
                let s = v();
                let (w, h) = s.split_once('x').expect("WxH");
                a.size = Some((w.parse().unwrap(), h.parse().unwrap()));
            }
            "--view" => {
                let s = v();
                a.view = named_view(&s).unwrap_or_else(|| {
                    let f: Vec<f32> = s.split(',').map(|x| x.parse().unwrap()).collect();
                    [f[0], f[1], f[2], f[3]]
                });
            }
            "--time" => a.time = v().parse().unwrap(),
            "--out" => a.out = PathBuf::from(v()),
            "--shader" => a.shader = PathBuf::from(v()),
            "--ss" => a.ss = v().parse().unwrap(),
            "--ref" => a.reference = true,
            "--frames" => a.frames = v().parse().unwrap(),
            "--fps" => a.fps = v().parse().unwrap(),
            "--speed" => a.speed = v().parse().unwrap(),
            "--p" => {
                let s = v();
                let (i, val) = s.split_once('=').expect("i=value");
                let i: usize = i.parse().unwrap();
                a.params[i / 4][i % 4] = val.parse().unwrap();
            }
            _ => panic!("unknown arg {k}"),
        }
    }
    a
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
fn downsample(src: &[u8], w: u32, h: u32, ss: u32) -> image::RgbImage {
    let (ow, oh) = (w / ss, h / ss);
    let lut: Vec<f32> = (0..=255u8).map(srgb_to_lin).collect();
    let mut out = image::RgbImage::new(ow, oh);
    let n = (ss * ss) as f32;
    for y in 0..oh {
        for x in 0..ow {
            let mut acc = [0.0f32; 3];
            for sy in 0..ss {
                for sx in 0..ss {
                    let i = (((y * ss + sy) * w + x * ss + sx) * 4) as usize;
                    for c in 0..3 {
                        acc[c] += lut[src[i + c] as usize];
                    }
                }
            }
            out.put_pixel(x, y, image::Rgb([lin_to_srgb(acc[0] / n), lin_to_srgb(acc[1] / n), lin_to_srgb(acc[2] / n)]));
        }
    }
    out
}

fn main() {
    let args = parse_args();
    let aspect = (args.view[2] - args.view[0]) / (args.view[3] - args.view[1]);
    let (ow, oh) = args.size.unwrap_or(((800.0 * aspect).round() as u32, 800));
    let (w, h) = (ow * args.ss, oh * args.ss);

    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .expect("adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("device");

    for frame in 0..args.frames {
    let time = (args.time + frame as f32 / args.fps) * args.speed;
    let out_path = if args.frames > 1 { args.out.join(format!("{frame:04}.png")) } else { args.out.clone() };
    let uniforms = Uniforms {
        resolution: [w as f32, h as f32],
        time,
        _pad: 0.0,
        view: args.view,
        params: args.params,
    };
    let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: std::mem::size_of::<Uniforms>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&ubuf, 0, bytemuck::bytes_of(&uniforms));

    let make_tex = |w: u32, h: u32, format: wgpu::TextureFormat| {
        let t = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let v = t.create_view(&Default::default());
        (t, v)
    };
    let ff = wgpu::TextureFormat::Rgba32Float;
    let (_f0, f0v) = make_tex(w, h, ff);
    let (_f1, f1v) = make_tex(w, h, ff);
    let (_an, anv) = make_tex(64, 64, ff);
    let (tex, view) = make_tex(w, h, wgpu::TextureFormat::Rgba8Unorm);

    let common = std::fs::read_to_string(args.shader.join("common.wgsl")).expect("common.wgsl");
    // (pass name, inputs, output view, output format)
    let passes: [(&str, Vec<&wgpu::TextureView>, &wgpu::TextureView, wgpu::TextureFormat); 4] = [
        ("field", vec![], &f0v, ff),
        ("anchors", vec![&f0v], &anv, ff),
        ("drip", vec![&f0v, &anv], &f1v, ff),
        ("shade", vec![&f1v], &view, wgpu::TextureFormat::Rgba8Unorm),
    ];

    let t0 = std::time::Instant::now();
    let mut enc = device.create_command_encoder(&Default::default());
    for (name, inputs, out, format) in &passes {
        let src = std::fs::read_to_string(args.shader.join(format!("{name}.wgsl"))).expect("pass shader");
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(name),
            source: wgpu::ShaderSource::Wgsl(format!("{common}\n{src}").into()),
        });
        let mut lentries = vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        }];
        let mut entries = vec![wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() }];
        for (i, v) in inputs.iter().enumerate() {
            lentries.push(wgpu::BindGroupLayoutEntry {
                binding: i as u32 + 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
            entries.push(wgpu::BindGroupEntry { binding: i as u32 + 1, resource: wgpu::BindingResource::TextureView(v) });
        }
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &lentries });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(name),
            layout: Some(&layout),
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
                targets: &[Some((*format).into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &bgl, entries: &entries });
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(name),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: out,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }

    let row = (w * 4).next_multiple_of(256);
    let rbuf = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (row * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    enc.copy_texture_to_buffer(
        tex.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &rbuf,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) },
        },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    queue.submit([enc.finish()]);
    let slice = rbuf.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let gpu_ms = t0.elapsed().as_secs_f32() * 1000.0;

    let data = slice.get_mapped_range().unwrap();
    let mut tight = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let s = (y * row) as usize;
        tight.extend_from_slice(&data[s..s + (w * 4) as usize]);
    }
    let render = downsample(&tight, w, h, args.ss);

    if let Some(dir) = out_path.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    if args.reference {
        let cover = image::open(COVER).expect("cover").to_rgb8();
        let cs = cover.width() as f32;
        let v = args.view;
        let (cx, cy) = ((v[0] * cs).max(0.0) as u32, (v[1] * cs).max(0.0) as u32);
        let (cw, ch) = (((v[2] - v[0]) * cs) as u32, ((v[3] - v[1]) * cs) as u32);
        let crop = image::imageops::crop_imm(&cover, cx, cy, cw.min(cover.width() - cx), ch.min(cover.height() - cy)).to_image();
        let crop = image::imageops::resize(&crop, ow, oh, image::imageops::FilterType::CatmullRom);
        let mut both = image::RgbImage::new(ow * 2 + 8, oh);
        image::imageops::replace(&mut both, &crop, 0, 0);
        image::imageops::replace(&mut both, &render, (ow + 8) as i64, 0);
        both.save(&out_path).unwrap();
    } else {
        render.save(&out_path).unwrap();
    }
    println!("{} {}x{} (ss {}) gpu {:.0}ms", out_path.display(), ow, oh, args.ss, gpu_ms);
    }
}
