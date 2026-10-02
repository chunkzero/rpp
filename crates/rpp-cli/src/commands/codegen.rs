//! `rpp codegen`: write the TypeScript SDK and tsconfig files.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rpp_fetch::registry::PACKAGE_MANIFEST;

use crate::project::{is_plugin_manifest, nearest, CONFIG_FILE};
use crate::{codegen, ui};

/// Run the codegen command from `dir`.
pub fn run(dir: &Path) -> Result<()> {
    let root = find_root(dir)?;
    ui::intro("Generate TypeScript definitions");
    codegen::write(&root)?;
    ui::success(format!("Wrote .rpp/sdk and tsconfig in {}", root.display()));
    Ok(())
}

/// The nearest ancestor of `start` containing `rpp.config.ts` or an
/// `rpp.json` plugin manifest.
pub(crate) fn find_root(start: &Path) -> Result<PathBuf> {
    let start = std::path::absolute(start)?;
    nearest(&start, |dir| {
        dir.join(CONFIG_FILE).is_file() || is_plugin_manifest(&dir.join(PACKAGE_MANIFEST))
    })?
    .with_context(|| {
        format!(
            "no `{CONFIG_FILE}` or plugin `{PACKAGE_MANIFEST}` found in `{}` or any parent \
             directory",
            start.display()
        )
    })
}
