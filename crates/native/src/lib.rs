//! The `currents` command line: the interactive viewer, headless rendering to image files and
//! timing of the GPU passes.

mod bench;
mod cli;
pub mod export;
mod store;

use clap::parser::ValueSource;
use clap::{CommandFactory, FromArgMatches};
use cli::{Cli, Command};
use currents_core::viewer::Launch;
use currents_core::{shaders, viewer};
use snafu::{ResultExt, Snafu};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("the benchmark failed"))]
    Bench { source: bench::Error },
    #[snafu(display("rendering failed"))]
    Render { source: export::Error },
    #[snafu(display("the viewer failed"))]
    Viewer { source: viewer::Error },
    #[snafu(display("could not load the shaders"))]
    Shaders { source: shaders::Error },
}

/// The viewer's starting point: the saved settings with this run's flags laid over them.
fn launch(cli: &Cli, matches: &clap::ArgMatches) -> Result<Launch, Error> {
    let store::Loaded { mut settings, mut store, issues } = store::load();
    let given = |id: &str| matches.value_source(id) == Some(ValueSource::CommandLine);
    let from_file = settings.clone();
    cli.common.overlay(&mut settings, given);
    let overridden = currents_core::settings::toml::changed(&from_file, &settings);
    if !overridden.is_empty() {
        tracing::info!(?overridden, "flags override the settings file for this run");
    }
    store.assume(&settings);
    Ok(Launch {
        settings,
        sources: cli.common.sources().context(ShadersSnafu)?,
        debug: cli.common.debug,
        store: Some(Box::new(store)),
        issues,
    })
}

/// Logs this program's own events at `info` and other crates' at `warn` to stderr, unless
/// `RUST_LOG` says otherwise; takes in the `log` records wgpu emits.
fn init_logging() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn,currents=info,currents_core=info"));
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().compact().with_writer(std::io::stderr))
        .with(filter)
        .init();
}

/// Parses the command line and runs the command it names.
pub fn run() -> Result<(), Error> {
    init_logging();
    let matches = Cli::command().get_matches();
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());
    match &cli.command {
        Some(Command::Render(args)) => export::run(&cli.common, args).context(RenderSnafu),
        Some(Command::Bench(args)) => bench::run(&cli.common, args).context(BenchSnafu),
        None => viewer::run(launch(&cli, &matches)?).context(ViewerSnafu),
    }
}
