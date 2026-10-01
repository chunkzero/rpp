//! [`JsPluginFactory`]: bundling and validation on the main thread.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rpp_js::{Bundle, BundleRequest, Call, Cancellation, Engine, Limits};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::host::{PackInfo, PhaseCell, RuntimeAccess};
use crate::js::host::JsHost;
use crate::js::instance::{self, JsPluginInstance};
use crate::manifest::PluginManifest;
use crate::model::{PluginFactory, PluginInstance, ProcessorDef};
use crate::util::canonical::canonical_options_json;
use crate::util::glob::GlobSet;
use crate::util::hash::HashWriter;
use crate::util::json_toml::{toml_to_json, Datetimes};

const RUNTIME_SOURCE: &str = include_str!("sdk/runtime.ts");

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
    cache_key: u64,
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
    /// Read `rpp.json` (or `plugin.toml`) in `dir`, bundle its entry, evaluate it once to read its
    /// processors and handlers, and compute the cache key.
    ///
    /// The cache key covers the bundled code, the manifest, declared component
    /// bytes, canonical `options`, host access and the rpp version.
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

        let bundle = rpp_js::bundle(&BundleRequest {
            root: root.clone(),
            entry: "rpp:entry".into(),
            virtual_modules: virtual_modules(&manifest.entry),
            ..Default::default()
        })
        .map_err(|e| load_error(e.to_string()))?;

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
            "options": toml_to_json(&options, Datetimes::Strings),
            "pack": pack_json,
        });
        let description = describe(&id, &bundle, limits, &access).map_err(load_error)?;
        let processors = processor_defs(description.processors).map_err(load_error)?;
        let cache_key = compute_cache_key(
            &root,
            &manifest,
            &manifest_source,
            &bundle,
            &options,
            &access,
        )?;

        Ok(Self {
            shared: Arc::new(Shared {
                id,
                bundle,
                init,
                limits,
                access,
                processors,
                handlers: description.handlers,
                cache_key,
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

    fn instantiate(&self) -> Result<Box<dyn PluginInstance>> {
        Ok(Box::new(JsPluginInstance::new(self.clone())?))
    }
}

fn virtual_modules(entry: &str) -> BTreeMap<String, String> {
    let entry_module = format!(
        "import plugin from {};\nimport {{ register }} from \"rpp:runtime\";\nregister(plugin);\nexport * from \"rpp:runtime\";\n",
        Value::String(format!("./{entry}"))
    );
    BTreeMap::from([
        ("#rpp".to_string(), super::SDK_FILES[0].1.to_string()),
        ("rpp:runtime".to_string(), RUNTIME_SOURCE.to_string()),
        ("rpp:entry".to_string(), entry_module),
    ])
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

fn compute_cache_key(
    root: &Path,
    manifest: &PluginManifest,
    manifest_source: &str,
    bundle: &Bundle,
    options: &toml::Value,
    access: &RuntimeAccess,
) -> Result<u64> {
    let mut writer = HashWriter::new();

    writer.write_str("rpp.js.plugin.v1");
    writer.write_str(env!("CARGO_PKG_VERSION"));
    writer.write(bundle.code.as_bytes());
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
