use std::sync::mpsc;

use snafu::ResultExt;

use super::{Error, ReadbackSnafu};
use crate::cli::OutputSize;

/// A mappable buffer that receives a rendered texture, one padded row at a time.
pub(super) struct Readback {
    buffer: wgpu::Buffer,
    size: OutputSize,
    row_pitch: u32,
}

/// The readback buffer while mapped for reading; unmaps on drop.
pub(super) struct Mapped<'a> {
    readback: &'a Readback,
}

impl Readback {
    pub(super) fn new(device: &wgpu::Device, size: OutputSize) -> Self {
        let row_pitch = (size.width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(row_pitch * size.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self { buffer, size, row_pitch }
    }

    /// Records a copy of the whole `texture`, which must match the size given to `new`.
    pub(super) fn copy_from(&self, enc: &mut wgpu::CommandEncoder, texture: &wgpu::Texture) {
        enc.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.row_pitch),
                    rows_per_image: Some(self.size.height),
                },
            },
            wgpu::Extent3d { width: self.size.width, height: self.size.height, depth_or_array_layers: 1 },
        );
    }

    /// Blocks until every submitted copy has landed and the buffer is mapped.
    pub(super) fn map(&self, device: &wgpu::Device) -> Result<Mapped<'_>, Error> {
        let (sender, receiver) = mpsc::channel();
        self.buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        device.poll(wgpu::PollType::wait_indefinitely()).expect("polling without a submission index cannot fail");
        receiver.recv().expect("the map callback runs before an indefinite poll returns").context(ReadbackSnafu)?;
        Ok(Mapped { readback: self })
    }
}

impl Mapped<'_> {
    /// The pixels as tightly packed rows of RGBA bytes.
    pub(super) fn pixels(self) -> Vec<u8> {
        let Readback { buffer, size, row_pitch } = self.readback;
        let view = buffer.slice(..).get_mapped_range().expect("readback buffer is mapped while `Mapped` lives");
        view.chunks_exact(*row_pitch as usize).flat_map(|row| &row[..size.width as usize * 4]).copied().collect()
    }
}

impl Drop for Mapped<'_> {
    fn drop(&mut self) {
        self.readback.buffer.unmap();
    }
}
