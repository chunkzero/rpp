//! [`JsPluginFactory`]: bundling and validation on the main thread.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use rpp_js::{Bundle, Call, Cancellation, Limits};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::access::RuntimeAccess;
use super::bundle::{bundle_plugin, PluginBundle};
use super::bundle_cache::{load_entry, store_entry};
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
use crate::util::versioned::Versioned;

const DESCRIBE_CACHE_KIND: &str = "describe";
const DESCRIBE_CACHE_VERSION: u32 = 1;

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

/// The `describe` result of a deterministic plugin, cached under `.rpp/cache/describe`.
#[derive(Serialize, Deserialize)]
struct DescribeCacheEntry {
    version: u32,
    key: u64,
    /// The `describe` result as JSON text.
    description: String,
}

impl Versioned for DescribeCacheEntry {
    const VERSION: u32 = DESCRIBE_CACHE_VERSION;

    fn version(&self) -> u32 {
        self.version
    }
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
    /// A plugin without permissions reuses its `describe` result from `.rpp/cache/describe` while
    /// the generator key, `build.limits` and `build.wasm` are unchanged, without evaluating it.
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
        let cache_dir = project_root.join(".rpp/cache");
        let PluginBundle {
            bundle,
            authoring,
            source,
        } = bundle_plugin(
            &manifest,
            &root,
            &project_root.join(&config.build.source),
            &discovery,
            &cache_dir,
        )?;
        let limits = runtime_limits(&config.build.limits);
        let access = RuntimeAccess::new(spec);
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
        let cache_key = keys::cache_key(processor_key, &bundle);
        let describe_key = access
            .is_deterministic()
            .then(|| keys::describe_key(cache_key, limits, &config.build.wasm));
        let description = cached_description(&cache_dir, &id, describe_key, || {
            describe(&id, &bundle, limits, &access)
        })
        .map_err(&load_error)?;
        let processors = processor_defs(description.processors).map_err(&load_error)?;

        Ok(Self {
            shared: Arc::new(Shared {
                init: init_args(&id, &config.pack, &options),
                cache_key,
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

/// The description stored for plugin `id` under `key`, or the result of `describe`, which is
/// stored when `key` is set. A missing, corrupt or stale entry is a miss.
fn cached_description(
    cache_dir: &Path,
    id: &str,
    key: Option<u64>,
    describe: impl FnOnce() -> std::result::Result<Value, String>,
) -> std::result::Result<Description, String> {
    let cached = key.and_then(|key| {
        let entry: DescribeCacheEntry = load_entry(cache_dir, DESCRIBE_CACHE_KIND, id)?;
        (entry.key == key).then_some(entry.description)
    });
    if let Some(description) = cached.and_then(|text| serde_json::from_str(&text).ok()) {
        return Ok(description);
    }
    let value = describe()?;
    let description = serde_json::from_value(value.clone())
        .map_err(|e| format!("invalid plugin description: {e}"))?;
    if let Some(key) = key {
        let entry = DescribeCacheEntry {
            version: DESCRIBE_CACHE_VERSION,
            key,
            description: value.to_string(),
        };
        store_entry(cache_dir, DESCRIBE_CACHE_KIND, id, &entry);
    }
    Ok(description)
}

/// Evaluate the bundle once and return what the plugin registered.
fn describe(
    id: &str,
    bundle: &Bundle,
    limits: Limits,
    access: &RuntimeAccess,
) -> std::result::Result<Value, String> {
    #[cfg(test)]
    tests::DESCRIPTIONS.with(|count| count.set(count.get() + 1));
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
    Ok(output.value)
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

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::config::{PluginPermissions, SecurityMode};

    thread_local! {
        pub(super) static DESCRIPTIONS: Cell<usize> = const { Cell::new(0) };
    }

    fn descriptions() -> usize {
        DESCRIPTIONS.with(Cell::get)
    }

    fn write(root: &Path, rel: &str, contents: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    fn write_processor_name(root: &Path, name: &str) {
        write(
            root,
            "src/name.ts",
            &format!("export const name = \"{name}\";\n"),
        );
    }

    /// A plugin whose processor name comes from the imported `src/name.ts`.
    fn write_plugin(root: &Path) {
        write(
            root,
            "rpp.json",
            r#"{ "name": "ts-test", "version": "1.0.0", "entry": "src/plugin.ts" }"#,
        );
        write(
            root,
            "src/plugin.ts",
            r##"import { definePlugin } from "#rpp";
import { name } from "./name.ts";
export default definePlugin({ processors: { [name]: { files: "**/*", run() {} } } });
"##,
        );
        write_processor_name(root, "first");
    }

    fn processor_names(root: &Path) -> Vec<String> {
        let config = Config::new("test-pack", 34);
        let plugin = PluginConfig {
            package: "ts-test".into(),
            options: json!({}),
            security: SecurityMode::Sandboxed,
            permissions: PluginPermissions::default(),
            outputs: BTreeMap::new(),
        };
        let factory = JsPluginFactory::load(JsPluginSpec {
            dir: root,
            project_root: root,
            config: &config,
            plugin: &plugin,
            #[cfg(feature = "wasm")]
            components: BTreeMap::new(),
        })
        .unwrap();
        factory
            .processors()
            .iter()
            .map(|processor| processor.name.clone())
            .collect()
    }

    #[test]
    fn reuses_description_until_a_plugin_module_changes() {
        let dir = tempfile::tempdir().unwrap();
        write_plugin(dir.path());
        assert_eq!(processor_names(dir.path()), ["first"]);
        assert_eq!(processor_names(dir.path()), ["first"]);
        assert_eq!(descriptions(), 1);

        write_processor_name(dir.path(), "second");
        assert_eq!(processor_names(dir.path()), ["second"]);
        assert_eq!(descriptions(), 2);
    }

    #[test]
    fn corrupt_description_is_a_miss() {
        let dir = tempfile::tempdir().unwrap();
        write_plugin(dir.path());
        processor_names(dir.path());
        let entries = dir.path().join(".rpp/cache").join(DESCRIBE_CACHE_KIND);
        for entry in std::fs::read_dir(&entries).unwrap() {
            std::fs::write(entry.unwrap().path(), b"garbage").unwrap();
        }
        assert_eq!(processor_names(dir.path()), ["first"]);
        assert_eq!(descriptions(), 2);
        processor_names(dir.path());
        assert_eq!(descriptions(), 2);
    }
}
