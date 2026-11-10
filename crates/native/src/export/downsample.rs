use crate::cli::OutputSize;

fn srgb_to_linear(c: u8) -> f32 {
    let c = f32::from(c) / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5) as u8
}

/// Box-filters `factor` by `factor` blocks of RGBA bytes (`src_width` pixels per row) in linear light.
pub(super) fn downsample(src: &[u8], src_width: u32, factor: u32, out: OutputSize) -> image::RgbImage {
    let lut: Vec<f32> = (0..=255u8).map(srgb_to_linear).collect();
    let n = (factor * factor) as f32;
    image::RgbImage::from_fn(out.width, out.height, |x, y| {
        let mut acc = [0.0f32; 3];
        for sy in 0..factor {
            for sx in 0..factor {
                let i = (((y * factor + sy) * src_width + x * factor + sx) * 4) as usize;
                for (sum, &byte) in acc.iter_mut().zip(&src[i..i + 3]) {
                    *sum += lut[usize::from(byte)];
                }
            }
        }
        image::Rgb(acc.map(|a| linear_to_srgb(a / n)))
    })
}

#[cfg(test)]
mod tests {
    use assert2::check;

    use super::*;

    const BLACK: [u8; 4] = [0, 0, 0, 255];
    const WHITE: [u8; 4] = [255, 255, 255, 255];

    fn size(width: u32, height: u32) -> OutputSize {
        OutputSize { width, height }
    }

    #[test]
    fn a_block_is_averaged_in_linear_light() {
        let src = [BLACK, WHITE, WHITE, BLACK].concat();
        let image = downsample(&src, 2, 2, size(1, 1));
        let [r, g, b] = image.get_pixel(0, 0).0;
        check!([r, g, b] == [g; 3]);
        check!(g.abs_diff(188) <= 1);
    }

    #[test]
    fn factor_one_keeps_the_input_colours() {
        let colours = [[10, 200, 30, 255], BLACK, [255, 128, 7, 255]];
        let image = downsample(&colours.concat(), 3, 1, size(3, 1));
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
        let a = downsample(&opaque.concat(), 2, 2, size(1, 1));
        let b = downsample(&translucent.concat(), 2, 2, size(1, 1));
        check!(a == b);
    }

    #[test]
    fn every_byte_survives_the_transfer_functions() {
        check!(let None = (0..=255u8).find(|&v| linear_to_srgb(srgb_to_linear(v)) != v));
    }
}
