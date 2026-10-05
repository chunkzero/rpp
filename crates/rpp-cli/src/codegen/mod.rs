//! TypeScript editor and type-checker support: the SDK under `.rpp/sdk` and the
//! tsconfig files that map `#rpp` onto it.

mod component_dts;
mod tsconfig;

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};

use rpp::manifest::PluginManifest;
use rpp_fetch::registry::PACKAGE_MANIFEST;

use self::tsconfig::{tsconfig, PluginConfig, ROOT_TSCONFIG};
use crate::project::{is_plugin_manifest, resolve_ts_packages, CONFIG_FILE};

/// Write the SDK and tsconfig files under `root`, and `tsconfig.json` if absent.
/// Returns whether any file changed.
pub fn write(root: &Path) -> Result<bool> {
    let ts_project = root.join(CONFIG_FILE).is_file();
    let plugin_configs = if ts_project {
        let packages = resolve_ts_packages(root)?;
        Some(
            packages
                .into_iter()
                .filter_map(|(name, package)| {
                    let path = package.dir.join(package.manifest.config?);
                    let jsx = package.manifest.jsx;
                    Some((name, PluginConfig { path, jsx }))
                })
                .collect(),
        )
    } else if is_plugin_manifest(&root.join(PACKAGE_MANIFEST)) {
        Some(BTreeMap::new())
    } else {
        None
    };
    let rpp_dir = root.join(".rpp");
    let mut changed = false;
    for (path, contents) in rpp::js::SDK_FILES {
        changed |= write_if_changed(&rpp_dir.join("sdk").join(path), contents)?;
    }
    changed |= write_if_changed(&rpp_dir.join("sdk/globals.d.ts"), rpp_js::GLOBALS_DTS)?;
    changed |= write_if_changed(
        &rpp_dir.join("tsconfig.json"),
        &tsconfig(plugin_configs.as_ref()),
    )?;

    changed |= write_component_dts(root)?;

    let root_config = root.join("tsconfig.json");
    if !root_config.exists() {
        std::fs::write(&root_config, ROOT_TSCONFIG)
            .with_context(|| format!("writing {}", root_config.display()))?;
        changed = true;
    }
    Ok(changed)
}

/// Write `.rpp/generated/<name>.d.ts` for each component the plugin manifest in `root` declares.
fn write_component_dts(root: &Path) -> Result<bool> {
    if !is_plugin_manifest(&root.join(PACKAGE_MANIFEST)) {
        return Ok(false);
    }
    let manifest = PluginManifest::load(root)?;
    let mut changed = false;
    for (name, component) in &manifest.components {
        let wasm = root.join(&component.module);
        let bytes = match std::fs::read(&wasm) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                crate::ui::warn(format!(
                    "component `{name}`: {} is not built; skipping its type definitions",
                    wasm.display()
                ));
                continue;
            }
            Err(error) => {
                return Err(error).with_context(|| format!("reading {}", wasm.display()));
            }
        };
        let dts = component_dts::from_wasm(name, &bytes)
            .with_context(|| format!("generating types for component `{name}`"))?;
        changed |= write_if_changed(&root.join(format!(".rpp/generated/{name}.d.ts")), &dts)?;
    }
    Ok(changed)
}

fn write_if_changed(path: &Path, contents: &str) -> Result<bool> {
    if std::fs::read_to_string(path).is_ok_and(|existing| existing == contents) {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

/// Generate best-effort for build and dev: failures warn instead of aborting.
pub(crate) fn write_best_effort(root: &Path) {
    if let Err(error) = write(root) {
        crate::ui::warn(format!("TypeScript definitions: {error:#}"));
    }
}
