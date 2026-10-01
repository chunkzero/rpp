//! `rpp codegen`: write the TypeScript SDK and tsconfig files.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use rpp_fetch::registry::PACKAGE_MANIFEST;

use crate::project::{CONFIG_FILE, TS_CONFIG_FILE};
use crate::{codegen, ui};

const PLUGIN_FILE: &str = "plugin.toml";

/// Run the codegen command from `dir`.
pub fn run(dir: &Path) -> Result<()> {
    let root = find_root(dir)?;
    ui::intro("Generate TypeScript definitions");
    codegen::write(&root)?;
    ui::success(format!("Wrote .rpp/sdk and tsconfig in {}", root.display()));
    Ok(())
}

/// The nearest ancestor of `start` containing `rpp.toml`, `rpp.config.ts`, `plugin.toml` or an
/// `rpp.json` plugin manifest (one with `name` and `version`).
pub(crate) fn find_root(start: &Path) -> Result<PathBuf> {
    let start = std::path::absolute(start)?;
    let found = start.ancestors().find(|dir| {
        [CONFIG_FILE, TS_CONFIG_FILE, PLUGIN_FILE]
            .iter()
            .any(|name| dir.join(name).is_file())
            || is_plugin_manifest(&dir.join(PACKAGE_MANIFEST))
    });
    match found {
        Some(dir) => Ok(dir.to_path_buf()),
        None => bail!(
            "no `{CONFIG_FILE}`, `{TS_CONFIG_FILE}`, `{PLUGIN_FILE}` or plugin `{PACKAGE_MANIFEST}` found in `{}` or any parent directory",
            start.display()
        ),
    }
}

fn is_plugin_manifest(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .is_some_and(|json| json.get("name").is_some() && json.get("version").is_some())
}
