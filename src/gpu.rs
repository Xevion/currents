use snafu::{ResultExt, Snafu};
use tracing::info;

pub(crate) struct Gpu {
    pub(crate) adapter: wgpu::Adapter,
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
}

#[derive(Debug, Snafu)]
pub(crate) enum Error {
    #[snafu(display("no suitable graphics adapter"))]
    NoAdapter { source: wgpu::RequestAdapterError },
    #[snafu(display("could not open a device on {adapter}"))]
    OpenDevice { adapter: String, source: wgpu::RequestDeviceError },
}

/// Requests the high-performance adapter (compatible with `surface`, if given) and a device on it.
pub(crate) fn request(instance: &wgpu::Instance, surface: Option<&wgpu::Surface<'_>>) -> Result<Gpu, Error> {
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: surface,
        ..Default::default()
    }))
    .context(NoAdapterSnafu)?;
    let details = adapter.get_info();
    let (device, queue) = pollster::block_on(adapter.request_device(&device_desc(&adapter)))
        .context(OpenDeviceSnafu { adapter: details.name.clone() })?;
    info!(
        adapter = %details.name,
        backend = ?details.backend,
        timestamp_queries = device.features().contains(wgpu::Features::TIMESTAMP_QUERY),
        "gpu ready"
    );
    Ok(Gpu { adapter, device, queue })
}

/// Device request that turns on GPU timestamps where the adapter has them.
fn device_desc(adapter: &wgpu::Adapter) -> wgpu::DeviceDescriptor<'static> {
    wgpu::DeviceDescriptor {
        required_features: wgpu::Features::FLOAT32_FILTERABLE | (adapter.features() & wgpu::Features::TIMESTAMP_QUERY),
        ..Default::default()
    }
}

/// Bilinear sampler with the default edge handling.
pub(crate) fn linear_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    })
}
