use crate::{cli::plugin::create::handle_create, config::Config};
use clap::Subcommand;
use rpp::plugin::PluginConfig;
use std::{
    fs::{self},
    path::PathBuf,
};

mod create;
mod preset;

#[derive(Subcommand, Debug, Clone)]
#[command(about, long_about = None)]
pub enum PluginCommand {
    /// Install a plugin from a remote source
    #[command(about, aliases = ["add"])]
    Install,
    /// Create a new plugin in the current project
    Create,
    /// Remove a plugin from the current project
    Remove,
    /// List all plugins in the current project
    List,
}

impl PluginCommand {
    pub fn run(self, config_path: PathBuf) -> anyhow::Result<()> {
        match self {
            PluginCommand::Install => handle_install(),
            PluginCommand::Create => handle_create(config_path),
            PluginCommand::Remove => handle_remove(),
            PluginCommand::List => handle_list(config_path),
        }
    }
}

fn handle_install() -> anyhow::Result<()> {
    println!("Remote plugin installation will be supported in a future version.");
    println!("For now, you can create local plugins using `rpp plugin create`");
    Ok(())
}

fn handle_remove() -> anyhow::Result<()> {
    println!("Plugin removal will be supported in a future version.");
    Ok(())
}

fn handle_list(config_path: PathBuf) -> anyhow::Result<()> {
    let config = load_config(&config_path)?;

    let local_plugins = config_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Invalid config path"))?
        .join(&config.rpp.root_dir)
        .join("plugins");

    if config.rpp.plugins.is_empty() {
        println!("No plugins found in this project");
        println!("Create one with `rpp plugin create`");
    } else {
        println!("Found {} plugin(s):", config.rpp.plugins.len());

        for ele in config.rpp.plugins {
            match ele {
                crate::config::PluginDef::Local { id } => {
                    let plugin_path = local_plugins.join(&id);

                    if !plugin_path.exists() {
                        println!("  • {} (Local) (Not found)", id);
                    } else {
                        print_plugin_data(plugin_path.join("plugin.toml"))?;
                    };
                }
                crate::config::PluginDef::Remote {
                    repository: _,
                    id: _,
                    version: _,
                } => {}
            };
        }
    };

    Ok(())
}

fn load_config(config_path: &PathBuf) -> anyhow::Result<Config> {
    let config_content = fs::read_to_string(config_path)
        .map_err(|_| {
            anyhow::anyhow!(
                "Failed to find an rpp project in the current directory.\nUse `rpp init` to initialize a new project"
            )
        })?;

    toml::from_str(&config_content)
        .map_err(|e| anyhow::anyhow!("Failed to parse project configuration: {}", e))
}

fn print_plugin_data(plugin_config_path: PathBuf) -> anyhow::Result<()> {
    let config_content = fs::read_to_string(plugin_config_path)?;

    let config: PluginConfig = toml::from_str(&config_content)
        .map_err(|e| anyhow::anyhow!("Failed to parse plugin configuration: {}", e))?;

    println!("  • {} ({})", config.id, config.version);
    if !config.description.is_empty() {
        println!("    {}", config.description);
    }
    Ok(())
}
