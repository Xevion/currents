pub(super) struct Pass {
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) layout: wgpu::BindGroupLayout,
}

/// How a pass reads its input textures.
#[derive(Clone, Copy)]
pub(super) struct Inputs {
    pub(super) count: u32,
    /// Whether the textures are sampled with filtering; otherwise they are bound as
    /// unfilterable, alongside a non-filtering sampler.
    pub(super) filterable: bool,
}

impl Inputs {
    pub(super) const NONE: Self = Self { count: 0, filterable: true };
}

impl Pass {
    /// Builds the pipeline for `source` with a uniform buffer and `inputs` textures (plus their
    /// sampler) bound in group 0.
    pub(super) fn new(
        device: &wgpu::Device,
        name: &str,
        source: String,
        inputs: Inputs,
        format: wgpu::TextureFormat,
    ) -> Self {
        let layout = bind_layout(device, name, inputs);
        let pipeline = pipeline(device, name, source, &layout, &[], format);
        Self { pipeline, layout }
    }
}

/// A uniform buffer and `inputs` textures (plus their sampler) in group 0.
fn bind_layout(device: &wgpu::Device, name: &str, inputs: Inputs) -> wgpu::BindGroupLayout {
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
    entries.extend((0..inputs.count).map(|i| wgpu::BindGroupLayoutEntry {
        binding: i + 1,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: inputs.filterable },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }));
    if inputs.count > 0 {
        let sampler = if inputs.filterable {
            wgpu::SamplerBindingType::Filtering
        } else {
            wgpu::SamplerBindingType::NonFiltering
        };
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: inputs.count + 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(sampler),
            count: None,
        });
    }
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(name), entries: &entries })
}

/// A fullscreen pipeline for `source` bound through `layout`; `constants` fill its overrides.
pub(super) fn pipeline(
    device: &wgpu::Device,
    name: &str,
    source: String,
    layout: &wgpu::BindGroupLayout,
    constants: &[(&str, f64)],
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(name),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(name),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    let options = wgpu::PipelineCompilationOptions { constants, ..Default::default() };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(name),
        layout: Some(&pl),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            compilation_options: options.clone(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_main"),
            compilation_options: options,
            targets: &[Some(format.into())],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// Clears `target` to black, then draws one fullscreen triangle with `pipeline` and `bind`.
pub(crate) fn draw_fullscreen(
    enc: &mut wgpu::CommandEncoder,
    target: &wgpu::TextureView,
    pipeline: &wgpu::RenderPipeline,
    bind: &wgpu::BindGroup,
    timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
) {
    let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        timestamp_writes,
        label: None,
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
        })],
        ..Default::default()
    });
    rp.set_pipeline(pipeline);
    rp.set_bind_group(0, bind, &[]);
    rp.draw(0..3, 0..1);
}
