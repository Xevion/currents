//! The `currents` command line: the interactive viewer, headless rendering to image files and
//! timing of the GPU passes.

mod bench;
mod cli;
pub mod export;
mod host_package;
mod store;
#[cfg(target_os = "linux")]
mod x11;

use clap::parser::ValueSource;
use clap::{CommandFactory, FromArgMatches};
use cli::{Cli, Command};
use currents_core::viewer::{Launch, Placement};
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
    #[snafu(display("could not package for the host"))]
    HostPackage { source: host_package::Error },
    #[snafu(display("could not load the shaders"))]
    Shaders { source: shaders::Error },
}

/// The viewer's starting point: the saved settings with this run's flags laid over them.
fn launch(cli: &Cli, matches: &clap::ArgMatches) -> Result<Launch, Error> {
    let store::Loaded { mut settings, mut store, issues } = store::load();
    let given = |id: &str| matches.value_source(id) == Some(ValueSource::CommandLine);
    let from_file = settings.clone();
    cli.common.overlay(&mut settings, given);
    let placement = cli.place.placement();
    if placement != Placement::Window {
        settings.camera.locked = true;
    }
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
        on_window: Some(Box::new({
            let placement = placement.clone();
            move |window, _remote| place(&window, &placement)
        })),
        placement,
    })
}

/// Finishes `placement` once the window is up: the X11 hints a desktop window needs, or a
/// thread that keeps an embedded one the size of its parent.
#[cfg(target_os = "linux")]
fn place(window: &std::sync::Arc<winit::window::Window>, placement: &Placement) {
    let Some(id) = x11::xid(window) else {
        if *placement != Placement::Window {
            tracing::warn!("the window is not an X11 one; it stays an ordinary window");
        }
        return;
    };
    match placement {
        Placement::Window => {},
        Placement::Desktop { .. } => {
            if let Err(error) = x11::mark_desktop(id) {
                tracing::warn!(%error, "could not mark the window as the desktop's");
            }
        },
        &Placement::Embed(parent) => {
            let window = std::sync::Arc::clone(window);
            std::thread::spawn(move || {
                let followed = x11::follow_parent(parent, |width, height| {
                    tracing::debug!(width, height, "parent resized");
                    let _ = window.request_inner_size(winit::dpi::PhysicalSize::new(width, height));
                });
                if let Err(error) = followed {
                    tracing::warn!(%error, "stopped following the parent window's size");
                }
            });
        },
    }
}

#[cfg(not(target_os = "linux"))]
fn place(_: &std::sync::Arc<winit::window::Window>, _: &Placement) {}

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
        Some(Command::HostPackage(args)) => host_package::run(args.host, &args.dir).context(HostPackageSnafu),
        None => viewer::run(launch(&cli, &matches)?).context(ViewerSnafu),
    }
}
