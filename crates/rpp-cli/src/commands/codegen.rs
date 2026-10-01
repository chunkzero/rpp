//! `rpp codegen`: write the TypeScript SDK and tsconfig files.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

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

/// The nearest ancestor of `start` containing `rpp.toml`, `rpp.config.ts` or `plugin.toml`.
pub(crate) fn find_root(start: &Path) -> Result<PathBuf> {
    let start = std::path::absolute(start)?;
    let found = start.ancestors().find(|dir| {
        [CONFIG_FILE, TS_CONFIG_FILE, PLUGIN_FILE]
            .iter()
            .any(|name| dir.join(name).is_file())
    });
    match found {
        Some(dir) => Ok(dir.to_path_buf()),
        None => bail!(
            "no `{CONFIG_FILE}`, `{TS_CONFIG_FILE}` or `{PLUGIN_FILE}` found in `{}` or any parent directory",
            start.display()
        ),
    }
}
