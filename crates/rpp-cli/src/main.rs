//! The `rpp` command-line interface.
//!
//! Subcommands (spec §9): `init`, `build`, `dev`, `clean`, and `plugin ...`.
//! Shared project plumbing lives in [`project`]; pretty output in [`ui`]; LuaLS
//! editor stubs in [`luals`].

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use rpp_cli::commands;
use rpp_cli::commands::build::BuildArgs;
use rpp_cli::commands::component::ComponentCommand;
use rpp_cli::commands::init::InitArgs;
use rpp_cli::commands::plugin::PluginCommand;
use rpp_cli::ui;

/// rpp — a Minecraft resource pack build tool.
#[derive(Debug, Parser)]
#[command(name = "rpp", version, about, long_about = None)]
struct Cli {
    /// Run as if rpp were started in `<dir>` instead of the current directory.
    #[arg(short = 'C', long = "dir", global = true)]
    dir: Option<PathBuf>,

    /// Increase log verbosity (`-v` = debug, `-vv` = trace).
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Scaffold a new rpp project.
    Init(InitCli),
    /// Resolve plugins, build incrementally, then squash + zip.
    Build(BuildCli),
    /// Watch, rebuild, and serve with live reload.
    Dev,
    /// Remove the build output and cache.
    Clean,
    /// Manage plugins (`add`/`remove`/`list`/`update`/`search`).
    #[command(subcommand)]
    Plugin(PluginCommand),
    /// Inspect and generate bindings for WASM components.
    #[command(subcommand)]
    Component(ComponentCommand),
}

#[derive(Debug, Args)]
struct InitCli {
    /// Target directory (defaults to the current directory).
    dir: Option<PathBuf>,
    /// Pack name (skips the prompt).
    #[arg(long)]
    name: Option<String>,
    /// Pack description (skips the prompt).
    #[arg(long)]
    description: Option<String>,
    /// Pack format (skips the prompt).
    #[arg(long)]
    pack_format: Option<u32>,
    /// Accept defaults without prompting.
    #[arg(short, long)]
    yes: bool,
}

#[derive(Debug, Args)]
struct BuildCli {
    /// Clean the cache first (a full rebuild).
    #[arg(long)]
    no_cache: bool,
    /// Skip the squash/zip phase.
    #[arg(long)]
    no_squash: bool,
    /// Worker thread count (0 / unset = available parallelism).
    #[arg(long)]
    jobs: Option<usize>,
}

fn main() {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    let dir = cli.dir.clone().unwrap_or_else(|| PathBuf::from("."));

    let result = match cli.command {
        Command::Init(args) => commands::init::run(InitArgs {
            dir: args.dir.or(cli.dir),
            name: args.name,
            description: args.description,
            pack_format: args.pack_format,
            yes: args.yes,
        }),
        Command::Build(args) => commands::build::run(
            &dir,
            BuildArgs {
                no_cache: args.no_cache,
                no_squash: args.no_squash,
                jobs: args.jobs,
            },
        ),
        Command::Dev => commands::dev::run(&dir),
        Command::Clean => commands::clean::run(&dir),
        Command::Plugin(cmd) => commands::plugin::run(&dir, cmd),
        Command::Component(cmd) => commands::component::run(&dir, cmd),
    };

    if let Err(err) = result {
        if err
            .downcast_ref::<std::io::Error>()
            .is_some_and(|error| error.kind() == std::io::ErrorKind::Interrupted)
        {
            ui::cancel();
            std::process::exit(130);
        }
        ui::error(format!("{err:#}"));
        std::process::exit(1);
    }
}

/// Initialize tracing from `-v` flags and `RUST_LOG`.
fn init_tracing(verbose: u8) {
    use tracing_subscriber::{fmt, EnvFilter};

    let default = match verbose {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("rpp={default},rpp_cli={default}")));

    fmt()
        .with_env_filter(filter)
        .with_ansi(
            ui::is_terminal() && std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty()),
        )
        .with_target(false)
        .without_time()
        .with_writer(std::io::stderr)
        .try_init()
        .ok();
}
