mod build;
mod serve;

pub use build::BuildCommand;
pub use serve::ServeCommand;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "rpp")]
#[command(about = "Resource Pack Processor")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Build a resource pack
    Build(BuildCommand),
    /// Start development server with hot reload
    Serve(ServeCommand),
}
