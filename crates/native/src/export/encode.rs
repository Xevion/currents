use std::fmt;
use std::path::Path;

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder, RgbImage};
use snafu::{ResultExt, Snafu};

/// An image file format the renderer can write.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    Png,
    Jpeg,
    Webp,
}

impl Format {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
        }
    }

    /// The format `path`'s extension names, if it is one of these.
    pub fn from_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            "png" => Some(Self::Png),
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "webp" => Some(Self::Webp),
            _ => None,
        }
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Png => "PNG",
            Self::Jpeg => "JPEG",
            Self::Webp => "WebP",
        })
    }
}

/// A format and the settings its encoder takes.
#[derive(Clone, Copy, Debug)]
pub struct Encoding {
    pub format: Format,
    /// Quality of lossy JPEG and WebP, 0 to 100.
    pub quality: u8,
    /// Whether WebP keeps every pixel exactly; PNG always does and JPEG never.
    pub lossless: bool,
}

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("the PNG encoder failed"))]
    Png { source: image::ImageError },
    #[snafu(display("the JPEG encoder failed"))]
    Jpeg { source: image::ImageError },
    #[snafu(display("the WebP encoder failed: {message}"))]
    Webp { message: String },
}

fn webp<T>(result: webpx::Result<T>) -> Result<T, Error> {
    result.map_err(|error| Error::Webp { message: error.to_string() })
}

/// Encodes `image` as one still picture.
pub fn encode(image: &RgbImage, encoding: Encoding) -> Result<Vec<u8>, Error> {
    let (width, height) = image.dimensions();
    let mut out = Vec::new();
    match encoding.format {
        Format::Png => PngEncoder::new(&mut out)
            .write_image(image.as_raw(), width, height, ExtendedColorType::Rgb8)
            .context(PngSnafu)?,
        Format::Jpeg => JpegEncoder::new_with_quality(&mut out, encoding.quality)
            .write_image(image.as_raw(), width, height, ExtendedColorType::Rgb8)
            .context(JpegSnafu)?,
        Format::Webp => {
            out = webp(
                webpx::Encoder::new_rgb(image.as_raw(), width, height)
                    .quality(f32::from(encoding.quality))
                    .lossless(encoding.lossless)
                    .encode(webpx::Unstoppable),
            )?;
        },
    }
    Ok(out)
}

/// An animated WebP, encoded as its frames arrive.
pub struct Animation {
    encoder: webpx::AnimationEncoder,
    fps: f32,
    frames: u32,
}

impl Animation {
    /// An animation of `width` by `height` frames shown `fps` to the second.
    pub fn new(width: u32, height: u32, fps: f32, encoding: Encoding) -> Result<Self, Error> {
        let mut encoder = webp(webpx::AnimationEncoder::new(width, height))?;
        if encoding.lossless {
            encoder.set_lossless(true);
        } else {
            encoder.set_quality(f32::from(encoding.quality));
        }
        Ok(Self { encoder, fps, frames: 0 })
    }

    /// Milliseconds into the animation at which the next frame starts.
    fn timestamp(&self) -> i32 {
        (f64::from(self.frames) * 1000.0 / f64::from(self.fps)).round() as i32
    }

    /// Appends `image`, which must have the animation's size.
    pub fn push(&mut self, image: &RgbImage) -> Result<(), Error> {
        webp(self.encoder.add_frame_rgb(image.as_raw(), self.timestamp()))?;
        self.frames += 1;
        Ok(())
    }

    pub fn finish(self) -> Result<Vec<u8>, Error> {
        let end = self.timestamp();
        webp(self.encoder.finish(end))
    }
}

#[cfg(test)]
mod tests {
    use assert2::{assert, check};
    use rstest::rstest;

    use super::*;

    fn gradient(width: u32, height: u32, shift: u32) -> RgbImage {
        RgbImage::from_fn(width, height, |x, y| image::Rgb([(x * 8 + shift) as u8, (y * 8) as u8, 90]))
    }

    #[rstest]
    #[case::png("a/b.png", Some(Format::Png))]
    #[case::jpg("frame.jpg", Some(Format::Jpeg))]
    #[case::jpeg_upper("frame.JPEG", Some(Format::Jpeg))]
    #[case::webp("anim.webp", Some(Format::Webp))]
    #[case::directory("out/frames", None)]
    #[case::other("frame.gif", None)]
    fn the_extension_names_the_format(#[case] path: &str, #[case] format: Option<Format>) {
        check!(Format::from_path(Path::new(path)) == format);
    }

    #[rstest]
    #[case::png(Format::Png, false, image::ImageFormat::Png)]
    #[case::webp(Format::Webp, true, image::ImageFormat::WebP)]
    fn lossless_stills_decode_to_the_same_pixels(
        #[case] format: Format,
        #[case] lossless: bool,
        #[case] container: image::ImageFormat,
    ) {
        let image = gradient(32, 24, 0);
        assert!(let Ok(bytes) = encode(&image, Encoding { format, quality: 90, lossless }));
        assert!(let Ok(decoded) = image::load_from_memory_with_format(&bytes, container));
        check!(decoded.to_rgb8() == image);
    }

    #[rstest]
    #[case::jpeg(Format::Jpeg, image::ImageFormat::Jpeg)]
    #[case::webp(Format::Webp, image::ImageFormat::WebP)]
    fn lossy_stills_keep_the_size(#[case] format: Format, #[case] container: image::ImageFormat) {
        let image = gradient(32, 24, 0);
        assert!(let Ok(bytes) = encode(&image, Encoding { format, quality: 80, lossless: false }));
        assert!(let Ok(decoded) = image::load_from_memory_with_format(&bytes, container));
        check!((decoded.width(), decoded.height()) == (32, 24));
    }

    #[test]
    fn an_animation_holds_every_frame_at_its_time() {
        let encoding = Encoding { format: Format::Webp, quality: 90, lossless: true };
        assert!(let Ok(mut animation) = Animation::new(16, 16, 20.0, encoding));
        for shift in [0, 60, 120] {
            assert!(let Ok(()) = animation.push(&gradient(16, 16, shift)));
        }
        assert!(let Ok(bytes) = animation.finish());
        assert!(let Ok(mut decoder) = webpx::AnimationDecoder::new(&bytes));
        assert!(let Ok(frames) = decoder.decode_all());
        let times: Vec<i32> = frames.iter().map(|frame| frame.timestamp_ms).collect();
        check!(times == [50, 100, 150]);
    }
}
