//! [`JsPluginFactory`]: bundling and validation on the main thread.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rpp_js::{Bundle, BundlePackage, BundleRequest, Call, Cancellation, Engine, Limits};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::host::{PackInfo, PhaseCell, RuntimeAccess};
use crate::js::bundle_cache::cached_bundle;
use crate::js::discover::{discovered_module, Discovery};
use crate::js::host::JsHost;
use crate::js::instance::{self, JsPluginInstance};
use crate::manifest::PluginManifest;
use crate::model::{PluginFactory, PluginInstance, ProcessorDef};
use crate::util::canonical::canonical_options_json;
use crate::util::glob::GlobSet;
use crate::util::hash::HashWriter;
use crate::util::json_toml::toml_to_json;
use crate::util::path::to_forward_slash;

const RUNTIME_SOURCE: &str = include_str!("sdk/runtime.ts");
const CONFIG_SDK: &str = include_str!("sdk/config.ts");

/// Resource limits applied to each JavaScript runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsPluginLimits {
    /// V8 heap limit in bytes.
    pub memory_limit: usize,
    /// Maximum wall-clock time for one call.
    pub execution_limit: Duration,
}

impl Default for JsPluginLimits {
    fn default() -> Self {
        Self {
            memory_limit: 256 * 1024 * 1024,
            execution_limit: Duration::from_secs(30),
        }
    }
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
    /// Read `rpp.json` in `dir`, bundle its entry, evaluate it once to read its
    /// processors and handlers, and compute the cache key.
    ///
    /// When the manifest declares `discover` patterns, the files they match under `source`
    /// (the absolute pack source directory) are bundled with the plugin and exposed through
    /// `ctx.discovered(name)`. Those files and everything they import from `source` are not
    /// part of the pack ([`PluginFactory::is_authoring_source`]).
    ///
    /// The processor key covers the manifest, declared component bytes, canonical `options`,
    /// host access, the rpp version and every bundled file outside `source` (the plugin's own
    /// files). The generator key adds the bundled code, so editing a discovered module or
    /// adding or removing one reruns generators but not cached processor results. Top-level
    /// side effects of discovered modules are not tracked for processors.
    ///
    /// The bundle is cached under `<cache_dir>/bundles` while its inputs are unchanged.
    ///
    /// # Errors
    ///
    /// [`crate::Error::PluginLoad`] for bundling or evaluation failures (with
    /// source-mapped stacks), a missing default export, or invalid processor
    /// declarations; I/O and manifest errors otherwise.
    pub fn load(
        dir: impl AsRef<Path>,
        options: toml::Value,
        pack: PackInfo,
        limits: JsPluginLimits,
        access: RuntimeAccess,
        source: &Path,
        cache_dir: Option<&Path>,
    ) -> Result<Self> {
        let dir = dir.as_ref();
        let (manifest, manifest_source) = PluginManifest::load_with_source(dir)?;
        let id = manifest.id.clone();
        let load_error = |message: String| Error::PluginLoad {
            plugin: id.clone(),
            message,
        };

        if !super::is_js_entry(&manifest.entry) {
            return Err(load_error(format!(
                "entry `{}` is not a JavaScript or TypeScript module",
                manifest.entry
            )));
        }
        let root = dir.canonicalize().map_err(|e| Error::io(dir, e))?;
        let entry_path = root.join(&manifest.entry);
        let canonical_entry = entry_path
            .canonicalize()
            .map_err(|e| Error::io(&entry_path, e))?;
        if !canonical_entry.starts_with(&root) {
            return Err(load_error(format!(
                "entry `{}` escapes the plugin directory",
                manifest.entry
            )));
        }

        let discovery = Discovery::new(&manifest.discover).map_err(&load_error)?;
        let (bundle, authoring, source) = if manifest.discover.is_empty() {
            let request = BundleRequest {
                root: root.clone(),
                entry: "rpp:entry".into(),
                virtual_modules: virtual_modules(&format!("./{}", manifest.entry), false),
                ..Default::default()
            };
            let bundle = cached_bundle(cache_dir, &id, &request, || {
                rpp_js::bundle(&request).map_err(|e| e.to_string())
            })
            .map_err(&load_error)?;
            (bundle, BTreeSet::new(), None)
        } else {
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
                bundle_discovered(&manifest, &root, &source, &discovery, cache_dir)
                    .map_err(&load_error)?;
            (bundle, authoring, Some(source))
        };

        let limits = Limits {
            heap_bytes: limits.memory_limit,
            time: limits.execution_limit,
        };
        let mut pack_json = json!({ "name": pack.name });
        if let Some(description) = pack.description {
            pack_json["description"] = json!(description);
        }
        if let Some(format) = pack.format {
            pack_json["format"] = json!(format);
        }
        let init = json!({
            "plugin": id,
            "options": toml_to_json(&options),
            "pack": pack_json,
        });
        let description = describe(&id, &bundle, limits, &access).map_err(load_error)?;
        let processors = processor_defs(description.processors).map_err(load_error)?;
        let processor_key = compute_processor_key(
            &root,
            &manifest,
            &manifest_source,
            &bundle,
            source.as_deref(),
            &options,
            &access,
        )?;
        let cache_key = compute_cache_key(processor_key, &bundle);

        Ok(Self {
            shared: Arc::new(Shared {
                id,
                bundle,
                init,
                limits,
                access,
                processors,
                handlers: description.handlers,
                processor_key,
                cache_key,
                discovery,
                authoring,
                overrides: manifest.overrides.clone(),
            }),
        })
    }

