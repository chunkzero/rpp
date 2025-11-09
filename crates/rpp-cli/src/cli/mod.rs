mod build;
mod init;
mod plugin;

use crate::cli::build::BuildCommand;
use crate::cli::init::init;
use crate::cli::plugin::PluginCommand;
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

/// A toolchain to build & test Minecraft resource packs.
#[derive(Parser, Debug, Clone)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// The path to the configuration file
    #[arg(short, long = "config", default_value = "rpp.toml")]
    pub config_path: String,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug, Clone)]
#[command(version, about, long_about = None)]
pub enum Command {
    /// Initialize RPP
    #[command()]
    Init,
    /// Build a resource pack
    #[command()]
    Build(BuildCommand),
    /// Manage RPP plugins
    #[command()]
    Plugin {
        #[command(subcommand)]
        command: PluginCommand,
    },
    /// Launch an HTTP dev server that watches for changes and continuously updates a client.
    #[command(about)]
    Serve {},
}

impl Command {
    pub fn run(self, config_path: PathBuf) -> anyhow::Result<()> {
        match self {
            Command::Init => init(),
            Command::Build(command) => command.run(),
            Command::Plugin { command } => command.run(config_path),
            Command::Serve { .. } => Err(anyhow::anyhow!("Serve command not implemented")),
        }
    }
}

#[derive(Args, Debug, Clone)]
pub struct DefaultArgs {
    #[arg(short, long, default_value = "./rpp.jsonc")]
    pub config: PathBuf,
}
