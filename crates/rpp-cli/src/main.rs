use clap::Parser;
use tracing_subscriber::EnvFilter;

mod cli;
mod dev_server;

use cli::{Cli, Commands};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Build(cmd) => cmd.run(),
        Commands::Serve(cmd) => cmd.run().await,
    }
}
