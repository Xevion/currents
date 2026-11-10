use std::time::Instant;

use currents_core::gpu::{self, Gpu};
use currents_core::renderer::{Renderer, Uniforms};
use currents_core::shaders::{self, Sources};
use snafu::{ResultExt, Snafu};

use crate::cli::{BenchArgs, Common, NamedView, OutputSize};

/// Frames rendered before timing starts, while pipelines and the GPU clock settle and the
/// first key, which is rendered whole, is taken.
const WARMUP_FRAMES: u32 = 4;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("could not open a graphics device"))]
    Gpu { source: gpu::Error },
    #[snafu(display("could not load the shaders"))]
    Shaders { source: shaders::Error },
}

#[derive(Clone, Copy, PartialEq, Eq, strum::Display)]
#[strum(serialize_all = "lowercase")]
enum PassKind {
    Field,
    Shade,
}

/// A part of a pass that can be cut out of its source to see what it costs.
struct Stage {
    pass: PassKind,
    name: &'static str,
    /// Text in the pass and what stands in for it with the stage removed.
    edits: &'static [(&'static str, &'static str)],
}

const STAGES: &[Stage] = &[
    Stage {
        pass: PassKind::Field,
        name: "trace steps past the first",
        edits: &[("const MAX_TRACE_STEPS = 192;", "const MAX_TRACE_STEPS = 1;")],
    },
    Stage {
        pass: PassKind::Field,
        name: "eddy noise",
        edits: &[
            (
                "let n1 = gradient_noise_smooth(vec2f(xi.x / lx, xi.y / ly) + vec2f(3.7, 1.3) + vec2f(0.0, 0.04 * t));",
                "let n1 = vec3f(0.0 * t);",
            ),
            (
                "let n2 = gradient_noise_smooth(xi / l2 + vec2f(-8.1, 5.9) + vec2f(0.05 * t, 0.0));",
                "let n2 = vec3f(0.0 * t);",
            ),
        ],
    },
    Stage {
        pass: PassKind::Field,
        name: "eddy envelope",
        edits: &[(
            "let grad = en.amp.x * ng + nv * en.grad_noise + en.amp.y * sg + sv * en.grad_sinuous;",
            "let grad = ng + sg;",
        )],
    },
    Stage {
        pass: PassKind::Field,
        name: "steps back to the key",
        edits: &[("let yn = step_back(y, tt, h, scn);", "let yn = y;")],
    },
    Stage {
        pass: PassKind::Field,
        name: "key read",
        edits: &[("liquid = key_liquid(rel + acc.xy, scn, coarse);", "liquid = rel + acc.xy;")],
    },
    Stage {
        pass: PassKind::Field,
        name: "screen velocity",
        edits: &[("let v = flow_velocity(rel, u.time, scn);", "let v = vec2f(0.0);")],
    },
    Stage {
        pass: PassKind::Shade,
        name: "flow grain",
        edits: &[
            ("let fine = flow_grain(gp, vflow, 1.2, fpx, 1.0);", "let fine = vec3f(0.0);"),
            ("let clump = flow_grain(gp, vflow, 5.5, fpx, 9.0);", "let clump = vec3f(0.0);"),
        ],
    },
    Stage {
        pass: PassKind::Shade,
        name: "stream and bands",
        edits: &[("col = stream_color(pix);", "col = pix.black;"), ("col = band_color(pix);", "col = pix.black;")],
    },
    Stage { pass: PassKind::Shade, name: "ball shadow", edits: &[("col *= ball_shadow(pix, scn, ball);", "")] },
    Stage { pass: PassKind::Shade, name: "spray fade", edits: &[("col = spray_fade(col, pix);", "")] },
    Stage {
        pass: PassKind::Shade,
        name: "ball",
        edits: &[("let painted = paint_ball(col, pix, scn, ball);", "let painted = vec4f(col, 0.0);")],
    },
    Stage {
        pass: PassKind::Shade,
        name: "glare",
        edits: &[("col = glare(painted.rgb, pix.cover);", "col = painted.rgb;")],
    },
];

