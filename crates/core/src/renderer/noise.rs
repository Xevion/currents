use super::pass::{draw_fullscreen, pipeline};
use crate::shaders::Sources;

/// Texels across the finest level of the noise tile: sixteen samples to each lattice cell.
const TEXELS: u32 = 1024;
/// Levels of the tile, each half as fine as the one before; the coarsest keeps two samples to
/// a cell, below which the noise would lose its shape.
const LEVELS: u32 = 4;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

pub(super) fn source(sources: &Sources) -> String {
    sources.assemble(include_str!("noise.wgsl"))
}

/// Renders the gradient noise the passes sample into a texture, once. The finest level is the
/// noise itself; each one after averages it over its texels, as a pixel that wide would see it.
pub(super) fn bake(device: &wgpu::Device, queue: &wgpu::Queue, sources: &Sources) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("noise"),
        size: wgpu::Extent3d { width: TEXELS, height: TEXELS, depth_or_array_layers: 1 },
        mip_level_count: LEVELS,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("noise"), entries: &[] });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &layout, entries: &[] });
    let mut enc = device.create_command_encoder(&Default::default());
    for level in 0..LEVELS {
        let target = texture.create_view(&wgpu::TextureViewDescriptor {
            base_mip_level: level,
            mip_level_count: Some(1),
            ..Default::default()
        });
        let constants = [("texels", f64::from(TEXELS >> level)), ("taps", f64::from(1u32 << level))];
        let pipeline = pipeline(device, "noise", source(sources), "fs_main", &layout, &constants, FORMAT);
        draw_fullscreen(&mut enc, &target, &pipeline, &bind, None);
    }
    queue.submit([enc.finish()]);
    texture.create_view(&Default::default())
}

/// Sampler that wraps, so the tile repeats, and blends between its levels.
pub(super) fn sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    })
}
