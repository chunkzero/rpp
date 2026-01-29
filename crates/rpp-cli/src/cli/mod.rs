mod build;
mod dev;
mod init;

pub use build::BuildCommand;
pub use dev::DevCommand;
pub use init::InitCommand;

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
    Dev(DevCommand),
    /// Initialize a new RPP project
    Init(InitCommand),
}
