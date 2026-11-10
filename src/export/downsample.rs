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
