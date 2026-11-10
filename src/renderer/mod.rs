mod pass;
mod timing;

use std::io;
use std::path::{Path, PathBuf};

pub(crate) use pass::draw_fullscreen;
use pass::{Pass, read_shader};
use snafu::Snafu;
pub(crate) use timing::PassTimes;
use timing::Timing;

use crate::gpu::linear_sampler;

#[derive(Debug, Snafu)]
pub(crate) enum Error {
    #[snafu(display("could not read shader {}", path.display()))]
    ReadShader { path: PathBuf, source: io::Error },
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Uniforms {
    pub(crate) resolution: [f32; 2],
    pub(crate) time: f32,
    /// Shade pixels per field texel along each axis.
    pub(crate) field_scale: f32,
    pub(crate) view: [f32; 4],
    pub(crate) params: [[f32; 4]; 8],
}

const FIELD_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
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
pub(crate) struct Renderer {
    ubuf: wgpu::Buffer,
    field: Pass,
    shade: Pass,
    targets: Option<Targets>,
    sampler: wgpu::Sampler,
    timing: Option<Timing>,
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
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue, shader_dir: &Path) -> Result<Self, Error> {
        let common = read_shader(&shader_dir.join("common.wgsl"))?;
        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            ubuf,
            field: Pass::new(device, &common, shader_dir, "field", 0, FIELD_FORMAT)?,
            shade: Pass::new(device, &common, shader_dir, "shade", 1, COLOR_FORMAT)?,
            targets: None,
            sampler: linear_sampler(device),
            timing: Timing::new(device, queue),
        })
    }

    /// Call after submitting the encoder passed to `render`: starts reading back its timestamps.
    pub(crate) fn after_submit(&mut self) {
        if let Some(t) = self.timing.as_mut() {
            t.start_readback();
        }
    }

    /// Milliseconds in the field and shade passes of the most recent frame read back. With
    /// `wait`, blocks until the latest submitted readback lands; otherwise never stalls.
    pub(crate) fn pass_ms(&mut self, device: &wgpu::Device, wait: bool) -> Option<PassTimes> {
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

    /// Renders one frame into the internal colour texture; `uniforms.resolution` sets its size.
    /// Returns whether that texture was recreated, so views of `color` taken earlier are stale.
    pub(crate) fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        enc: &mut wgpu::CommandEncoder,
        uniforms: &Uniforms,
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
        draw_fullscreen(enc, &t.color_view, &self.shade.pipeline, &t.shade_bind, writes(1));
        if let Some(tm) = self.timing.as_mut() {
            tm.resolve(enc);
        }
        rebuilt
    }

    pub(crate) fn color(&self) -> &wgpu::Texture {
        &self.targets.as_ref().expect("colour target requested before the first render").color
    }

    pub(crate) fn color_view(&self) -> &wgpu::TextureView {
        &self.targets.as_ref().expect("colour target requested before the first render").color_view
    }
}
