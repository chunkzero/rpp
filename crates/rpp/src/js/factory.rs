//! [`JsPluginFactory`]: bundling and validation on the main thread.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use rpp_js::{Bundle, Call, Cancellation, Limits};
use serde::Deserialize;
use serde_json::{json, Value};

use super::access::RuntimeAccess;
use super::bundle::{bundle_plugin, PluginBundle};
use super::config::CONFIG_FILE;
use super::discover::Discovery;
use super::host::{JsHost, Phase};
use super::instance::{self, JsPluginInstance};
use super::{keys, runtime_limits};
use crate::config::{Config, PackConfig, PluginConfig};
use crate::error::{Error, Result};
use crate::manifest::PluginManifest;
use crate::model::{GeneratorHost, PluginFactory, PluginInstance, ProcessorDef};
use crate::util::glob::GlobSet;

/// One configured plugin for [`JsPluginFactory::load`].
pub struct JsPluginSpec<'a> {
    /// The plugin package directory, containing `rpp.json`.
    pub dir: &'a Path,
    /// The project root. `build.source`, process working directories and permission paths
    /// resolve against it, and bundles are cached under `.rpp/cache/bundles`.
    pub project_root: &'a Path,
    /// The project config, for pack metadata, `build.limits` and `build.source`.
    pub config: &'a Config,
    /// This plugin's entry in [`Config::plugins`].
    pub plugin: &'a PluginConfig,
    /// The manifest's compiled components, by name.
    #[cfg(feature = "wasm")]
    pub components: BTreeMap<String, rpp_wasm::CompiledComponent>,
}

/// A bundled, validated TypeScript plugin. Cheap to clone and shared across workers.
#[derive(Clone)]
pub struct JsPluginFactory {
    shared: Arc<Shared>,
}

pub(super) struct Shared {
    pub(super) id: String,
    pub(super) bundle: Bundle,
    pub(super) init: Value,
    pub(super) limits: Limits,
    pub(super) access: RuntimeAccess,
    processors: Vec<ProcessorDef>,
    pub(super) handlers: Handlers,
    processor_key: u64,
    cache_key: u64,
    discovery: Discovery,
    /// Source-relative files bundled into the plugin that are not pack content.
    authoring: BTreeSet<String>,
    overrides: Vec<String>,
}

/// Which optional handlers the plugin registered.
#[derive(Debug, Clone, Copy, Deserialize)]
pub(super) struct Handlers {
    pub(super) generator: bool,
    #[serde(rename = "onStart")]
    pub(super) on_start: bool,
    #[serde(rename = "onFinish")]
    pub(super) on_finish: bool,
}

#[derive(Deserialize)]
struct Description {
    processors: Vec<ProcessorDescription>,
    #[serde(flatten)]
    handlers: Handlers,
}

#[derive(Deserialize)]
struct ProcessorDescription {
    name: String,
    files: Vec<String>,
    priority: i32,
}

impl JsPluginFactory {
    /// Read `rpp.json` in `spec.dir`, bundle its entry, evaluate it once under
    /// `build.limits` to read its processors and handlers, and compute the cache keys.
    ///
    /// When the manifest declares `discover` patterns, the files they match under
    /// `build.source` are bundled with the plugin and exposed through
    /// `ctx.discovered(name)`. Those files and everything they import from the source
    /// directory are not part of the pack ([`PluginFactory::is_authoring_source`]).
    ///
    /// The processor key covers the manifest, declared component bytes, canonical `options`,
    /// host access, the rpp version and every bundled file outside the source directory (the
    /// plugin's own files). The generator key adds the bundled code, so editing a discovered
    /// module or adding or removing one reruns generators but not cached processor results.
    /// Top-level side effects of discovered modules are not tracked for processors.
    ///
    /// # Errors
    ///
    /// [`crate::Error::Config`] when `spec.plugin` is invalid, such as a sandboxed plugin
    /// granted permissions. [`crate::Error::PluginLoad`] for bundling or evaluation failures (with
    /// source-mapped stacks), a missing default export, or invalid processor
    /// declarations; I/O and manifest errors otherwise.
    pub fn load(spec: JsPluginSpec<'_>) -> Result<Self> {
        let (dir, project_root) = (spec.dir, spec.project_root);
        let (config, plugin) = (spec.config, spec.plugin);
        plugin.validate(&project_root.join(CONFIG_FILE))?;
        let (manifest, manifest_source) = PluginManifest::load_with_source(dir)?;
        let id = manifest.id.clone();
        let load_error = |message: String| Error::PluginLoad {
            plugin: id.clone(),
            message,
        };

        let root = plugin_root(dir, &manifest)?;
        let discovery = Discovery::new(&manifest.discover).map_err(&load_error)?;
        let PluginBundle {
            bundle,
            authoring,
            source,
        } = bundle_plugin(
            &manifest,
            &root,
            &project_root.join(&config.build.source),
            &discovery,
            &project_root.join(".rpp/cache"),
        )?;
        let limits = runtime_limits(&config.build.limits);
        let access = RuntimeAccess::new(spec);
        let description = describe(&id, &bundle, limits, &access).map_err(&load_error)?;
        let processors = processor_defs(description.processors).map_err(&load_error)?;
        let options = keys::canonical_options(&plugin.options);
        let processor_key = keys::processor_key(
            &root,
            &manifest,
            &manifest_source,
            &bundle,
            source.as_deref(),
            &options,
            &access,
        )?;

        Ok(Self {
            shared: Arc::new(Shared {
                init: init_args(&id, &config.pack, &options),
                cache_key: keys::cache_key(processor_key, &bundle),
                id,
                bundle,
                limits,
                access,
                processors,
                handlers: description.handlers,
                processor_key,
                discovery,
                authoring,
                overrides: manifest.overrides,
            }),
        })
    }

