pub(crate) struct Gpu {
    pub(crate) adapter: wgpu::Adapter,
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
}

/// Requests the high-performance adapter (compatible with `surface`, if given) and a device on it.
pub(crate) fn request(instance: &wgpu::Instance, surface: Option<&wgpu::Surface<'_>>) -> Gpu {
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: surface,
        ..Default::default()
    }))
    .expect("adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&device_desc(&adapter))).expect("device");
    Gpu { adapter, device, queue }
}

/// Device request that turns on GPU timestamps where the adapter has them.
fn device_desc(adapter: &wgpu::Adapter) -> wgpu::DeviceDescriptor<'static> {
    wgpu::DeviceDescriptor {
        required_features: wgpu::Features::FLOAT32_FILTERABLE | (adapter.features() & wgpu::Features::TIMESTAMP_QUERY),
        ..Default::default()
    }
}
