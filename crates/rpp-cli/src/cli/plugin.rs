use std::fs;

use clap::Subcommand;
use rpp::plugin::{ID_REGEX, SEMVER_REGEX};

use crate::{config::Config, DEFAULT_CONFIG_PATH};

#[derive(Subcommand, Debug, Clone)]
#[command(about, long_about = None)]
pub enum PluginCommand {
    /// Add a plugin
    #[command(about, aliases = ["add"])]
    Install,
    Create,
    Remove,
    List,
}

impl PluginCommand {
    pub fn run(self) -> anyhow::Result<()> {
        match self {
            PluginCommand::Install => {
                println!("Remote plugins will be supported in a future version.");
                Ok(())
            }
            PluginCommand::Create => {
                let config: Config = match fs::read_to_string(DEFAULT_CONFIG_PATH) {
                    Ok(text) => toml::from_str(&text)?,
                    Err(_) => {
                        println!("Failed to find an rpp project in the working directory");
                        println!("Use `rpp init` to initialize a project");
                        return Ok(());
                    }
                };

                let _plugin_id: String =
                    dialoguer::Input::with_theme(&dialoguer::theme::ColorfulTheme::default())
                        .with_prompt("ID (a-zA-Z0-9_-)")
                        .validate_with(|text: &String| {
                            if ID_REGEX.is_match(text) {
                                Ok(())
                            } else {
                                Err("Only letters, numbers, dashes, and underscores are allowed")
                            }
                        })
                        .interact_text()
                        .unwrap();

                let _version: String =
                    dialoguer::Input::with_theme(&dialoguer::theme::ColorfulTheme::default())
                        .with_prompt("Version (SemVer)")
                        .with_initial_text("0.1.0-alpha.0")
                        .validate_with(|text: &String| {
                            if SEMVER_REGEX.is_match(text) {
                                Ok(())
                            } else {
                                Err("Invalid SemVer string. See https://semver.org/")
                            }
                        })
                        .interact_text()
                        .unwrap();

                let _ = dialoguer::Select::with_theme(&dialoguer::theme::ColorfulTheme::default())
                    .with_prompt("Preset")
                    .items(vec!["Blank", "Language Generator"])
                    .default(0)
                    .interact()
                    .unwrap();

                println!();
                println!("Successfully created plugin!");
                println!("Read docs at https://rpp.oglass.dev/docs/plugin");
                Ok(())
            }
            PluginCommand::Remove => {
                println!("Remote plugins will be supported in a future version.");
                Ok(())
            }
            PluginCommand::List => todo!(),
        }
    }
}
