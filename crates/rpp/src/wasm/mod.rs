//! WASM (WASIp2 component) plugin adapter (spec §5).
//!
//! This module wraps the standalone [`rpp_wasm`] host crate behind the `wasm`
//! cargo feature and adapts it to the runtime-agnostic plugin model in
//! [`crate::model`]. It provides:
//!
//! - [`WasmPluginFactory`]: a [`PluginFactory`] over a compiled component. It
//!   exposes the component's id/version/processors/has-generator (read from the
//!   host's [`rpp_wasm::PluginInfo`]) and computes a deterministic `cache_key`
//!   (xxh3 over the `.wasm` bytes, the `plugin.toml`, and the canonicalized
//!   options).
//! - [`WasmPluginInstance`]: a [`PluginInstance`] that drives a single
//!   [`rpp_wasm::WasmInstance`], mapping [`rpp_wasm::ProcessResult`] to
//!   [`ProcessOutcome`] and bridging the generator phase.
//!
//! # The generator bridge
//!
//! Generator-phase host callbacks are bridged via synchronous channels; see
//! [`generator_bridge`].

mod generator_bridge;

use std::path::Path;
use std::sync::Arc;

use crossbeam_channel::Receiver;
use parking_lot::Mutex;
use rpp_wasm::{ProcessResult, WasmEngine, WasmInstance};

use crate::error::{Error, Result};
use crate::manifest::PluginManifest;
use crate::model::{
    BuildStats, GeneratorHost, PackFile, PluginFactory, PluginInstance, ProcessOutcome,
    ProcessorDef,
};
use crate::util::canonical::canonical_options_json;
use crate::util::hash::HashWriter;
use crate::util::path::validate_relative;

use self::generator_bridge::{run_generate, ChannelHost, HostRequest};

/// A [`PluginFactory`] backed by a compiled WASIp2 component.
///
/// Cloning is cheap: the underlying [`rpp_wasm::CompiledPlugin`] shares its
/// engine and component. Each [`PluginFactory::instantiate`] call creates a
/// fresh, isolated [`rpp_wasm::WasmInstance`].
#[derive(Clone)]
pub struct WasmPluginFactory {
    compiled: rpp_wasm::CompiledPlugin,
    id: String,
    version: String,
    processors: Vec<ProcessorDef>,
    has_generator: bool,
    options_json: String,
    cache_key: u64,
}

impl WasmPluginFactory {
    /// Load a WASM plugin from its package directory.
    ///
    /// `manifest_dir` is the plugin package directory; `manifest` is its parsed
    /// `plugin.toml` (whose `module` field names the component file relative to
    /// the directory). `options` are the per-plugin options from `rpp.toml`.
    ///
    /// The plugin is compiled via `engine.load(..)`, which performs a throwaway
    /// instantiation to read `get-info`. The reported id/version/processors are
    /// surfaced through the [`PluginFactory`] trait.
    pub fn load(
        engine: &WasmEngine,
        manifest_dir: impl AsRef<Path>,
        manifest: &PluginManifest,
        options: toml::Value,
    ) -> Result<Self> {
        let manifest_dir = manifest_dir.as_ref();

        let module = manifest.module.as_deref().ok_or_else(|| Error::Manifest {
            path: manifest_dir.join("plugin.toml"),
            message: "`module` is required for wasm runtime".into(),
        })?;
        let wasm_path = manifest_dir.join(module);

        let wasm_bytes = std::fs::read(&wasm_path).map_err(|e| Error::io(&wasm_path, e))?;

        let compiled = engine.load(&wasm_path).map_err(|e| Error::PluginLoad {
            plugin: manifest.id.clone(),
            message: format!("failed to load wasm component `{module}`: {e}"),
        })?;

        let info = compiled.info();
        if info.id != manifest.id || info.version != manifest.version.to_string() {
            return Err(Error::PluginLoad {
                plugin: manifest.id.clone(),
                message: format!(
                    "component reports {} v{}, but plugin.toml declares {} v{}",
                    info.id, info.version, manifest.id, manifest.version
                ),
            });
        }
        let mut processor_names = std::collections::BTreeSet::new();
        for processor in &info.processors {
            if processor.name.is_empty() || !processor_names.insert(&processor.name) {
                return Err(Error::PluginLoad {
                    plugin: manifest.id.clone(),
                    message: format!(
                        "component reports an empty or duplicate processor name `{}`",
                        processor.name
                    ),
                });
            }
        }
        let processors: Vec<ProcessorDef> = info
            .processors
            .iter()
            .map(|p| ProcessorDef {
                name: p.name.clone(),
                patterns: p.patterns.clone(),
                priority: p.priority,
            })
            .collect();

        let options_json = options_to_json(&options);
        let cache_key = compute_cache_key(&wasm_bytes, manifest_dir, &options)?;

        Ok(WasmPluginFactory {
            id: info.id.clone(),
            version: info.version.clone(),
            has_generator: info.has_generator,
            processors,
            compiled,
            options_json,
            cache_key,
        })
    }
}

