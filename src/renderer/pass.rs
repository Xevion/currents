use std::path::Path;

pub(super) struct Pass {
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) layout: wgpu::BindGroupLayout,
}

impl Pass {
    /// Builds the pipeline for `dir/name.wgsl` appended to `common`, with a uniform buffer and
    /// `inputs` filterable textures (plus their sampler) bound in group 0.
    pub(super) fn new(
        device: &wgpu::Device,
        common: &str,
        dir: &Path,
        name: &str,
        inputs: u32,
        format: wgpu::TextureFormat,
    ) -> Self {
        let src =
            std::fs::read_to_string(dir.join(format!("{name}.wgsl"))).unwrap_or_else(|e| panic!("{name}.wgsl: {e}"));
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
        Self { pipeline, layout }
    }
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
