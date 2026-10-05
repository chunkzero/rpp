//! Bundling a plugin entry with the SDK, its runtime and any discovered modules.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use rpp_js::{Bundle, BundlePackage, BundleRequest};
use serde_json::Value;

use super::bundle_cache::cached_bundle;
use super::discover::{discovered_module, Discovery};
use super::{SDK_CONFIG, SDK_INDEX};
use crate::error::{Error, Result};
use crate::manifest::PluginManifest;
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
            entry: "rpp:entry".into(),
            virtual_modules: virtual_modules(&format!("./{}", manifest.entry), false),
            ..Default::default()
        };
        let bundle = cached_bundle(cache_dir, &manifest.id, &request, || {
            rpp_js::bundle(&request).map_err(|e| e.to_string())
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
            "import discovered from \"rpp:discovered\";\n",
            "register(plugin, discovered);",
        )
    } else {
        ("", "register(plugin);")
    };
    let entry_module = format!(
        "import plugin from {};\nimport {{ register }} from \"rpp:runtime\";\n{import}{register}\nexport * from \"rpp:runtime\";\n",
        Value::String(entry.to_string())
    );
    BTreeMap::from([
        ("#rpp".to_string(), SDK_INDEX.to_string()),
        ("rpp:runtime".to_string(), RUNTIME_SOURCE.to_string()),
        ("rpp:entry".to_string(), entry_module),
    ])
}

/// Bundle the plugin together with the files its patterns match under `source`. The bundle
/// root is `source`; the plugin's own files are the package `#plugin`.
fn bundle_discovered(
    manifest: &PluginManifest,
    plugin_root: &Path,
    source: &Path,
    discovery: &Discovery,
    cache_dir: &Path,
) -> std::result::Result<(Bundle, BTreeSet<String>), String> {
    let entries = discovery.discover(source)?;

    let mut virtual_modules = virtual_modules("#plugin", true);
    virtual_modules.insert("#rpp/config".into(), SDK_CONFIG.into());
    virtual_modules.insert(
        "rpp:discovered".into(),
        discovered_module("", discovery, &entries),
    );
    let package = |entry: &str| BundlePackage {
        dir: plugin_root.to_path_buf(),
        entry: entry.to_string(),
    };
    let mut packages = BTreeMap::from([("#plugin".to_string(), package(&manifest.entry))]);
    let mut jsx_import_source = None;
    if let Some(config) = &manifest.config {
        let specifier = format!("#plugins/{}", manifest.id);
        packages.insert(specifier.clone(), package(config));
        if manifest.jsx {
            packages.insert(format!("{specifier}/jsx-runtime"), package(config));
            jsx_import_source = Some(specifier);
        }
    }
    let request = BundleRequest {
        root: source.to_path_buf(),
        entry: "rpp:entry".into(),
        virtual_modules,
        packages,
        jsx_import_source,
    };
    let bundle = cached_bundle(cache_dir, &manifest.id, &request, || {
        rpp_js::bundle(&request).map_err(|e| e.to_string())
    })?;

    let authoring = bundle
        .inputs
        .iter()
        .filter_map(|input| input.strip_prefix(source).ok())
        .map(to_forward_slash)
        .collect();
    Ok((bundle, authoring))
}
