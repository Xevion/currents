mod downsample;
mod readback;
mod reference;

use std::path::{Path, PathBuf};
use std::time::Instant;
use std::{fs, io};

use downsample::downsample;
use readback::Readback;
use reference::Reference;
use snafu::{ResultExt, Snafu, ensure};

use crate::cli::{Common, OutputSize, RenderArgs};
use crate::gpu::{self, Gpu};
use crate::renderer::{self, PassTimes, Renderer, Uniforms};

const COVER: &str = "references/stills/cover-apple-3000.jpg";

#[derive(Debug, Snafu)]
pub(crate) enum Error {
    #[snafu(display("could not open a graphics device"))]
    Gpu { source: gpu::Error },
    #[snafu(display("could not set up the renderer"))]
    Renderer { source: renderer::Error },
    #[snafu(display("an output size of {width}x{height} has no pixels to render"))]
    EmptyOutput { width: u32, height: u32 },
    #[snafu(display("reading the rendered frame back from the GPU failed"))]
    Readback { source: wgpu::BufferAsyncError },
    #[snafu(display("could not read the cover image {}", path.display()))]
    OpenCover { path: PathBuf, source: image::ImageError },
    #[snafu(display("could not create the directory {}", path.display()))]
    CreateDir { path: PathBuf, source: io::Error },
    #[snafu(display("could not write {}", path.display()))]
    SaveImage { path: PathBuf, source: image::ImageError },
}

/// Renders `args.frames` frames headlessly and writes each as a PNG, printing a timing summary after the last.
pub(crate) fn run(common: &Common, args: &RenderArgs) -> Result<(), Error> {
    let size = args.size.unwrap_or_else(|| OutputSize::for_view(args.view));
    ensure!(size.width > 0 && size.height > 0, EmptyOutputSnafu { width: size.width, height: size.height });
    let ss = common.supersampling();
    let shaded = size.scaled(ss);

    let instance = wgpu::Instance::default();
    let Gpu { device, queue, .. } = gpu::request(&instance, None).context(GpuSnafu)?;
    let mut renderer = Renderer::new(&device, &queue, &common.shaders).context(RendererSnafu)?;
    let readback = Readback::new(&device, shaded);
    let reference = args.reference.then(|| Reference::open(Path::new(COVER))).transpose()?;
    let mut pass_sum = PassTimes::default();

    for frame in 0..args.frames {
        let out_path = if args.frames > 1 { args.out.join(format!("{frame:04}.png")) } else { args.out.clone() };
        let uniforms = Uniforms {
            resolution: [shaded.width as f32, shaded.height as f32],
            time: (args.time + frame as f32 / args.fps) * common.speed,
            field_scale: ss as f32,
            view: args.view.to_array(),
            params: common.param_array(),
        };

        let started = Instant::now();
        let mut enc = device.create_command_encoder(&Default::default());
        renderer.render(&device, &queue, &mut enc, &uniforms);
        readback.copy_from(&mut enc, renderer.color());
        queue.submit([enc.finish()]);
        renderer.after_submit();
        let mapped = readback.map(&device)?;
        let wall_ms = started.elapsed().as_secs_f32() * 1000.0;
        let pass = renderer.pass_ms(&device, true).unwrap_or_default();
        pass_sum.field += pass.field;
        pass_sum.shade += pass.shade;

        let image = downsample(&mapped.pixels(), shaded.width, ss, size);
        match &reference {
            Some(reference) => save(&out_path, &reference.side_by_side(args.view, &image))?,
            None => save(&out_path, &image)?,
        }
        if args.frames == 1 || frame + 1 == args.frames {
            print_summary(&out_path, size, ss, wall_ms, pass_sum, args.frames);
        }
    }
    Ok(())
}

fn save(path: &Path, image: &image::RgbImage) -> Result<(), Error> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).context(CreateDirSnafu { path: dir })?;
    }
    image.save(path).context(SaveImageSnafu { path })
}

/// Prints the output path, size and wall time, then each pass's time averaged over `frames`.
fn print_summary(path: &Path, size: OutputSize, ss: u32, wall_ms: f32, pass_sum: PassTimes, frames: u32) {
    let n = frames as f32;
    println!(
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
}
