use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const PASSES: u32 = 2;

/// GPU time spent in each pass of one frame, in milliseconds.
#[derive(Clone, Copy, Default)]
pub(crate) struct PassTimes {
    pub(crate) field: f32,
    pub(crate) shade: f32,
}

/// GPU timestamps around each pass, read back without stalling: a frame's stamps are copied
/// out only while the readback buffer is free, and picked up once its mapping completes.
pub(super) struct Timing {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    read: wgpu::Buffer,
    period_ns: f32,
    copied: bool,
    mapping: bool,
    ready: Arc<AtomicBool>,
    last: Option<PassTimes>,
}

impl Timing {
    /// `None` when the device was not created with timestamp queries.
    pub(super) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        device.features().contains(wgpu::Features::TIMESTAMP_QUERY).then(|| {
            let buf = |usage| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: 8 * 2 * u64::from(PASSES),
                    usage,
                    mapped_at_creation: false,
                })
            };
            Self {
                set: device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: None,
                    ty: wgpu::QueryType::Timestamp,
                    count: 2 * PASSES,
                }),
                resolve: buf(wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC),
                read: buf(wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ),
                period_ns: queue.get_timestamp_period(),
                copied: false,
                mapping: false,
                ready: Arc::new(AtomicBool::new(false)),
                last: None,
            }
        })
    }

    /// Timestamp writes that bracket render pass `pass`.
    pub(super) fn pass_writes(&self, pass: u32) -> wgpu::RenderPassTimestampWrites<'_> {
        wgpu::RenderPassTimestampWrites {
            query_set: &self.set,
            beginning_of_pass_write_index: Some(2 * pass),
            end_of_pass_write_index: Some(2 * pass + 1),
        }
    }

    /// Copies this frame's stamps toward the readback buffer, unless a mapping still holds it.
    pub(super) fn resolve(&mut self, enc: &mut wgpu::CommandEncoder) {
        if self.mapping {
            return;
        }
        enc.resolve_query_set(&self.set, 0..2 * PASSES, &self.resolve, 0);
        enc.copy_buffer_to_buffer(&self.resolve, 0, &self.read, 0, None);
        self.copied = true;
    }

    /// Call after submitting the encoder given to `resolve`: starts mapping the readback buffer.
    pub(super) fn start_readback(&mut self) {
        if self.copied && !self.mapping {
            self.copied = false;
            self.mapping = true;
            let ready = self.ready.clone();
            self.read.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                if r.is_ok() {
                    ready.store(true, Ordering::Release);
                }
            });
        }
    }

    /// Times of the most recent frame read back. With `wait`, blocks until the latest submitted
    /// readback lands; otherwise never stalls.
    pub(super) fn latest(&mut self, device: &wgpu::Device, wait: bool) -> Option<PassTimes> {
        if self.mapping {
            let poll = if wait { wgpu::PollType::wait_indefinitely() } else { wgpu::PollType::Poll };
            device.poll(poll).unwrap();
        }
        if self.ready.swap(false, Ordering::Acquire) {
            let ticks: Vec<u64> = bytemuck::cast_slice(&self.read.slice(..).get_mapped_range().unwrap()).to_vec();
            self.read.unmap();
            self.mapping = false;
            let ms = |i: usize| ticks[2 * i + 1].saturating_sub(ticks[2 * i]) as f32 * self.period_ns * 1e-6;
            self.last = Some(PassTimes { field: ms(0), shade: ms(1) });
        }
        self.last
    }
}
