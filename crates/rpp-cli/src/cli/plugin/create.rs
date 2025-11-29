use std::{fs::File, io::Write, path::PathBuf};

use askama::Template;
use dialoguer::theme::ColorfulTheme;
use rpp::plugin::{ID_REGEX, SEMVER_REGEX};
use std::fs;
use toml_edit::DocumentMut;

use crate::{
    cli::plugin::{load_config, preset::PluginPreset},
    config::PluginDef,
    template::{LuaRcTemplate, PluginTomlTemplate},
};

pub fn handle_create(config_path: PathBuf) -> anyhow::Result<()> {
    let config = load_config(&config_path)?;
    let root_dir = config_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Invalid config path"))?
        .join(&config.rpp.root_dir);

    let theme = ColorfulTheme::default();

    let id = dialoguer::Input::with_theme(&theme)
        .with_prompt("Plugin ID (letters, numbers, dashes, and underscores only)")
        .validate_with(|input: &String| {
            let trimmed = input.trim();
            if trimmed.is_empty() {
                return Err("Plugin ID cannot be empty");
            }
            if ID_REGEX.is_match(trimmed) {
                Ok(())
            } else {
                Err("Plugin ID can only contain letters, numbers, dashes, and underscores")
            }
        })
        .interact_text()?
        .trim()
        .to_string();

    let plugin_dir = root_dir.join("plugins").join(&id);
    if plugin_dir.exists() {
        let overwrite = dialoguer::Confirm::with_theme(&theme)
            .with_prompt(&format!("Plugin '{}' already exists. Overwrite it?", &id))
            .default(false)
            .interact()?;

        if !overwrite {
            println!("Plugin creation cancelled.");
            return Ok(());
        }
    }

    let plugin_info = collect_remaining_plugin_info(&theme, id.clone())?;

    update_rpp_config(&config_path, PluginDef::Local { id })?;

    create_plugin_structure(&root_dir, &plugin_info)?;
    // If you want an extra line, just do \n. No need call empty println!.
    println!();
    println!("Successfully created plugin '{}'!", plugin_info.id);
    println!("Read the documentation at https://rpp.oglass.dev/docs/plugin");
    println!(
        "Plugin location: {}/plugins/{}",
        root_dir.display(),
        plugin_info.id
    );

    Ok(())
}

struct PluginInfo {
    id: String,
    version: String,
    description: String,
    preset: PluginPreset,
}

fn collect_remaining_plugin_info(theme: &ColorfulTheme, id: String) -> anyhow::Result<PluginInfo> {
    let version = dialoguer::Input::with_theme(theme)
        .with_prompt("Plugin version (Semantic Versioning)")
        .with_initial_text("0.1.0-alpha.0")
        .validate_with(|input: &String| {
            let trimmed = input.trim();
            if SEMVER_REGEX.is_match(trimmed) {
                Ok(())
            } else {
                Err("Invalid semantic version format. See https://semver.org/ for details")
            }
        })
        .interact_text()?
        .trim()
        .to_string();

    let description = dialoguer::Input::with_theme(theme)
        .with_prompt("Plugin description")
        .default("A new rpp plugin".to_string())
        .interact_text()?
        .trim()
        .to_string();

    // Why is this a vector instead of a slice?
    let preset_options = vec![
        PluginPreset::Blank.as_str(),
        PluginPreset::LanguageGenerator.as_str(),
    ];

    let preset_index = dialoguer::Select::with_theme(theme)
        .with_prompt("Plugin template")
        .items(&preset_options)
        .default(0)
        .interact()?;

    let preset = PluginPreset::from_index(preset_index)
        .ok_or_else(|| anyhow::anyhow!("Invalid preset selection"))?;

    Ok(PluginInfo {
        id,
        version,
        description,
        preset,
    })
}

fn create_plugin_structure(root_dir: &PathBuf, plugin_info: &PluginInfo) -> anyhow::Result<()> {
    // These similar names are confusing :).
    // Also if you don't use plugins_dir anywhere else, you could do:
    // let plugin_dir = {
    //     let plugins_dir = root_dir.join("plugins");
    //     plugins_dir.join(&plugin_info.id);
    // };
    let plugins_dir = root_dir.join("plugins");
    let plugin_dir = plugins_dir.join(&plugin_info.id);

    if plugin_dir.exists() {
        fs::remove_dir_all(&plugin_dir)
            .map_err(|e| anyhow::anyhow!("Failed to remove existing plugin directory: {}", e))?;
    }

    fs::create_dir_all(&plugin_dir)
        .map_err(|e| anyhow::anyhow!("Failed to create plugin directory: {}", e))?;

    create_plugin_toml(&plugin_dir, plugin_info)?;
    create_luarc_json(&plugin_dir, root_dir)?;
    plugin_info.preset.create_preset_files(&plugin_dir)?;

    Ok(())
}

fn create_plugin_toml(plugin_dir: &PathBuf, plugin_info: &PluginInfo) -> anyhow::Result<()> {
    let template = PluginTomlTemplate {
        id: &plugin_info.id,
        version: &plugin_info.version,
        description: &plugin_info.description,
    };

    let content = template
        .render()
        .map_err(|e| anyhow::anyhow!("Failed to render plugin.toml template: {}", e))?;

    let file_path = plugin_dir.join("plugin.toml");
    let mut file = File::create(&file_path)
        .map_err(|e| anyhow::anyhow!("Failed to create plugin.toml: {}", e))?;

    file.write_all(content.as_bytes())
        .map_err(|e| anyhow::anyhow!("Failed to write plugin.toml: {}", e))?;

    Ok(())
}

fn create_luarc_json(plugin_dir: &PathBuf, root_dir: &PathBuf) -> anyhow::Result<()> {
    let api_source = root_dir.join(".rpp").join("api");
    let relative_api_source = pathdiff::diff_paths(&api_source, plugin_dir)
        .ok_or_else(|| anyhow::anyhow!("Failed to calculate relative path to API source"))?;

    let template = LuaRcTemplate {
        api_source: &relative_api_source.to_string_lossy(),
    };

    let content = template
        .render()
        .map_err(|e| anyhow::anyhow!("Failed to render .luarc.json template: {}", e))?;

    let file_path = plugin_dir.join(".luarc.json");
    let mut file = File::create(&file_path)
        .map_err(|e| anyhow::anyhow!("Failed to create .luarc.json: {}", e))?;

    file.write_all(content.as_bytes())
        .map_err(|e| anyhow::anyhow!("Failed to write .luarc.json: {}", e))?;

    Ok(())
}

fn update_rpp_config(config_path: &PathBuf, def: PluginDef) -> anyhow::Result<()> {
    let mut config = fs::read_to_string(config_path)
        .map_err(|_| {
            anyhow::anyhow!(
                "Failed to find an rpp project in the current directory.\nUse `rpp init` to initialize a new project"
            )
        })?.parse::<DocumentMut>()?;

    if let Some(plugin_arr) = config["rpp"]["plugins"].as_array_mut() {
        plugin_arr.push(def.as_string());
    };

    fs::write(config_path, config.to_string())?;

    Ok(())
}
