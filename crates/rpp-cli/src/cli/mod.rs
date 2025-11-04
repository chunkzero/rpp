mod build;
mod plugin;
mod init;

use std::path::PathBuf;
use clap::{Args, Parser, Subcommand};
use crate::cli::build::BuildCommand;
use crate::cli::init::init;
use crate::cli::plugin::PluginCommand;

/// A toolchain to build & test Minecraft resource packs.
#[derive(Parser, Debug, Clone)]
#[command(version, about, long_about = None)]
pub struct Cli {
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
    pub fn run(self) -> anyhow::Result<()> {
        match self {
            Command::Init => init(),
            Command::Build(command) => command.run(),
            Command::Plugin { command } => command.run(),
            Command::Serve { .. } => {
                Err(anyhow::anyhow!("Serve command not implemented"))
            }
        }
    }
}

#[derive(Args, Debug, Clone)]
pub struct DefaultArgs {
    #[arg(short, long, default_value = "./rpp.jsonc")]
    pub config: PathBuf,
}