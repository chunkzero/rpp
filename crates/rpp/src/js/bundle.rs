//! Bundling a plugin entry with the SDK, its runtime and any discovered modules.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use rpp_js::{Bundle, BundlePackage, BundleRequest};
use serde_json::Value;

use super::bundle_cache::cached_bundle;
use super::discover::{discovered_module, Discovery};
use super::{hint_renamed_specifiers, jsx_modules, JSX_IMPORT_SOURCE, SDK_CONFIG, SDK_INDEX};
use crate::error::{Error, Result};
use crate::manifest::{plugin_modules, PluginManifest};
use crate::util::path::to_forward_slash;

const RUNTIME_SOURCE: &str = include_str!("sdk/runtime.ts");

/// A bundled plugin.
pub(super) struct PluginBundle {
    pub(super) bundle: Bundle,
    /// Source-relative files bundled into the plugin that are not pack content.
    pub(super) authoring: BTreeSet<String>,
    /// The canonical pack source directory, when the plugin discovers entries in it.
    pub(super) source: Option<PathBuf>,
}

/// Bundle the plugin at `root`, caching the bundle under `<cache_dir>/bundles`. A plugin with
/// `discover` patterns is bundled from `source`, which must not contain `root`.
pub(super) fn bundle_plugin(
    manifest: &PluginManifest,
    root: &Path,
    source: &Path,
    discovery: &Discovery,
    cache_dir: &Path,
) -> Result<PluginBundle> {
    let load_error = |message: String| Error::PluginLoad {
        plugin: manifest.id.clone(),
        message,
    };
    if discovery.is_empty() {
        let request = BundleRequest {
            root: root.to_path_buf(),
            entry: "rpp:internal/entry".into(),
            virtual_modules: virtual_modules(&format!("./{}", manifest.entry), false),
            packages: self_packages(manifest, root),
            jsx_import_source: Some(JSX_IMPORT_SOURCE.to_string()),
        };
        let bundle = cached_bundle(cache_dir, &manifest.id, &request, || {
            rpp_js::bundle(&request).map_err(|e| hint_renamed_specifiers(e.to_string()))
        })
        .map_err(load_error)?;
        return Ok(PluginBundle {
            bundle,
            authoring: BTreeSet::new(),
            source: None,
        });
    }

    let source = source.canonicalize().map_err(|e| Error::io(source, e))?;
    if root.starts_with(&source) {
        return Err(load_error(format!(
            "plugin directory {} is inside the pack source directory {}; plugins that \
             declare `discover` must live outside `build.source`",
            root.display(),
            source.display()
        )));
    }
    let (bundle, authoring) =
        bundle_discovered(manifest, root, &source, discovery, cache_dir).map_err(load_error)?;
    Ok(PluginBundle {
        bundle,
        authoring,
        source: Some(source),
    })
}

fn virtual_modules(entry: &str, discovered: bool) -> BTreeMap<String, String> {
    let (import, register) = if discovered {
        (
            "import discovered from \"rpp:internal/discovered\";\n",
            "register(plugin, discovered);",
        )
    } else {
        ("", "register(plugin);")
    };
    let entry_module = format!(
        "import plugin from {};\nimport {{ register }} from \"rpp:internal/runtime\";\n{import}{register}\nexport * from \"rpp:internal/runtime\";\n",
        Value::String(entry.to_string())
    );
    [
        ("rpp".to_string(), SDK_INDEX.to_string()),
        (
            "rpp:internal/runtime".to_string(),
            RUNTIME_SOURCE.to_string(),
        ),
        ("rpp:internal/entry".to_string(), entry_module),
    ]
    .into_iter()
    .chain(jsx_modules())
    .collect()
}

/// The plugin's own [`plugin_modules`], as packages rooted at `plugin_root`.
fn self_packages(manifest: &PluginManifest, plugin_root: &Path) -> BTreeMap<String, BundlePackage> {
    plugin_modules(&manifest.id, manifest.config.as_deref(), &manifest.exports)
        .into_iter()
        .map(|(specifier, module)| {
            let dir = plugin_root.to_path_buf();
            let entry = module.to_string();
            (specifier, BundlePackage { dir, entry })
        })
        .collect()
}

/// Bundle the plugin together with the files its patterns match under `source`. The bundle
/// root is `source`; the plugin's own files are the package `rpp:internal/plugin`.
fn bundle_discovered(
    manifest: &PluginManifest,
    plugin_root: &Path,
    source: &Path,
    discovery: &Discovery,
    cache_dir: &Path,
) -> std::result::Result<(Bundle, BTreeSet<String>), String> {
    let entries = discovery.discover(source)?;

    let mut virtual_modules = virtual_modules("rpp:internal/plugin", true);
    virtual_modules.insert("rpp:config".into(), SDK_CONFIG.into());
    virtual_modules.insert(
        "rpp:internal/discovered".into(),
        discovered_module("", discovery, &entries),
    );
    let mut packages = self_packages(manifest, plugin_root);
    packages.insert(
        "rpp:internal/plugin".to_string(),
        BundlePackage {
            dir: plugin_root.to_path_buf(),
            entry: manifest.entry.clone(),
        },
    );
    let request = BundleRequest {
        root: source.to_path_buf(),
        entry: "rpp:internal/entry".into(),
        virtual_modules,
        packages,
        jsx_import_source: Some(JSX_IMPORT_SOURCE.to_string()),
    };
    let bundle = cached_bundle(cache_dir, &manifest.id, &request, || {
        rpp_js::bundle(&request).map_err(|e| hint_renamed_specifiers(e.to_string()))
    })?;

    let authoring = bundle
        .inputs
        .iter()
        .filter_map(|input| input.strip_prefix(source).ok())
        .map(to_forward_slash)
        .collect();
    Ok((bundle, authoring))
}
