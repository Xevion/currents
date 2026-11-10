mod cli;
mod export;
mod gpu;
mod renderer;
mod viewer;

use clap::Parser;
use cli::{Cli, Command};
use snafu::{ResultExt, Snafu};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[derive(Debug, Snafu)]
enum Error {
    #[snafu(display("rendering failed"))]
    Render { source: export::Error },
    #[snafu(display("the viewer failed"))]
    Viewer { source: viewer::Error },
}

/// Logs to stderr at `warn` unless `RUST_LOG` says otherwise, and takes in the `log` records wgpu emits.
fn init_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().compact().with_writer(std::io::stderr))
        .with(filter)
        .init();
}

#[snafu::report]
fn main() -> Result<(), Error> {
    init_logging();
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Render(args)) => export::run(&cli.common, &args).context(RenderSnafu),
        None => viewer::run(cli.common).context(ViewerSnafu),
    }
}
