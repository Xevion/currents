use std::sync::LazyLock;

use rayon::prelude::*;

/// Rows of RGBA pixels as a GPU readback lays them out: `pitch` bytes apart, of which the
/// first `width` pixels are the picture and the rest padding.
#[derive(Clone, Copy)]
pub struct Rows<'a> {
    pub bytes: &'a [u8],
    pub pitch: usize,
    pub width: u32,
}

impl<'a> Rows<'a> {
    /// Rows with no padding between them.
    pub fn packed(bytes: &'a [u8], width: u32) -> Self {
        Self { bytes, pitch: width as usize * 4, width }
    }

    fn count(&self) -> usize {
        self.bytes.len().checked_div(self.pitch).unwrap_or(0)
    }
}

fn srgb_to_linear(c: u8) -> f32 {
    let c = f32::from(c) / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5) as u8
}

/// Bins the unit interval is cut into to find a first guess at the sRGB byte. They are
/// narrower than the closest two bytes lie apart, so the guess is at most one byte short.
const BINS: usize = 4096;

/// Both transfer functions as tables, giving exactly the bytes the functions above do.
struct Transfer {
    linear: [f32; 256],
    /// The smallest linear value that encodes to each byte or above; nothing reaches the last.
    floor: [f32; 257],
    /// The byte the lower edge of each bin encodes to.
    guess: [u8; BINS + 1],
}

impl Transfer {
    fn new() -> Self {
        let mut floor = [f32::INFINITY; 257];
        floor[0] = 0.0;
        for (byte, floor) in floor.iter_mut().enumerate().take(256).skip(1) {
            // the encoding never decreases, so the bit patterns of positive floats can be bisected
            let (mut below, mut at) = (0.0f32.to_bits(), 1.0f32.to_bits());
            while at - below > 1 {
                let mid = below + (at - below) / 2;
                if usize::from(linear_to_srgb(f32::from_bits(mid))) >= byte {
                    at = mid;
                } else {
                    below = mid;
                }
            }
            *floor = f32::from_bits(at);
        }
        Self {
            linear: std::array::from_fn(|byte| srgb_to_linear(byte as u8)),
            floor,
            guess: std::array::from_fn(|bin| linear_to_srgb(bin as f32 / BINS as f32)),
        }
    }

    #[inline]
    fn to_srgb(&self, c: f32) -> u8 {
        let c = c.clamp(0.0, 1.0);
        let byte = self.guess[(c * BINS as f32) as usize];
        // a comparison rather than a loop: whether the guess is short is close to a coin toss
        byte + u8::from(c >= self.floor[usize::from(byte) + 1])
    }
}

static TRANSFER: LazyLock<Transfer> = LazyLock::new(Transfer::new);

/// Box-filters `factor` by `factor` blocks of `src` in linear light, one output row per task.
///
/// The output is `src` divided by `factor` along each axis, rounded down.
pub fn downsample(src: Rows<'_>, factor: u32) -> image::RgbImage {
    let transfer = &*TRANSFER;
    let factor = factor.max(1) as usize;
    let (width, height) = (src.width as usize / factor, src.count() / factor);
    let mut out = vec![0u8; width * height * 3];
    let samples = (factor * factor) as f32;
    if width > 0 {
        out.par_chunks_mut(width * 3).enumerate().for_each_init(
            || vec![0.0f32; width * 3],
            |sums, (y, row)| {
                sums.fill(0.0);
                for sy in 0..factor {
                    let line = &src.bytes[(y * factor + sy) * src.pitch..][..width * factor * 4];
                    for (sum, block) in sums.as_chunks_mut::<3>().0.iter_mut().zip(line.chunks_exact(factor * 4)) {
                        for pixel in block.as_chunks::<4>().0 {
                            sum[0] += transfer.linear[usize::from(pixel[0])];
                            sum[1] += transfer.linear[usize::from(pixel[1])];
                            sum[2] += transfer.linear[usize::from(pixel[2])];
                        }
                    }
                }
                for (byte, sum) in row.iter_mut().zip(sums.iter()) {
                    *byte = transfer.to_srgb(sum / samples);
                }
            },
        );
    }
    image::RgbImage::from_raw(width as u32, height as u32, out).expect("the buffer was sized for the image")
}

#[cfg(test)]
mod tests {
    use assert2::check;

    use super::*;

    const BLACK: [u8; 4] = [0, 0, 0, 255];
    const WHITE: [u8; 4] = [255, 255, 255, 255];

    #[test]
    fn a_block_is_averaged_in_linear_light() {
        let src = [BLACK, WHITE, WHITE, BLACK].concat();
        let image = downsample(Rows::packed(&src, 2), 2);
        check!(image.dimensions() == (1, 1));
        let [r, g, b] = image.get_pixel(0, 0).0;
        check!([r, g, b] == [g; 3]);
        check!(g.abs_diff(188) <= 1);
    }

    #[test]
    fn factor_one_keeps_the_input_colours() {
        let colours = [[10, 200, 30, 255], BLACK, [255, 128, 7, 255]];
        let image = downsample(Rows::packed(&colours.concat(), 3), 1);
        let got: Vec<[u8; 3]> = image.pixels().map(|p| p.0).collect();
        check!(got == [[10, 200, 30], [0, 0, 0], [255, 128, 7]]);
    }

    #[test]
    fn alpha_does_not_affect_the_result() {
        let opaque = [[200, 40, 90, 255], [10, 10, 10, 255], [0, 255, 0, 255], [90, 90, 255, 255]];
        let mut translucent = opaque;
        for (pixel, alpha) in translucent.iter_mut().zip([0, 64, 128, 7]) {
            pixel[3] = alpha;
        }
        let a = downsample(Rows::packed(&opaque.concat(), 2), 2);
        let b = downsample(Rows::packed(&translucent.concat(), 2), 2);
        check!(a == b);
    }

    #[test]
    fn padding_after_each_row_is_skipped() {
        let pad = [77u8; 5];
        let padded = [&WHITE[..], &BLACK[..], &pad[..], &BLACK[..], &WHITE[..], &pad[..]].concat();
        let packed = [WHITE, BLACK, BLACK, WHITE].concat();
        let rows = Rows { bytes: &padded, pitch: 13, width: 2 };
        check!(downsample(rows, 1) == downsample(Rows::packed(&packed, 2), 1));
        check!(downsample(rows, 2) == downsample(Rows::packed(&packed, 2), 2));
    }

    #[test]
    fn every_byte_survives_the_transfer_functions() {
        check!(let None = (0..=255u8).find(|&v| linear_to_srgb(srgb_to_linear(v)) != v));
    }

    #[test]
    fn the_tables_encode_exactly_as_the_transfer_function_does() {
        let transfer = &*TRANSFER;
        let sweep = (0..=2_000_000u32).map(|i| i as f32 / 2_000_000.0);
        // the values either side of every point where the byte changes
        let edges = transfer.floor[..256].iter().flat_map(|&c| [f32::from_bits(c.to_bits().saturating_sub(1)), c]);
        let outside = [-1.0, 1.5, f32::MIN_POSITIVE];
        check!(let None = sweep.chain(edges).chain(outside).find(|&c| transfer.to_srgb(c) != linear_to_srgb(c)));
    }
}
