//! The pass that blends each frame into the ones before it, and the textures it keeps.

use super::{COLOR_FORMAT, draw_fullscreen, make_tex};
use crate::gpu::linear_sampler;
use crate::shaders::Sources;

/// The blend so far. Finer than the colour it is shown at, so that a frame's small share is
/// not rounded away.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

pub(super) fn source(sources: &Sources) -> String {
    sources.assemble(include_str!("resolve.wgsl"))
}

/// Where in the pixel successive frames shade, as shifts from its centre: four points spread
/// evenly over it, in an order that makes every step from one to the next the same length. A
/// longer step than the rest would show as a pulse each time the cycle came round to it.
const POINTS: [[f32; 2]; 4] = [[-0.125, -0.375], [0.375, -0.125], [0.125, 0.375], [-0.375, 0.125]];

/// Where in the pixel frame `frame` shades, as a shift from its centre.
pub(super) fn jitter(frame: u32) -> [f32; 2] {
    POINTS[frame as usize % POINTS.len()]
}

pub(super) struct Resolve {
    pipeline: wgpu::RenderPipeline,
    /// Draws the blend, sharpened, as the picture.
    show: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

/// What one size of picture needs: the frame as shaded, and two blends that take turns being
/// read and written.
pub(super) struct History {
    pub(super) shaded: wgpu::TextureView,
    kept: [wgpu::TextureView; 2],
    binds: [wgpu::BindGroup; 2],
    /// Which blend the next frame reads.
    read: usize,
}

impl Resolve {
    pub(super) fn new(device: &wgpu::Device, sources: &Sources) -> Self {
        let texture = |binding, filterable| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("resolve"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture(3, true),
                texture(4, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // the field is only ever loaded texel by texel, which every device allows
                texture(6, false),
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("resolve"),
            source: wgpu::ShaderSource::Wgsl(source(sources).into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("resolve"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry, format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
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
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(format.into())],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        Self {
            pipeline: pipeline("fs_main", FORMAT),
            show: pipeline("fs_show", COLOR_FORMAT),
            layout,
            sampler: linear_sampler(device),
        }
    }

    /// Textures and bindings for a picture `width` by `height`, whose field is `field`.
    pub(super) fn history(
        &self,
        device: &wgpu::Device,
        uniforms: &wgpu::Buffer,
        field: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> History {
        let view = |format| make_tex(device, width, height, format).create_view(&Default::default());
        let shaded = view(COLOR_FORMAT);
        let kept = [view(FORMAT), view(FORMAT)];
        let binds = std::array::from_fn(|read| {
            let texture =
                |binding, view| wgpu::BindGroupEntry { binding, resource: wgpu::BindingResource::TextureView(view) };
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() },
                    texture(3, &shaded),
                    texture(4, &kept[read]),
                    texture(6, field),
                    wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                ],
            })
        });
        History { shaded, kept, binds, read: 0 }
    }

    /// Blends the frame in `history.shaded` into the blend so far, then draws that to `shown`.
    pub(super) fn draw(
        &self,
        enc: &mut wgpu::CommandEncoder,
        history: &mut History,
        shown: &wgpu::TextureView,
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let written = history.read ^ 1;
        draw_fullscreen(enc, &history.kept[written], &self.pipeline, &history.binds[history.read], None);
        // the bindings that read the blend just written are the other turn's
        draw_fullscreen(enc, shown, &self.show, &history.binds[written], timestamp_writes);
        history.read = written;
    }
}

#[cfg(test)]
mod tests {
    use assert2::check;

    use super::*;

    #[test]
    fn shading_points_centre_on_the_pixel_and_step_evenly_round_the_cycle() {
        let count = POINTS.len() as u32;
        let steps: Vec<f32> = (0..count)
            .map(|frame| {
                let (a, b) = (jitter(frame), jitter(frame + 1));
                (a[0] - b[0]).hypot(a[1] - b[1])
            })
            .collect();
        check!(steps.iter().all(|step| (step - steps[0]).abs() < 1e-6));
        for axis in 0..2 {
            check!(POINTS.iter().map(|p| p[axis]).sum::<f32>().abs() < 1e-6);
        }
    }
}
