mod size;

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
pub(crate) use currents_core::View;
use currents_core::renderer::FieldView;
use currents_core::shaders::{self, Sources};
use currents_core::viewer::Settings;
use currents_core::{ParamBlock, ParamOverride};
pub(crate) use size::OutputSize;

/// Live rendering of the Currents cover. With no subcommand, opens the interactive viewer
/// (scroll to zoom at the cursor, drag to pan, space to pause, R to reset, D to cycle the
/// field debug views, Esc to quit).
#[derive(Parser)]
pub(crate) struct Cli {
    #[command(flatten)]
    pub(crate) common: Common,
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
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
    #[arg(long, default_value_t = 2, global = true)]
    pub(crate) ss: u32,
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

    pub(crate) fn sources(&self) -> Result<Sources, shaders::Error> {
        Sources::from_dir(&self.shaders)
    }

    /// The viewer setup: every window pixel rendered, with the shaders read from disk.
    pub(crate) fn settings(&self) -> Result<Settings, shaders::Error> {
        Ok(Settings {
            sources: self.sources()?,
            speed: self.speed,
            supersampling: self.supersampling(),
            params: self.param_array(),
            budget: None,
            debug: self.debug,
        })
    }
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Render frames to PNG without opening a window
    Render(RenderArgs),
}

#[derive(Args)]
pub(crate) struct RenderArgs {
    /// Named view (full, wide, a..h, user) or x0,y0,x1,y1 in cover coordinates
    #[arg(long, default_value = "full", allow_hyphen_values = true)]
    pub(crate) view: View,
    /// Output size as WxH; defaults to 800 pixels tall at the view's aspect
    #[expect(clippy::doc_markdown, reason = "the help text must read WxH without backticks")]
    #[arg(long)]
    pub(crate) size: Option<OutputSize>,
    /// Start time in seconds (before the speed multiplier)
    #[arg(long, default_value_t = 0.0)]
    pub(crate) time: f32,
    /// Output PNG, or a directory when rendering several frames
    #[arg(long, default_value = "out/frame.png")]
    pub(crate) out: PathBuf,
    /// Place the matching crop of the cover on the left
    #[arg(long = "ref")]
    pub(crate) reference: bool,
    #[arg(long, default_value_t = 1)]
    pub(crate) frames: u32,
    #[arg(long, default_value_t = 30.0)]
    pub(crate) fps: f32,
}
