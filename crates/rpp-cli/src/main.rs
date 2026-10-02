//! The `rpp` command-line interface.
//!
//! Subcommands (spec §9): `init`, `build`, `dev`, `clean`, `codegen`, `check`, `add`, `remove`,
//! `update`, `search`, and `plugin pack`.
//! Shared project plumbing lives in [`project`]; pretty output in [`ui`].

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use rpp_cli::commands;
use rpp_cli::commands::build::BuildArgs;
use rpp_cli::commands::init::InitArgs;
use rpp_cli::commands::plugin::PluginCommand;
use rpp_cli::ui;

const RELEASE_VERSION: &str = match option_env!("RPP_RELEASE_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

/// rpp — a Minecraft resource pack build tool.
#[derive(Debug, Parser)]
#[command(name = "rpp", version = RELEASE_VERSION, about, long_about = None)]
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
    Init(InitArgs),
    /// Resolve plugins, build incrementally, then squash + zip.
    Build(BuildArgs),
    /// Watch, rebuild, and serve with live reload.
    Dev,
    /// Remove the build output and cache.
    Clean,
    /// Add dependencies to `rpp.json` (`<name>[@range]` or `path:<dir>`).
    Add {
        /// Dependencies to add.
        #[arg(required = true)]
        specs: Vec<String>,
    },
    /// Remove dependencies from `rpp.json`.
    Remove {
        /// Dependency names to remove.
        #[arg(required = true)]
        names: Vec<String>,
    },
    /// Re-select registry dependency versions within their ranges.
    Update {
        /// Dependencies to update (all when omitted).
        names: Vec<String>,
    },
    /// Search the plugin registry.
    Search {
        /// The search query.
        query: String,
    },
    /// Write the TypeScript SDK and tsconfig files.
    Codegen,
    /// Generate definitions, then type-check TypeScript with `tsc`.
    Check,
    /// Package plugins for distribution (`pack`).
    #[command(subcommand)]
    Plugin(PluginCommand),
}

fn main() {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    let dir = cli.dir.clone().unwrap_or_else(|| PathBuf::from("."));

    let result = match cli.command {
        Command::Init(args) => commands::init::run(InitArgs {
            dir: args.dir.or(cli.dir),
            ..args
        }),
        Command::Build(args) => commands::build::run(&dir, args),
        Command::Dev => commands::dev::run(&dir),
        Command::Clean => commands::clean::run(&dir),
        Command::Add { specs } => commands::deps::add(&dir, &specs),
        Command::Remove { names } => commands::deps::remove(&dir, &names),
        Command::Update { names } => commands::deps::update(&dir, &names),
        Command::Search { query } => commands::deps::search(&query),
        Command::Codegen => commands::codegen::run(&dir),
        Command::Check => commands::check::run(&dir),
        Command::Plugin(cmd) => commands::plugin::run(&dir, cmd),
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
