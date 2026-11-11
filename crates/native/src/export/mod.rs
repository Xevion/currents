pub mod downsample;
pub mod encode;
mod readback;
mod reference;
mod sink;

use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use currents_core::gpu::{self, Gpu};
use currents_core::renderer::{PassTimes, Renderer, Uniforms};
use currents_core::shaders;
use downsample::downsample;
use encode::{Encoding, Format};
use readback::Readback;
use reference::Reference;
use sink::Sink;
use snafu::{ResultExt, Snafu, ensure};

use crate::cli::{Common, OutputSize, RenderArgs, View};

const COVER: &str = "references/stills/cover-apple-3000.jpg";

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("could not open a graphics device"))]
    Gpu { source: gpu::Error },
    #[snafu(display("could not load the shaders"))]
    Shaders { source: shaders::Error },
    #[snafu(display("an output size of {width}x{height} has no pixels to render"))]
    EmptyOutput { width: u32, height: u32 },
    #[snafu(display("--lossless applies to WebP, and the output is {format}"))]
    LosslessFormat { format: Format },
    #[snafu(display("reading the rendered frame back from the GPU failed"))]
    Readback { source: wgpu::BufferAsyncError },
    #[snafu(display("could not read the cover image {}", path.display()))]
    OpenCover { path: PathBuf, source: image::ImageError },
    #[snafu(display("could not create the directory {}", path.display()))]
    CreateDir { path: PathBuf, source: io::Error },
    #[snafu(display("could not encode {}", path.display()))]
    Encode { path: PathBuf, source: encode::Error },
    #[snafu(display("could not write {}", path.display()))]
    Write { path: PathBuf, source: io::Error },
}

/// Where the frames go.
enum Target {
    Still(PathBuf),
    /// Numbered files in a directory.
    Sequence(PathBuf),
    /// One animated WebP.
    Animation(PathBuf),
}

impl Target {
    /// The explicit format wins, then the output's extension, then PNG. Several frames make a
    /// sequence unless the output is itself a WebP file, which holds them as an animation.
    fn plan(args: &RenderArgs) -> (Self, Format) {
        let named = args.out.as_deref().and_then(Format::from_path);
        let format = args.format.or(named).unwrap_or(Format::Png);
        let target = match &args.out {
            Some(out) if args.frames > 1 && format == Format::Webp && named == Some(Format::Webp) => {
                Self::Animation(out.clone())
            },
            Some(out) if args.frames > 1 => Self::Sequence(out.clone()),
            Some(out) => Self::Still(out.clone()),
            None if args.frames > 1 => Self::Sequence(PathBuf::from("out/frames")),
            None => Self::Still(PathBuf::from(format!("out/frame.{}", format.extension()))),
        };
        (target, format)
    }

    fn path(&self) -> &Path {
        match self {
            Self::Still(path) | Self::Sequence(path) | Self::Animation(path) => path,
        }
    }

    fn sink(&self, fps: f32, encoding: Encoding) -> Sink {
        match self {
            Self::Still(path) => {
                let path = path.clone();
                Sink::stills(move |_| path.clone(), encoding)
            },
            Self::Sequence(dir) => {
                let dir = dir.clone();
                Sink::stills(move |frame| dir.join(format!("{frame:04}.{}", encoding.format.extension())), encoding)
            },
            Self::Animation(path) => Sink::animation(path.clone(), fps, encoding),
        }
    }
}

/// Renders `args.frames` frames headlessly and writes them out, printing a timing summary after the last.
///
/// Each frame is encoded and written on other threads while the next one renders.
pub(crate) fn run(common: &Common, args: &RenderArgs) -> Result<(), Error> {
    let size = args.size.unwrap_or_else(|| OutputSize::for_view(args.view));
    ensure!(size.width > 0 && size.height > 0, EmptyOutputSnafu { width: size.width, height: size.height });
    let (target, format) = Target::plan(args);
    ensure!(!args.lossless || format == Format::Webp, LosslessFormatSnafu { format });
    let encoding = Encoding { format, quality: args.quality, lossless: args.lossless };
    let ss = common.supersampling();
    let shaded = size.scaled(ss);

    let sources = common.sources().context(ShadersSnafu)?;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle().with_env());
    let Gpu { device, queue, .. } = pollster::block_on(gpu::request(&instance, None)).context(GpuSnafu)?;
    let mut renderer = Renderer::new(&device, &queue, &sources);
    renderer.set_temporal(common.temporal());
    let readback = Readback::new(&device, shaded);
    let reference = args.reference.then(|| Reference::open(Path::new(COVER))).transpose()?;
    let sink = target.sink(args.fps, encoding);
    let mut pass_sum = PassTimes::default();
    let mut wall_ms = 0.0;
    let mut slowest = (0, 0.0f32);

    let begun = Instant::now();
    let from = args.view.to_array();
    let to = args.to.map_or(from, View::to_array);
    for frame in 0..args.frames {
        let along = frame as f32 / (args.frames - 1).max(1) as f32;
        let uniforms = Uniforms::new(
            [size.width, size.height],
            ss,
            common.field_step,
            (args.time + f64::from(frame) / f64::from(args.fps)) * f64::from(common.speed),
            std::array::from_fn(|edge| from[edge] + (to[edge] - from[edge]) * along),
            common.param_array(),
        );

        let started = Instant::now();
        let mut enc = device.create_command_encoder(&Default::default());
        renderer.render(&device, &queue, &mut enc, &uniforms, common.debug);
        readback.copy_from(&mut enc, renderer.color());
        queue.submit([enc.finish()]);
        renderer.after_submit();
        let mapped = readback.map(&device)?;
        wall_ms = started.elapsed().as_secs_f32() * 1000.0;
        let pass = renderer.pass_ms(&device, true).unwrap_or_default();
        pass_sum.field += pass.field;
        pass_sum.shade += pass.shade;
        // the first two frames trace the picture and then a whole key, as any run must
        if frame >= 2 && pass.field + pass.shade > slowest.1 {
            slowest = (frame, pass.field + pass.shade);
        }

        let image = mapped.with_rows(|rows| downsample(rows, ss));
        drop(mapped);
        let image = match &reference {
            Some(reference) => reference.side_by_side(args.view, &image),
            None => image,
        };
        if !sink.push(frame, image) {
            break;
        }
    }
    sink.finish()?;
    print_summary(target.path(), size, ss, wall_ms, pass_sum, args.frames, begun.elapsed().as_secs_f32());
    if args.frames > 2 {
        println!("slowest frame after the first two: {:.2}ms on the GPU (frame {})", slowest.1, slowest.0);
    }
    Ok(())
}

/// Prints the output path, size and the last frame's render time, then each pass's time averaged
/// over `frames` and how fast they were written.
fn print_summary(path: &Path, size: OutputSize, ss: u32, wall_ms: f32, pass_sum: PassTimes, frames: u32, seconds: f32) {
    let n = frames as f32;
    print!(
        "{} {}x{} (ss {}) wall {:.0}ms | field {:.2}ms shade {:.2}ms (mean of {})",
        path.display(),
        size.width,
        size.height,
        ss,
        wall_ms,
        pass_sum.field / n,
        pass_sum.shade / n,
        frames
    );
    if frames > 1 {
        print!(" | {:.1} frames/s", n / seconds);
    }
    println!();
}
