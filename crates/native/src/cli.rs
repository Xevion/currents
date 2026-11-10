mod size;

use std::path::PathBuf;
use std::str::FromStr;

use clap::{Args, Parser, Subcommand, ValueEnum};
pub(crate) use currents_core::View;
use currents_core::renderer::FieldView;
use currents_core::shaders::{self, Sources};
use currents_core::viewer::Settings;
use currents_core::{ParamBlock, ParamOverride};
pub(crate) use size::OutputSize;

use crate::export::encode::Format;

/// Live rendering of the Currents cover. With no subcommand, opens the interactive viewer
/// (scroll to zoom at the cursor, drag to pan, space to pause, R to reset, D to cycle the
/// field debug views, T to switch temporal antialiasing, Esc to quit).
#[derive(Parser)]
pub(crate) struct Cli {
    #[command(flatten)]
    pub(crate) common: Common,
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

/// How detail finer than a pixel is kept from flickering as it moves.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Antialias {
    /// Each frame stands alone
    Off,
    /// Each frame is blended with the ones before it, following the liquid
    Temporal,
}

#[derive(Args, Clone)]
pub(crate) struct Common {
    /// Directory holding the WGSL passes
    #[arg(long, default_value = "shaders", global = true)]
    pub(crate) shaders: PathBuf,
    /// Flow speed multiplier
    #[arg(long, default_value_t = 0.4, global = true)]
    pub(crate) speed: f32,
    /// Supersampling factor per axis
    #[arg(long, default_value_t = 1, global = true)]
    pub(crate) ss: u32,
    /// Antialiasing across frames
    #[arg(long, value_enum, default_value_t = Antialias::Temporal, global = true)]
    pub(crate) aa: Antialias,
    /// Output pixels per flow field texel; above 1 trades the stream's thinnest filaments for speed
    #[arg(long, default_value_t = 1, global = true)]
    pub(crate) field_step: u32,
    /// Tuning parameter override, INDEX=VALUE (repeatable)
    #[arg(short = 'p', long = "param", global = true)]
    pub(crate) params: Vec<ParamOverride>,
    /// Draw a view of the flow field instead of the picture: line, label, velocity or density
    #[arg(long, global = true)]
    pub(crate) debug: Option<FieldView>,
}

impl Common {
    /// The overrides packed into the shaders' parameter block.
    pub(crate) fn param_array(&self) -> ParamBlock {
        ParamOverride::pack(&self.params)
    }

    /// Supersampling factor per axis, never below one.
    pub(crate) fn supersampling(&self) -> u32 {
        self.ss.max(1)
    }

    /// Whether frames are blended with the ones before them.
    pub(crate) fn temporal(&self) -> bool {
        self.aa == Antialias::Temporal
    }

    pub(crate) fn sources(&self) -> Result<Sources, shaders::Error> {
        Sources::from_dir(&self.shaders)
    }

    /// The viewer setup: every window pixel rendered, with the shaders read from disk.
    pub(crate) fn settings(&self) -> Result<Settings, shaders::Error> {
        Ok(Settings {
            sources: self.sources()?,
            speed: self.speed,
            supersampling: self.supersampling(),
            temporal: self.temporal(),
            field_step: self.field_step,
            params: self.param_array(),
            budget: None,
            debug: self.debug,
        })
    }
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Render frames to image files without opening a window
    Render(RenderArgs),
    /// Time the GPU passes over views and sizes without opening a window
    Bench(BenchArgs),
}

/// A view together with the text it was given as.
#[derive(Clone)]
pub(crate) struct NamedView {
    pub(crate) name: String,
    pub(crate) view: View,
}

impl FromStr for NamedView {
    type Err = <View as FromStr>::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self { name: s.to_owned(), view: s.parse()? })
    }
}

#[derive(Args)]
pub(crate) struct BenchArgs {
    /// View to measure, as for `render` (repeatable)
    #[arg(long = "view", default_values = ["full", "a", "g"], allow_hyphen_values = true)]
    pub(crate) views: Vec<NamedView>,
    /// Output size as WxH, shaded at the supersampling factor times this (repeatable)
    #[expect(clippy::doc_markdown, reason = "the help text must read WxH without backticks")]
    #[arg(long = "size", default_values = ["1920x1080", "3840x2160"])]
    pub(crate) sizes: Vec<OutputSize>,
    /// Time of the first frame in seconds (before the speed multiplier)
    #[arg(long, default_value_t = 10.0)]
    pub(crate) time: f32,
    /// Frames timed per measurement
    #[arg(long, default_value_t = 30)]
    pub(crate) frames: u32,
    /// Also time each pass with one stage cut out, to show what the stage costs
    #[arg(long)]
    pub(crate) stages: bool,
    /// Shader directory to time alongside and compare against
    #[arg(long)]
    pub(crate) baseline: Option<PathBuf>,
}

#[derive(Args)]
pub(crate) struct RenderArgs {
    /// Named view (full, wide, a..h, user) or x0,y0,x1,y1 in cover coordinates
    #[arg(long, default_value = "full", allow_hyphen_values = true)]
    pub(crate) view: View,
    /// View the camera moves to over the frames at an even rate, given as for `--view`
    #[arg(long, allow_hyphen_values = true)]
    pub(crate) to: Option<View>,
    /// Output size as WxH; defaults to 800 pixels tall at the view's aspect
    #[expect(clippy::doc_markdown, reason = "the help text must read WxH without backticks")]
    #[arg(long)]
    pub(crate) size: Option<OutputSize>,
    /// Start time in seconds (before the speed multiplier)
    #[arg(long, default_value_t = 0.0)]
    pub(crate) time: f32,
    /// Output file, or a directory of numbered files for several frames (a .webp file takes
    /// them as one animation); defaults to out/frame.<format>, or out/frames for several
    #[arg(long)]
    pub(crate) out: Option<PathBuf>,
    /// Image format; defaults to the output file's extension, else PNG
    #[arg(long, value_enum)]
    pub(crate) format: Option<Format>,
    /// Quality of lossy JPEG and WebP, 0 to 100
    #[arg(long, default_value_t = 90, value_parser = clap::value_parser!(u8).range(0..=100))]
    pub(crate) quality: u8,
    /// Encode WebP without loss
    #[arg(long)]
    pub(crate) lossless: bool,
    /// Place the matching crop of the cover on the left
    #[arg(long = "ref")]
    pub(crate) reference: bool,
    #[arg(long, default_value_t = 1)]
    pub(crate) frames: u32,
    #[arg(long, default_value_t = 30.0)]
    pub(crate) fps: f32,
}