impl Stage {
    /// The sources with this stage cut out, or `None` when the pass no longer holds its text.
    fn removed_from(&self, sources: &Sources) -> Option<Sources> {
        let mut out = sources.clone();
        let text = match self.pass {
            PassKind::Field => &mut out.field,
            PassKind::Shade => &mut out.shade,
        };
        for (find, replace) in self.edits {
            if !text.contains(find) {
                return None;
            }
            *text = text.replace(find, replace);
        }
        Some(out)
    }
}

/// Median milliseconds per frame; the passes are `None` without GPU timestamps.
#[derive(Clone, Copy)]
struct Sample {
    field: Option<f32>,
    shade: Option<f32>,
    /// Mean and worst of both passes together, which show what the median hides: the frames
    /// that render a key.
    mean: Option<f32>,
    worst: Option<f32>,
    wall: f32,
}

impl Sample {
    fn gpu(&self) -> Option<f32> {
        self.field.zip(self.shade).map(|(field, shade)| field + shade)
    }

    fn pass(&self, pass: PassKind) -> Option<f32> {
        match pass {
            PassKind::Field => self.field,
            PassKind::Shade => self.shade,
        }
    }
}

fn median(mut values: Vec<f32>) -> Option<f32> {
    values.sort_by(f32::total_cmp);
    values.get(values.len() / 2).copied()
}

struct Bench<'a> {
    device: wgpu::Device,
    queue: wgpu::Queue,
    common: &'a Common,
    args: &'a BenchArgs,
}

impl Bench<'_> {
    /// A renderer for `sources`, or the reason its shaders did not compile.
    fn renderer(&self, sources: &Sources) -> Result<Renderer, String> {
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut renderer = Renderer::new(&self.device, &self.queue, sources);
        renderer.set_temporal(self.common.temporal());
        pollster::block_on(scope.pop()).map_or(Ok(renderer), |error| Err(error.to_string()))
    }

    /// Renders the warmup frames, then `frames` timed ones a thirtieth of a second apart.
    fn measure(&self, renderer: &mut Renderer, view: &NamedView, size: OutputSize) -> Sample {
        let (mut field, mut shade, mut wall) = (Vec::new(), Vec::new(), Vec::new());
        for frame in 0..WARMUP_FRAMES + self.args.frames.max(1) {
            let uniforms = Uniforms::new(
                [size.width, size.height],
                self.common.supersampling(),
                self.common.field_step,
                (self.args.time + frame as f32 / 30.0) * self.common.speed,
                view.view.to_array(),
                self.common.param_array(),
            );
            let started = Instant::now();
            let mut enc = self.device.create_command_encoder(&Default::default());
            renderer.render(&self.device, &self.queue, &mut enc, &uniforms, self.common.debug);
            self.queue.submit([enc.finish()]);
            renderer.after_submit();
            self.device
                .poll(wgpu::PollType::wait_indefinitely())
                .expect("polling without a submission index cannot fail");
            let elapsed = started.elapsed().as_secs_f32() * 1000.0;
            // read every frame's stamps, warmup too: one left unread is what the next read returns
            let pass = renderer.pass_ms(&self.device, true);
            if frame < WARMUP_FRAMES {
                continue;
            }
            wall.push(elapsed);
            if let Some(pass) = pass {
                field.push(pass.field);
                shade.push(pass.shade);
            }
        }
        let gpu: Vec<f32> = field.iter().zip(&shade).map(|(field, shade)| field + shade).collect();
        Sample {
            mean: (!gpu.is_empty()).then(|| gpu.iter().sum::<f32>() / gpu.len() as f32),
            worst: gpu.iter().copied().max_by(f32::total_cmp),
            field: median(field),
            shade: median(shade),
            wall: median(wall).unwrap_or_default(),
        }
    }

    /// Prints what each stage saves when cut out of its pass, against `full`.
    fn stages(&self, sources: &Sources, view: &NamedView, size: OutputSize, full: Sample) {
        for stage in STAGES {
            let Some(base) = full.pass(stage.pass) else { continue };
            let Some(cut) = stage.removed_from(sources) else {
                println!("    {:<28} not found in the pass source", stage.name);
                continue;
            };
            match self.renderer(&cut) {
                Ok(mut renderer) => {
                    let without = self.measure(&mut renderer, view, size).pass(stage.pass).unwrap_or(base);
                    let pass = stage.pass;
                    let saved = base - without;
                    println!("    {pass} {:<28} {saved:>7.2} ms {:>5.0}%", stage.name, 100.0 * saved / base);
                },
                Err(reason) => println!("    {:<28} did not compile: {reason}", stage.name),
            }
        }
    }
}

