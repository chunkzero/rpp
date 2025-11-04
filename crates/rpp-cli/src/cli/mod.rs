mod build;

use std::path::PathBuf;
use clap::{Args, Parser, Subcommand};
use crate::cli::build::BuildArgs;

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
    /// Build a command
    #[command(about)]
    Build(BuildArgs),
    /// Create a skull texture containing objmc info
    #[command(about = "Create a skull texture containing objmc info")]
    Head {},
    #[command(about = "Join multiple models together")]
    Join {
        output: String,
        models: Vec<String>
    },
}

#[derive(Args, Debug, Clone)]
pub struct DefaultArgs {
    #[arg(default_value = "./config.json")]
    pub config: PathBuf,
}