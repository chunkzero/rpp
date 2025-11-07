use clap::Parser;

use crate::cli::Cli;

mod cli;
mod config;
mod plugin;
mod server;
mod template;

pub const DEFAULT_CONFIG_PATH: &str = "./rpp.toml";

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();
    cli.command.run()
}
