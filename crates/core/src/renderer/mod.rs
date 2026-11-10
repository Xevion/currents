mod debug;
mod pass;
mod timing;

pub use debug::{FieldView, UnknownView};
pub(crate) use pass::draw_fullscreen;
use pass::{Inputs, Pass};
pub use timing::PassTimes;
use timing::Timing;

use crate::gpu::linear_sampler;
use crate::params::ParamBlock;
use crate::shaders::Sources;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Uniforms {
    pub resolution: [f32; 2],
    pub time: f32,
    /// Shade pixels per field texel along each axis.
    pub field_scale: f32,
    pub view: [f32; 4],
    pub params: ParamBlock,
}

pub(crate) const FIELD_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Shade resolution in pixels and the supersampling factor; the field texture is derived from both.
#[derive(Clone, Copy, PartialEq, Eq)]
struct TargetSize {
    width: u32,
    height: u32,
    field_scale: u32,
}

struct Targets {
    size: TargetSize,
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
    /// One pipeline per `FieldView`, in its order, bound like the shade pass.
    debug: Vec<wgpu::RenderPipeline>,
    targets: Option<Targets>,
    sampler: wgpu::Sampler,
    timing: Option<Timing>,
}

/// The shade pass with the `sample_field` the device can run appended. The two are separate
/// sources because even a constant branch between them changes the driver's codegen.
fn shade_source(sources: &Sources, filterable: bool) -> String {
    let sampler =
        if filterable { include_str!("sample_filtered.wgsl") } else { include_str!("sample_unfiltered.wgsl") };
    sources.assemble(&format!("{}\n{sampler}", sources.shade))
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
    /// Without `FLOAT32_FILTERABLE` (common on WebGL and phones) the shade pass filters the field itself.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, sources: &Sources) -> Self {
        let filterable = device.features().contains(wgpu::Features::FLOAT32_FILTERABLE);
        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let field = Pass::new(device, "field", sources.assemble(&sources.field), Inputs::NONE, FIELD_FORMAT);
        let shade = Pass::new(
            device,
            "shade",
            shade_source(sources, filterable),
            Inputs { count: 1, filterable },
            COLOR_FORMAT,
        );
        let debug_source = sources.assemble(include_str!("debug.wgsl"));
        let debug = FieldView::ALL
            .map(|view| {
                let constants = [("mode", f64::from(view.mode()))];
                pass::pipeline(device, view.name(), debug_source.clone(), &shade.layout, &constants, COLOR_FORMAT)
            })
            .to_vec();
        let sampler = if filterable {
            linear_sampler(device)
        } else {
            device.create_sampler(&wgpu::SamplerDescriptor::default())
        };
        Self { ubuf, field, shade, debug, targets: None, sampler, timing: Timing::new(device, queue) }
    }

    /// Call after submitting the encoder passed to `render`: starts reading back its timestamps.
    pub fn after_submit(&mut self) {
        if let Some(t) = self.timing.as_mut() {
            t.start_readback();
        }
    }

    /// Milliseconds in the field and shade passes of the most recent frame read back. With
    /// `wait`, blocks until the latest submitted readback lands; otherwise never stalls.
    pub fn pass_ms(&mut self, device: &wgpu::Device, wait: bool) -> Option<PassTimes> {
        self.timing.as_mut()?.latest(device, wait)
    }

    /// Returns whether the targets were recreated, which invalidates views taken from the old ones.
    fn ensure_targets(&mut self, device: &wgpu::Device, size: TargetSize) -> bool {
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return false;
        }
        let TargetSize { width, height, field_scale } = size;
        let field_view = make_tex(device, width.div_ceil(field_scale), height.div_ceil(field_scale), FIELD_FORMAT)
            .create_view(&Default::default());
        let color = make_tex(device, width, height, COLOR_FORMAT);
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
        self.targets = Some(Targets { size, field_view, color, color_view, field_bind, shade_bind });
        true
    }

    /// Renders one frame into the internal colour texture; `uniforms.resolution` sets its size,
    /// and `view` replaces the shaded picture with a view of the field. Returns whether that
    /// texture was recreated, so views of `color` taken earlier are stale.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        enc: &mut wgpu::CommandEncoder,
        uniforms: &Uniforms,
        view: Option<FieldView>,
    ) -> bool {
        let size = TargetSize {
            width: uniforms.resolution[0] as u32,
            height: uniforms.resolution[1] as u32,
            field_scale: uniforms.field_scale.max(1.0) as u32,
        };
        let rebuilt = self.ensure_targets(device, size);
        queue.write_buffer(&self.ubuf, 0, bytemuck::bytes_of(uniforms));
        let t = self.targets.as_ref().expect("ensure_targets left the targets unset");
        let writes = |pass| self.timing.as_ref().map(|tm| tm.pass_writes(pass));
        draw_fullscreen(enc, &t.field_view, &self.field.pipeline, &t.field_bind, writes(0));
        let colour = view.map_or(&self.shade.pipeline, |v| &self.debug[v as usize]);
        draw_fullscreen(enc, &t.color_view, colour, &t.shade_bind, writes(1));
        if let Some(tm) = self.timing.as_mut() {
            tm.resolve(enc);
        }
        rebuilt
    }

    pub fn color(&self) -> &wgpu::Texture {
        &self.targets.as_ref().expect("colour target requested before the first render").color
    }

    pub fn color_view(&self) -> &wgpu::TextureView {
        &self.targets.as_ref().expect("colour target requested before the first render").color_view
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    use crate::shader_check::{to_webgl, validate};

    #[test]
    fn field_shader_validates_and_translates_for_webgl() {
        let sources = Sources::embedded();
        let source = sources.assemble(&sources.field);
        validate("field", &source);
        to_webgl("field", &source, &[]);
    }

    #[rstest]
    fn shade_shader_validates_and_translates_for_webgl(#[values(true, false)] filterable: bool) {
        let source = shade_source(&Sources::embedded(), filterable);
        validate("shade", &source);
        to_webgl("shade", &source, &[]);
    }

    #[test]
    fn debug_views_validate_and_translate_for_webgl() {
        let source = Sources::embedded().assemble(include_str!("debug.wgsl"));
        validate("debug", &source);
        for view in FieldView::ALL {
            to_webgl(view.name(), &source, &[("mode", f64::from(view.mode()))]);
        }
    }
}
