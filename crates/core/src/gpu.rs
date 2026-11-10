use snafu::{ResultExt, Snafu, ensure};
use tracing::info;

use crate::renderer::FIELD_FORMAT;

pub struct Gpu {
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("no suitable graphics adapter"))]
    NoAdapter { source: wgpu::RequestAdapterError },
    #[snafu(display("{adapter} cannot render to 32-bit float textures, which the flow field is stored in"))]
    NoFloatTargets { adapter: String },
    #[snafu(display("could not open a device on {adapter}"))]
    OpenDevice { adapter: String, source: wgpu::RequestDeviceError },
}

/// Requests the high-performance adapter (compatible with `surface`, if given) and a device on it.
#[cfg_attr(
    target_arch = "wasm32",
    expect(clippy::future_not_send, reason = "browser GPU handles stay on the page thread")
)]
pub async fn request(instance: &wgpu::Instance, surface: Option<&wgpu::Surface<'_>>) -> Result<Gpu, Error> {
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: surface,
            ..Default::default()
        })
        .await
        .context(NoAdapterSnafu)?;
    let details = adapter.get_info();
    let field = adapter.get_texture_format_features(FIELD_FORMAT).allowed_usages;
    ensure!(
        field.contains(wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING),
        NoFloatTargetsSnafu { adapter: details.name.clone() }
    );
    let (device, queue) = adapter
        .request_device(&device_desc(&adapter))
        .await
        .context(OpenDeviceSnafu { adapter: details.name.clone() })?;
    info!(
        adapter = %details.name,
        backend = ?details.backend,
        float32_filterable = device.features().contains(wgpu::Features::FLOAT32_FILTERABLE),
        timestamp_queries = device.features().contains(wgpu::Features::TIMESTAMP_QUERY),
        "gpu ready"
    );
    Ok(Gpu { adapter, device, queue })
}

/// Turns on filtered float sampling and GPU timestamps where the adapter has them, and asks for
/// no more than the WebGL2 floor on GL (which browsers emulate) or the downlevel floor elsewhere.
fn device_desc(adapter: &wgpu::Adapter) -> wgpu::DeviceDescriptor<'static> {
    let floor = if adapter.get_info().backend == wgpu::Backend::Gl {
        wgpu::Limits::downlevel_webgl2_defaults()
    } else {
        wgpu::Limits::downlevel_defaults()
    };
    wgpu::DeviceDescriptor {
        required_features: adapter.features() & (wgpu::Features::FLOAT32_FILTERABLE | wgpu::Features::TIMESTAMP_QUERY),
        required_limits: floor.using_resolution(adapter.limits()),
        ..Default::default()
    }
}

/// Bilinear sampler with the default edge handling.
pub fn linear_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    })
}
