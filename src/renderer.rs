use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Uniforms {
    pub resolution: [f32; 2],
    pub time: f32,
    /// Shade pixels per field texel along each axis.
    pub field_scale: f32,
    pub view: [f32; 4],
    pub params: [[f32; 4]; 8],
}

/// Device request that turns on GPU timestamps where the adapter has them.
pub fn device_desc(adapter: &wgpu::Adapter) -> wgpu::DeviceDescriptor<'static> {
    wgpu::DeviceDescriptor {
        required_features: wgpu::Features::FLOAT32_FILTERABLE | (adapter.features() & wgpu::Features::TIMESTAMP_QUERY),
        ..Default::default()
    }
}

const FIELD_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

struct Pass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

struct Targets {
    size: (u32, u32, u32),
    field_view: wgpu::TextureView,
    color: wgpu::Texture,
    color_view: wgpu::TextureView,
    field_bind: wgpu::BindGroup,
    shade_bind: wgpu::BindGroup,
}

/// Two fullscreen passes: `field` writes the stripe coordinate, `shade` turns it into colour.
pub struct Renderer {
    ubuf: wgpu::Buffer,
    field: Pass,
    shade: Pass,
    targets: Option<Targets>,
    sampler: wgpu::Sampler,
    timing: Option<Timing>,
}

/// GPU timestamps around each pass, read back without stalling: a frame's stamps are copied
/// out only while the readback buffer is free, and picked up once its mapping completes.
struct Timing {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    read: wgpu::Buffer,
    period_ns: f32,
    copied: bool,
    mapping: bool,
    ready: Arc<AtomicBool>,
    last: Option<[f32; 2]>,
}

const PASSES: u32 = 2;

fn make_pass(
    device: &wgpu::Device,
    common: &str,
    dir: &Path,
    name: &str,
    inputs: u32,
    format: wgpu::TextureFormat,
) -> Pass {
    let src = std::fs::read_to_string(dir.join(format!("{name}.wgsl"))).unwrap_or_else(|e| panic!("{name}.wgsl: {e}"));
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(name),
        source: wgpu::ShaderSource::Wgsl(format!("{common}\n{src}").into()),
    });
    let mut entries = vec![wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }];
    entries.extend((0..inputs).map(|i| wgpu::BindGroupLayoutEntry {
        binding: i + 1,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }));
    if inputs > 0 {
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: inputs + 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        });
    }
    let layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(name), entries: &entries });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(name),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(name),
        layout: Some(&pl),
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
            targets: &[Some(format.into())],
        }),
        multiview_mask: None,
        cache: None,
    });
    Pass { pipeline, layout }
}

fn make_tex(device: &wgpu::Device, w: u32, h: u32, format: wgpu::TextureFormat) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