fn ns_per_pixel(ms: Option<f32>, pixels: u32) -> String {
    ms.map_or_else(|| "    n/a".to_owned(), |ms| format!("{:>7.2}", ms * 1e6 / pixels as f32))
}

fn ms(value: Option<f32>) -> String {
    value.map_or_else(|| "    n/a".to_owned(), |v| format!("{v:>7.2}"))
}

fn ratio(base: Option<f32>, now: Option<f32>) -> f32 {
    base.zip(now).map_or(f32::NAN, |(base, now)| base / now)
}

fn print_row(label: &str, size: OutputSize, ss: u32, sample: Sample) {
    let pixels = size.width * size.height;
    println!(
        "{label:<8} {:>11} | {} {} | {} {} | {} {} {} {:>7.2}",
        format!("{}x{}", size.width, size.height),
        ms(sample.field),
        ns_per_pixel(sample.field, pixels),
        ms(sample.shade),
        ns_per_pixel(sample.shade, pixels * ss * ss),
        ms(sample.gpu()),
        ms(sample.mean),
        ms(sample.worst),
        sample.wall,
    );
}

/// Times both passes over every view and size given, and each stage of them when asked. With a
/// baseline shader directory, times that too and prints how much faster the passes are than it.
pub(crate) fn run(common: &Common, args: &BenchArgs) -> Result<(), Error> {
    let sources = common.sources().context(ShadersSnafu)?;
    let instance = wgpu::Instance::default();
    let Gpu { adapter, device, queue } = pollster::block_on(gpu::request(&instance, None)).context(GpuSnafu)?;
    let info = adapter.get_info();
    let ss = common.supersampling();
    let bench = Bench { device, queue, common, args };
    let mut renderer = Renderer::new(&bench.device, &bench.queue, &sources);
    let mut baseline = match &args.baseline {
        Some(dir) => {
            let sources = Sources::from_dir(dir).context(ShadersSnafu)?;
            Some(Renderer::new(&bench.device, &bench.queue, &sources))
        },
        None => None,
    };

    println!("{} ({:?}), ss {ss}, median of {} frames", info.name, info.backend, args.frames.max(1));
    println!(
        "{:<8} {:>11} | {:>7} {:>7} | {:>7} {:>7} | {:>7} {:>7} {:>7} {:>7}",
        "view", "size", "field", "ns/px", "shade", "ns/px", "gpu ms", "mean", "worst", "wall ms"
    );
    for view in &args.views {
        for &size in &args.sizes {
            let sample = bench.measure(&mut renderer, view, size);
            print_row(&view.name, size, ss, sample);
            if let Some(baseline) = baseline.as_mut() {
                let base = bench.measure(baseline, view, size);
                print_row("  base", size, ss, base);
                println!(
                    "  {:>17} | {:>14.2}x | {:>14.2}x | {:>6.2}x",
                    "speedup",
                    ratio(base.field, sample.field),
                    ratio(base.shade, sample.shade),
                    ratio(base.gpu(), sample.gpu()),
                );
            }
            if args.stages {
                bench.stages(&sources, view, size, sample);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use assert2::check;

    use super::*;

    #[test]
    fn every_stage_is_found_in_the_embedded_shaders() {
        let sources = Sources::embedded();
        check!(let None = STAGES.iter().find(|stage| stage.removed_from(&sources).is_none()).map(|s| s.name));
    }
}