impl PluginFactory for WasmPluginFactory {
    fn id(&self) -> &str {
        &self.id
    }

    fn version(&self) -> &str {
        &self.version
    }

    fn cache_key(&self) -> u64 {
        self.cache_key
    }

    fn processors(&self) -> &[ProcessorDef] {
        &self.processors
    }

    fn has_generator(&self) -> bool {
        self.has_generator
    }

    fn instantiate(&self) -> Result<Box<dyn PluginInstance>> {
        // Bounded-1 channels: a generator request blocks for its reply, so there
        // is never more than one in flight.
        let (req_tx, req_rx) = crossbeam_channel::bounded::<HostRequest>(1);
        let host_error = Arc::new(Mutex::new(None));
        let host = ChannelHost {
            plugin_id: self.id.clone(),
            requests: req_tx,
            error: Arc::clone(&host_error),
        };

        let instance = self
            .compiled
            .instantiate(&self.options_json, host)
            .map_err(|e| Error::PluginLoad {
                plugin: self.id.clone(),
                message: format!("failed to instantiate wasm plugin: {e}"),
            })?;

        Ok(Box::new(WasmPluginInstance {
            plugin_id: self.id.clone(),
            instance,
            requests: req_rx,
            host_error,
        }))
    }
}

/// A live WASM plugin instance bound to one [`rpp_wasm::WasmInstance`].
pub struct WasmPluginInstance {
    plugin_id: String,
    instance: WasmInstance,
    /// The host-request receiver paired with the [`ChannelHost`] inside the
    /// instance's store. Serviced during [`PluginInstance::generate`].
    requests: Receiver<HostRequest>,
    host_error: Arc<Mutex<Option<String>>>,
}

impl PluginInstance for WasmPluginInstance {
    fn process(&mut self, processor: &str, file: &mut PackFile) -> Result<ProcessOutcome> {
        let result = self
            .instance
            .process(processor, &file.path, &file.contents)
            .map_err(|e| Error::Processor {
                plugin: self.plugin_id.clone(),
                processor: processor.to_string(),
                file: file.path.clone(),
                message: e.to_string(),
            })?;

        match result {
            ProcessResult::Unchanged => Ok(ProcessOutcome::Unchanged),
            ProcessResult::Modified { path, contents } => {
                validate_relative(&path).map_err(|message| Error::Processor {
                    plugin: self.plugin_id.clone(),
                    processor: processor.to_string(),
                    file: file.path.clone(),
                    message,
                })?;
                file.path = path;
                file.contents = contents;
                Ok(ProcessOutcome::Modified)
            }
            ProcessResult::Dropped => Ok(ProcessOutcome::Dropped),
        }
    }

    fn generate(&mut self, host: &mut dyn GeneratorHost) -> Result<()> {
        run_generate(
            &self.plugin_id,
            &self.requests,
            Arc::clone(&self.host_error),
            host,
            || self.instance.generate(),
        )
    }

    fn on_build_start(&mut self) -> Result<()> {
        // No corresponding WIT export.
        Ok(())
    }

    fn on_build_finish(&mut self, _stats: &BuildStats) -> Result<()> {
        // No corresponding WIT export.
        Ok(())
    }
}

/// Encode plugin options as a JSON object string for the guest's `configure`.
fn options_to_json(options: &toml::Value) -> String {
    let json: serde_json::Value = serde_json::to_value(options).unwrap_or(serde_json::Value::Null);
    // The guest expects an object; empty/null options become `{}`.
    match json {
        serde_json::Value::Null => "{}".to_string(),
        other => serde_json::to_string(&other).unwrap_or_else(|_| "{}".to_string()),
    }
}

/// Compute the cache key: xxh3 over the `.wasm` bytes, the raw `plugin.toml`,
/// and the canonicalized options.
fn compute_cache_key(wasm_bytes: &[u8], manifest_dir: &Path, options: &toml::Value) -> Result<u64> {
    let mut writer = HashWriter::new();
    writer.write_str("rpp.wasm.plugin.v1");

    writer.write_str("module");
    writer.write(wasm_bytes);

    let manifest_path = manifest_dir.join("plugin.toml");
    let manifest = std::fs::read(&manifest_path).map_err(|e| Error::io(&manifest_path, e))?;
    writer.write_str("plugin.toml");
    writer.write(&manifest);

    writer.write_str("options");
    writer.write(canonical_options_json(options).as_bytes());

    Ok(writer.finish())
}