impl Renderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, shader_dir: &Path) -> Self {
        let common = std::fs::read_to_string(shader_dir.join("common.wgsl")).expect("common.wgsl");
        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            ubuf,
            field: make_pass(device, &common, shader_dir, "field", 0, FIELD_FORMAT),
            shade: make_pass(device, &common, shader_dir, "shade", 1, COLOR_FORMAT),
            targets: None,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            timing: device.features().contains(wgpu::Features::TIMESTAMP_QUERY).then(|| {
                let buf = |usage| {
                    device.create_buffer(&wgpu::BufferDescriptor {
                        label: None,
                        size: 8 * 2 * PASSES as u64,
                        usage,
                        mapped_at_creation: false,
                    })
                };
                Timing {
                    set: device.create_query_set(&wgpu::QuerySetDescriptor {
                        label: None,
                        ty: wgpu::QueryType::Timestamp,
                        count: 2 * PASSES,
                    }),
                    resolve: buf(wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC),
                    read: buf(wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ),
                    period_ns: queue.get_timestamp_period(),
                    copied: false,
                    mapping: false,
                    ready: Arc::new(AtomicBool::new(false)),
                    last: None,
                }
            }),
        }
    }

    /// Call after submitting the encoder passed to `render`: starts reading back its timestamps.
    pub fn after_submit(&mut self) {
        let Some(t) = self.timing.as_mut() else { return };
        if t.copied && !t.mapping {
            t.copied = false;
            t.mapping = true;
            let ready = t.ready.clone();
            t.read.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                if r.is_ok() {
                    ready.store(true, Ordering::Release);
                }
            });
        }
    }

    /// Milliseconds in the field and shade passes of the most recent frame read back. With
    /// `wait`, blocks until the latest submitted readback lands; otherwise never stalls.
    pub fn pass_ms(&mut self, device: &wgpu::Device, wait: bool) -> Option<[f32; 2]> {
        let t = self.timing.as_mut()?;
        if t.mapping {
            let poll = if wait { wgpu::PollType::wait_indefinitely() } else { wgpu::PollType::Poll };
            device.poll(poll).unwrap();
        }
        if t.ready.swap(false, Ordering::Acquire) {
            let ticks: Vec<u64> = bytemuck::cast_slice(&t.read.slice(..).get_mapped_range().unwrap()).to_vec();
            t.read.unmap();
            t.mapping = false;
            let ms = |i: usize| ticks[2 * i + 1].saturating_sub(ticks[2 * i]) as f32 * t.period_ns * 1e-6;
            t.last = Some([ms(0), ms(1)]);
        }
        t.last
    }

    fn ensure_targets(&mut self, device: &wgpu::Device, w: u32, h: u32, fs: u32) {
        if self.targets.as_ref().is_some_and(|t| t.size == (w, h, fs)) {
            return;
        }
        let field_view =
            make_tex(device, w.div_ceil(fs), h.div_ceil(fs), FIELD_FORMAT).create_view(&Default::default());
        let color = make_tex(device, w, h, COLOR_FORMAT);
        let color_view = color.create_view(&Default::default());
        let field_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.field.layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: self.ubuf.as_entire_binding() }],
        });
        let shade_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.shade.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.ubuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&field_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        self.targets = Some(Targets { size: (w, h, fs), field_view, color, color_view, field_bind, shade_bind });
    }

    /// Renders one frame into the internal colour texture; `uniforms.resolution` sets its size.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        enc: &mut wgpu::CommandEncoder,
        uniforms: &Uniforms,
    ) {
        let (w, h) = (uniforms.resolution[0] as u32, uniforms.resolution[1] as u32);
        self.ensure_targets(device, w, h, uniforms.field_scale.max(1.0) as u32);
        queue.write_buffer(&self.ubuf, 0, bytemuck::bytes_of(uniforms));
        let t = self.targets.as_ref().unwrap();
        for (i, (pass, bind, view)) in
            [(&self.field, &t.field_bind, &t.field_view), (&self.shade, &t.shade_bind, &t.color_view)]
                .into_iter()
                .enumerate()
        {
            let i = i as u32;
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                timestamp_writes: self.timing.as_ref().map(|tm| wgpu::RenderPassTimestampWrites {
                    query_set: &tm.set,
                    beginning_of_pass_write_index: Some(2 * i),
                    end_of_pass_write_index: Some(2 * i + 1),
                }),
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            rp.set_pipeline(&pass.pipeline);
            rp.set_bind_group(0, bind, &[]);
            rp.draw(0..3, 0..1);
        }
        if let Some(tm) = self.timing.as_mut()
            && !tm.mapping
        {
            enc.resolve_query_set(&tm.set, 0..2 * PASSES, &tm.resolve, 0);
            enc.copy_buffer_to_buffer(&tm.resolve, 0, &tm.read, 0, None);
            tm.copied = true;
        }
    }

    pub fn color(&self) -> &wgpu::Texture {
        &self.targets.as_ref().expect("render first").color
    }

    pub fn color_view(&self) -> &wgpu::TextureView {
        &self.targets.as_ref().expect("render first").color_view
    }
}
