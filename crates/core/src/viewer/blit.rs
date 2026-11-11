use crate::gpu::linear_sampler;
use crate::renderer::draw_fullscreen;

/// Scales the renderer's colour texture over the whole of a target of any size.
pub(crate) struct Blit {
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    bind: Option<wgpu::BindGroup>,
}

impl Blit {
    pub(crate) fn new(device: &wgpu::Device, target: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blit"),
            source: wgpu::ShaderSource::Wgsl(include_str!("blit.wgsl").into()),
        });
        let options = wgpu::PipelineCompilationOptions {
            constants: &[("to_linear", f64::from(target.is_srgb()))],
            ..Default::default()
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit"),
            layout: None,
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
                targets: &[Some(target.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline, sampler: linear_sampler(device), bind: None }
    }

    /// Points the blit at `source`; call again whenever the texture behind the view is replaced.
    pub(crate) fn bind(&mut self, device: &wgpu::Device, source: &wgpu::TextureView) {
        self.bind = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(source) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        }));
    }

    pub(crate) fn draw(&self, enc: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let bind = self.bind.as_ref().expect("blit drawn before being bound to a source");
        draw_fullscreen(enc, target, &self.pipeline, bind, None);
    }
}

#[cfg(test)]
mod tests {
    use crate::shader_check::{to_webgl, validate};

    #[test]
    fn blit_shader_validates_and_translates_for_webgl() {
        validate("blit", include_str!("blit.wgsl"));
        to_webgl("blit", include_str!("blit.wgsl"), &[("to_linear", 0.0)]);
    }
}
