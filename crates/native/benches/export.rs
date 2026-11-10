//! The CPU half of an export: filtering a supersampled frame down and encoding the result.

use currents::export::downsample::{Rows, downsample};
use currents::export::encode::{Animation, Encoding, Format, encode};
use divan::Bencher;
use divan::counter::BytesCount;
use image::RgbImage;

/// Output frame size along each axis.
const SIZE: u32 = 1600;

fn main() {
    divan::main();
}

/// A picture with the renderer's mix of flat bands, sharp edges and fine grain, `shift` pixels along.
fn pixel(x: u32, y: u32, shift: u32) -> [u8; 3] {
    let band = ((x + shift) / 3 + y / 7 + ((x + shift) * y / 4096) % 5) % 23 < 12;
    let grain = (x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA6B)).wrapping_mul(0xC2B2_AE35) >> 27;
    if band { [150 + grain as u8, 120 + grain as u8, 170 + grain as u8] } else { [14 + (grain / 8) as u8, 11, 15] }
}

fn shaded(size: u32) -> Vec<u8> {
    (0..size * size).flat_map(|i| pixel(i % size, i / size, 0).into_iter().chain([255])).collect()
}

fn frame(shift: u32) -> RgbImage {
    RgbImage::from_fn(SIZE, SIZE, |x, y| image::Rgb(pixel(x, y, shift)))
}

const fn encoding(format: Format, lossless: bool) -> Encoding {
    Encoding { format, quality: 90, lossless }
}

#[divan::bench(args = [1, 2, 3])]
fn downsample_frame(bencher: Bencher<'_, '_>, factor: u32) {
    let src = shaded(SIZE * factor);
    bencher
        .counter(BytesCount::new(src.len()))
        .bench_local(|| downsample(Rows::packed(divan::black_box(&src), SIZE * factor), factor));
}

#[divan::bench(args = [(Format::Png, false), (Format::Jpeg, false), (Format::Webp, false), (Format::Webp, true)])]
fn encode_still(bencher: Bencher<'_, '_>, (format, lossless): (Format, bool)) {
    let image = frame(0);
    bencher
        .counter(BytesCount::of_slice(image.as_raw()))
        .bench_local(|| encode(divan::black_box(&image), encoding(format, lossless)));
}

#[divan::bench(args = [false, true], sample_count = 10)]
fn encode_animation_frame(bencher: Bencher<'_, '_>, lossless: bool) {
    let frames = [frame(0), frame(2), frame(4), frame(6)];
    bencher.bench_local(|| {
        let mut animation = Animation::new(SIZE, SIZE, 30.0, encoding(Format::Webp, lossless))?;
        for image in &frames {
            animation.push(divan::black_box(image))?;
        }
        animation.finish()
    });
}