    pub(super) fn shared(&self) -> &Shared {
        &self.shared
    }

    /// The access policy with a fresh phase tracker, so workers do not observe
    /// each other's phase transitions.
    pub(super) fn access(&self) -> RuntimeAccess {
        let mut access = self.shared.access.clone();
        access.phase = PhaseCell::new();
        access
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
        .is_some_and(|(_, ext)| matches!(ext, "ts" | "mts" | "cts"))
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
        ("#rpp".to_string(), super::SDK_FILES[0].1.to_string()),
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
    cache_dir: Option<&Path>,
) -> std::result::Result<(Bundle, BTreeSet<String>), String> {
    let entries = discovery.discover(source)?;

    let mut virtual_modules = virtual_modules("#plugin", true);
    virtual_modules.insert("#rpp/config".into(), CONFIG_SDK.into());
    virtual_modules.insert(
        "rpp:discovered".into(),
        discovered_module("", discovery, &entries),
    );
    let package = |entry: &str| BundlePackage {
        dir: plugin_root.to_path_buf(),
        entry: entry.to_string(),
    };
    let mut packages = BTreeMap::from([("#plugin".to_string(), package(&manifest.entry))]);
    if let Some(config) = &manifest.config {
        packages.insert(format!("#plugins/{}", manifest.id), package(config));
    }
    let request = BundleRequest {
        root: source.to_path_buf(),
        entry: "rpp:entry".into(),
        virtual_modules,
        packages,
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

/// Evaluate the bundle once and read what the plugin registered.
fn describe(
    id: &str,
    bundle: &Bundle,
    limits: Limits,
    access: &RuntimeAccess,
) -> std::result::Result<Description, String> {
    Engine::init_platform();
    let engine = instance::engine()?;
    let cancellation = Cancellation::new();
    let clock = instance::module_clock(id);
    let (mut runtime, _) = engine
        .load(id, bundle, limits, clock, &cancellation)
        .map_err(|e| e.to_string().trim_end().to_string())?;
    let mut host = JsHost::new(access, Instant::now() + limits.time, None);
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

fn compute_processor_key(
    root: &Path,
    manifest: &PluginManifest,
    manifest_source: &str,
    bundle: &Bundle,
    source: Option<&Path>,
    options: &toml::Value,
    access: &RuntimeAccess,
) -> Result<u64> {
    let mut writer = HashWriter::new();

    writer.write_str("rpp.js.processor.v1");
    writer.write_str(env!("CARGO_PKG_VERSION"));
    writer.write_str("inputs");
    for (input, hash) in bundle
        .input_hashes
        .iter()
        .filter(|(input, _)| source.is_none_or(|source| !input.starts_with(source)))
    {
        writer.write_str(&input.to_string_lossy());
        writer.write_u64(*hash);
    }
    writer.write_str("manifest");
    writer.write(manifest_source.as_bytes());
    for (name, component) in &manifest.components {
        let path = root.join(&component.module);
        let bytes = std::fs::read(&path).map_err(|e| Error::io(&path, e))?;
        writer.write_str("component");
        writer.write_str(name);
        writer.write_str(&component.module);
        writer.write(&bytes);
    }
    writer.write_str("options");
    writer.write(canonical_options_json(options).as_bytes());
    writer.write_str("host-access");
    let access_key =
        serde_json::to_vec(&(access.security, &access.permissions, &access.outputs))
            .map_err(|error| Error::Build(format!("failed to hash plugin host access: {error}")))?;
    writer.write(&access_key);

    Ok(writer.finish())
}

fn compute_cache_key(processor_key: u64, bundle: &Bundle) -> u64 {
    let mut writer = HashWriter::new();
    writer.write_str("rpp.js.plugin.v2");
    writer.write_u64(processor_key);
    writer.write(bundle.code.as_bytes());
    writer.finish()
}
