use askama::Template;
use dialoguer::theme::ColorfulTheme;
use rpp::plugin::ID_REGEX;

use crate::template::{PackJsoncTemplate, RppConfigTemplate};
use std::{
    fs::{self, File},
    io::Write,
    path::PathBuf,
};

static CONFIG_PATH: &str = "rpp.toml";

// I think the function name could be more descriptive. Something like "cli_init" could be better.
// If it's a method off of a struct that init is fine (although I usually do "new"), but it could be ambigous what you're initializing.
pub(super) fn init() -> anyhow::Result<()> {
    let theme = ColorfulTheme::default();

    if should_abort_existing_project(&theme)? {
        println!("Project initialization cancelled.");
        return Ok(());
    }

    let root_path = get_root_directory(&theme)?;
    let source_path = get_source_directory(&theme)?;
    let pack_id = get_pack_configuration(&theme)?;

    create_project_structure(&root_path, &source_path, pack_id.as_deref(), &theme)?;

    println!(
        "\nSuccessfully initialized rpp project at {}\nRun `rpp help` to see available commands",
        root_path.display()
    );

    Ok(())
}

fn should_abort_existing_project(theme: &ColorfulTheme) -> anyhow::Result<bool> {
    // My mentor really doesn't like explicit returns in Rust, I personally don't mind them but just something to think abt.
    // He'd tell me to make an else {} block after this so it's all implicit.
    if !PathBuf::from(CONFIG_PATH).exists() {
        return Ok(false);
    }

    let continue_init = dialoguer::Confirm::with_theme(theme)
        .with_prompt("An rpp project already exists in this directory. Continue?")
        .default(false)
        .interact()?;

    Ok(!continue_init)
}

fn get_root_directory(theme: &ColorfulTheme) -> anyhow::Result<PathBuf> {
    let root_input: String = dialoguer::Input::with_theme(theme)
        .with_prompt("Project root directory")
        .with_initial_text("./")
        .validate_with(validate_path)
        .interact_text()?;

    let root = PathBuf::from(&root_input).canonicalize().or_else(|_| {
        // If canonicalize fails, try to create the directory first
        let path = PathBuf::from(&root_input);
        fs::create_dir_all(&path)?;
        path.canonicalize()
    })?;

    ensure_directory_exists(&root)?;
    Ok(root)
}

fn get_source_directory(theme: &ColorfulTheme) -> anyhow::Result<String> {
    dialoguer::Input::with_theme(theme)
        .with_prompt("Source subdirectory (relative to project root)")
        .with_initial_text("rpp")
        .validate_with(validate_relative_path)
        .interact_text()
        .map_err(anyhow::Error::from)
}

fn get_pack_configuration(theme: &ColorfulTheme) -> anyhow::Result<Option<String>> {
    let create_pack = dialoguer::Confirm::with_theme(theme)
        .with_prompt("Create a resource pack?")
        .default(true)
        .interact()?;

    if !create_pack {
        return Ok(None);
    }

    let pack_id = dialoguer::Input::with_theme(theme)
        .with_prompt("Resource pack ID (letters, numbers, dashes, and underscores only)")
        .validate_with(|input: &String| {
            if input.trim().is_empty() {
                return Err("Pack ID cannot be empty");
            }
            // You could make this "else if" so that the above code doesn't need an explicit return.
            // I'll stop mentioning explicit returns but it occurs elsewhere too.
            if ID_REGEX.is_match(input.trim()) {
                Ok(())
            } else {
                Err("Pack ID can only contain letters, numbers, dashes, and underscores")
            }
        })
        .interact_text()?;

    Ok(Some(pack_id.trim().to_string()))
}

fn validate_path(input: &String) -> Result<(), &'static str> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Path cannot be empty");
    }

    if cfg!(windows) && contains_invalid_windows_chars(trimmed) {
        return Err("Path contains invalid characters for Windows");
    }

    Ok(())
}

fn validate_relative_path(input: &String) -> Result<(), &'static str> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Path cannot be empty");
    }

    if trimmed.starts_with('/') || (cfg!(windows) && trimmed.contains(':')) {
        return Err("Path must be relative, not absolute");
    }

    if cfg!(windows) && contains_invalid_windows_chars(trimmed) {
        return Err("Path contains invalid characters for Windows");
    }

    Ok(())
}

#[cfg(windows)]
fn contains_invalid_windows_chars(path: &str) -> bool {
    path.chars().any(|c| "<>:\"/\\|?*".contains(c))
}

#[cfg(not(windows))]
fn contains_invalid_windows_chars(_path: &str) -> bool {
    false
}

fn ensure_directory_exists(path: &PathBuf) -> anyhow::Result<()> {
    if path.exists() {
        if !path.is_dir() {
            anyhow::bail!("Path '{}' exists but is not a directory", path.display());
        }
    } else {
        fs::create_dir_all(path).map_err(|e| {
            anyhow::anyhow!("Failed to create directory '{}': {}", path.display(), e)
        })?;
    }
    Ok(())
}

fn create_project_structure(
    root: &PathBuf,
    source_relative: &str,
    pack_id: Option<&str>,
    theme: &ColorfulTheme,
) -> anyhow::Result<()> {
    let config_path = root.join(CONFIG_PATH);

    if config_path.exists() {
        let overwrite = dialoguer::Confirm::with_theme(theme)
            .with_prompt("Configuration file already exists. Overwrite it?")
            .default(true)
            .interact()?;

        if !overwrite {
            println!("Keeping existing configuration file.");
            return Ok(());
        }
    }

    let mut config_file = File::create(&config_path)
        .map_err(|e| anyhow::anyhow!("Failed to create config file: {}", e))?;

    let config_content = RppConfigTemplate {
        root: source_relative,
    }
    .render()?;

    config_file
        .write_all(config_content.as_bytes())
        .map_err(|e| anyhow::anyhow!("Failed to write config file: {}", e))?;

    let source_path = root.join(source_relative);
    ensure_directory_exists(&source_path)?;

    let directories = [".rpp", "plugins", "packs"];
    for dir in directories {
        let dir_path = source_path.join(dir);
        if !dir_path.exists() {
            fs::create_dir(&dir_path).map_err(|e| {
                anyhow::anyhow!("Failed to create directory '{}': {}", dir_path.display(), e)
            })?;
        }
    }

    if let Some(pack_id) = pack_id {
        create_resource_pack(&source_path, pack_id)?;
    }

    Ok(())
}

fn create_resource_pack(source_path: &PathBuf, pack_id: &str) -> anyhow::Result<()> {
    let pack_dir = source_path.join("packs").join(pack_id);
    fs::create_dir(&pack_dir)
        .map_err(|e| anyhow::anyhow!("Failed to create pack directory: {}", e))?;

    let pack_config_path = pack_dir.join("pack.jsonc");
    let mut pack_file = File::create(&pack_config_path)
        .map_err(|e| anyhow::anyhow!("Failed to create pack config: {}", e))?;

    let pack_content = PackJsoncTemplate { id: pack_id }.render()?;

    pack_file
        .write_all(pack_content.as_bytes())
        .map_err(|e| anyhow::anyhow!("Failed to write pack config: {}", e))?;

    Ok(())
}
