use clap::{Subcommand};

#[derive(Subcommand, Debug, Clone)]
#[command(about, long_about = None)]
pub enum PluginCommand {
    /// Add a plugin
    #[command(about, aliases = ["add"])]
    Install,
    Remove,
    List
}

impl PluginCommand {
    pub fn run(self) -> anyhow::Result<()> {
        Ok(())
    }
}