    pub(super) fn shared(&self) -> &Shared {
        &self.shared
    }
}

impl Shared {
    /// A host for one call in `phase` starting now.
    pub(super) fn host<'a>(
        &'a self,
        phase: Phase,
        generator: Option<&'a mut dyn GeneratorHost>,
    ) -> JsHost<'a> {
        let deadline = Instant::now() + self.limits.time;
        JsHost::new(&self.access, phase, deadline, generator)
    }
}

impl PluginFactory for JsPluginFactory {
    fn id(&self) -> &str {
        &self.shared.id
    }

    fn cache_key(&self) -> u64 {
        self.shared.cache_key
    }

    fn processor_key(&self) -> u64 {
        self.shared.processor_key
    }

    fn processors(&self) -> &[ProcessorDef] {
        &self.shared.processors
    }

    fn has_generator(&self) -> bool {
        self.shared.handlers.generator
    }

    fn cacheable_processors(&self) -> bool {
        self.shared.access.is_deterministic()
    }

    fn cacheable_generator(&self) -> bool {
        self.shared.access.is_deterministic()
    }

    fn output_roots(&self) -> BTreeMap<String, PathBuf> {
        self.shared.access.outputs.clone()
    }

    fn overrides(&self) -> &[String] {
        &self.shared.overrides
    }

    fn is_authoring_source(&self, rel: &str) -> bool {
        let discovery = &self.shared.discovery;
        discovery.matches(rel)
            || self.shared.authoring.contains(rel)
            || (!discovery.is_empty() && is_typescript(rel))
    }

    fn instantiate(&self) -> Result<Box<dyn PluginInstance>> {
        Ok(Box::new(JsPluginInstance::new(self.clone())?))
    }
}

/// TypeScript sources are never pack content; type-only imports are erased before bundling,
/// so the bundle's inputs alone would miss them.
fn is_typescript(rel: &str) -> bool {
    rel.rsplit_once('.')
        .is_some_and(|(_, ext)| matches!(ext, "ts" | "mts" | "cts" | "tsx"))
}

/// The canonical plugin directory, after checking that the entry stays inside it.
fn plugin_root(dir: &Path, manifest: &PluginManifest) -> Result<PathBuf> {
    let root = dir.canonicalize().map_err(|e| Error::io(dir, e))?;
    let entry = root.join(&manifest.entry);
    let canonical_entry = entry.canonicalize().map_err(|e| Error::io(&entry, e))?;
    if !canonical_entry.starts_with(&root) {
        return Err(Error::PluginLoad {
            plugin: manifest.id.clone(),
            message: format!("entry `{}` escapes the plugin directory", manifest.entry),
        });
    }
    Ok(root)
}

/// The argument of the `init` export.
fn init_args(id: &str, pack: &PackConfig, options: &Value) -> Value {
    let pack_json = json!({
        "name": pack.name,
        "description": pack.description,
        "format": pack.format,
    });
    json!({ "plugin": id, "options": options, "pack": pack_json })
}

/// Evaluate the bundle once and read what the plugin registered.
fn describe(
    id: &str,
    bundle: &Bundle,
    limits: Limits,
    access: &RuntimeAccess,
) -> std::result::Result<Description, String> {
    let engine = instance::engine()?;
    let cancellation = Cancellation::new();
    let clock = instance::module_clock(id);
    let (mut runtime, _) = engine
        .load(id, bundle, limits, clock, &cancellation)
        .map_err(|e| e.to_string().trim_end().to_string())?;
    let mut host = JsHost::new(access, Phase::Load, Instant::now() + limits.time, None);
    let call = Call {
        export: "describe",
        args: Value::Null,
        bytes: None,
        clock,
    };
    let output = runtime
        .call(&engine, call, &mut host, &cancellation)
        .map_err(|e| e.to_string().trim_end().to_string())?;
    serde_json::from_value(output.value).map_err(|e| format!("invalid plugin description: {e}"))
}

fn processor_defs(
    descriptions: Vec<ProcessorDescription>,
) -> std::result::Result<Vec<ProcessorDef>, String> {
    descriptions
        .into_iter()
        .map(|d| {
            GlobSet::new(&d.files).map_err(|e| format!("processor `{}`: {e}", d.name))?;
            Ok(ProcessorDef {
                name: d.name,
                patterns: d.files,
                priority: d.priority,
            })
        })
        .collect()
}
