use std::path::PathBuf;

use clap::Parser;

use crate::cli::Cli;
// I would put this in a seperate lib.rs file
mod cli;
mod config;
mod server;
mod template;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();
    cli.command.run(PathBuf::from(cli.config_path))
}
