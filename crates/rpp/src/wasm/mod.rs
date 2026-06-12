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
//! There is an impedance mismatch in the generator phase:
//!
//! - `rpp-wasm` requires `HostCallbacks: Send + 'static` to be supplied **at
//!   instantiation time** (it is moved into the wasmtime `Store`).
//! - The engine, however, only hands us a `&mut dyn GeneratorHost` transiently,
//!   during a single [`PluginInstance::generate`] call. That borrow is **not**
//!   `'static` and cannot be moved into the store.
//!
//! We bridge this with a pair of synchronous channels. At instantiation the
//! instance is given a [`ChannelHost`] whose generator-phase callbacks send a
//! [`HostRequest`] (carrying a one-shot reply channel) to a request queue and
//! block on the reply. Log callbacks bypass the channel and route straight to
//! `tracing`, so they work in any phase without a live `GeneratorHost`.
//!
//! During [`PluginInstance::generate`] we:
//!
//! 1. spawn a scoped thread ([`std::thread::scope`]) that runs
//!    `WasmInstance::generate()` (the `Store`/instance is `Send`), and
//! 2. on the calling thread, service [`HostRequest`]s against the real
//!    `&mut dyn GeneratorHost` until the worker thread signals completion.
//!
//! The worker reports completion over a separate channel. The calling thread
//! then drains any queued fire-and-forget requests and joins the worker to
//! obtain its result.
//!
//! Lifecycle hooks (`on_build_start`/`on_build_finish`) are **no-ops**: the WIT
//! contract has no corresponding exports.

use std::path::Path;
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use parking_lot::Mutex;
use rpp_wasm::{HostCallbacks, LogLevel, ProcessResult, WasmEngine, WasmInstance};

use crate::error::{Error, Result};
use crate::manifest::PluginManifest;
use crate::model::{
    BuildStats, GeneratorHost, PackFile, PluginFactory, PluginInstance, ProcessOutcome,
    ProcessorDef,
};
use crate::util::hash::HashWriter;
use crate::util::path::validate_relative;

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
        let cache_key = compute_cache_key(&wasm_bytes, manifest_dir, &options_json)?;

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
        // Run the guest's `generate` on a scoped thread while this thread
        // services its host-callback requests. See the module docs for why.
        let plugin_id = self.plugin_id.clone();
        let instance = &mut self.instance;
        let requests = &self.requests;
        let host_error = Arc::clone(&self.host_error);

        std::thread::scope(|scope| {
            let (done_tx, done_rx) = crossbeam_channel::bounded(1);
            let worker = scope.spawn(move || {
                let result = instance.generate();
                let _ = done_tx.send(());
                result
            });

            loop {
                crossbeam_channel::select! {
                    recv(requests) -> req => match req {
                        Ok(req) => serve_request(host, req),
                        Err(_) => break,
                    },
                    recv(done_rx) -> _ => {
                        for req in requests.try_iter() {
                            serve_request(host, req);
                        }
                        break;
                    },
                }
            }

            let result = match worker.join() {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(Error::Generator {
                    plugin: plugin_id.clone(),
                    message: e.to_string(),
                }),
                Err(_) => Err(Error::Generator {
                    plugin: plugin_id.clone(),
                    message: "generator worker thread panicked".into(),
                }),
            };
            if let Some(message) = host_error.lock().take() {
                return Err(Error::Generator {
                    plugin: plugin_id.clone(),
                    message,
                });
            }
            result
        })
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

/// Service one host request against the real [`GeneratorHost`].
fn serve_request(host: &mut dyn GeneratorHost, req: HostRequest) {
    match req {
        HostRequest::ListFiles { pattern, reply } => {
            let _ = reply.send(host.list_files(pattern.as_deref()));
        }
        HostRequest::ReadFile { path, reply } => {
            let _ = reply.send(host.read_file(&path));
        }
        HostRequest::ReadSource { path, reply } => {
            let _ = reply.send(host.read_source(&path));
        }
        HostRequest::Emit {
            path,
            contents,
            reply,
        } => {
            host.emit(&path, contents);
            let _ = reply.send(());
        }
        HostRequest::Remove { path, reply } => {
            host.remove(&path);
            let _ = reply.send(());
        }
    }
}

/// A request from the guest's host callbacks to the engine's
/// [`GeneratorHost`], carrying a one-shot reply channel where needed.
enum HostRequest {
    ListFiles {
        pattern: Option<String>,
        reply: Sender<Vec<String>>,
    },
    ReadFile {
        path: String,
        reply: Sender<Option<Vec<u8>>>,
    },
    ReadSource {
        path: String,
        reply: Sender<Option<Vec<u8>>>,
    },
    Emit {
        path: String,
        contents: Vec<u8>,
        reply: Sender<()>,
    },
    Remove {
        path: String,
        reply: Sender<()>,
    },
}

/// The [`HostCallbacks`] implementation handed to `rpp-wasm` at instantiation.
///
/// Generator-phase callbacks marshal a [`HostRequest`] onto the channel and
/// block on the reply; `log` routes directly to `tracing`. The host
/// (`rpp-wasm`) already suppresses generator-phase callbacks outside the
/// `generate` window, so `requests.send` only fires while a servicer is active.
struct ChannelHost {
    plugin_id: String,
    requests: Sender<HostRequest>,
    error: Arc<Mutex<Option<String>>>,
}

impl ChannelHost {
    /// Send a request expecting a reply, returning the (default on disconnect).
    fn request<T: Default>(&self, make: impl FnOnce(Sender<T>) -> HostRequest) -> T {
        let (reply_tx, reply_rx) = crossbeam_channel::bounded::<T>(1);
        if self.requests.send(make(reply_tx)).is_err() {
            return T::default();
        }
        reply_rx.recv().unwrap_or_default()
    }

    fn valid_path(&self, path: &str) -> bool {
        match validate_relative(path) {
            Ok(()) => true,
            Err(message) => {
                *self.error.lock() = Some(message);
                false
            }
        }
    }
}

impl HostCallbacks for ChannelHost {
    fn log(&mut self, level: LogLevel, message: &str) {
        log_message(&self.plugin_id, level, message);
    }

    fn list_files(&mut self, pattern: Option<&str>) -> Vec<String> {
        let pattern = pattern.map(str::to_string);
        self.request(|reply| HostRequest::ListFiles { pattern, reply })
    }

    fn read_file(&mut self, path: &str) -> Option<Vec<u8>> {
        if !self.valid_path(path) {
            return None;
        }
        let path = path.to_string();
        self.request(|reply| HostRequest::ReadFile { path, reply })
    }

    fn read_source(&mut self, path: &str) -> Option<Vec<u8>> {
        if !self.valid_path(path) {
            return None;
        }
        let path = path.to_string();
        self.request(|reply| HostRequest::ReadSource { path, reply })
    }

    fn emit_file(&mut self, path: &str, contents: Vec<u8>) {
        if !self.valid_path(path) {
            return;
        }
        self.request(|reply| HostRequest::Emit {
            path: path.to_string(),
            contents,
            reply,
        });
    }

    fn remove_file(&mut self, path: &str) {
        if !self.valid_path(path) {
            return;
        }
        self.request(|reply| HostRequest::Remove {
            path: path.to_string(),
            reply,
        });
    }
}

/// Route a guest log message to `tracing` (when enabled) or stderr.
#[allow(unused_variables)]
fn log_message(plugin_id: &str, level: LogLevel, message: &str) {
    #[cfg(feature = "tracing")]
    {
        match level {
            LogLevel::Debug => tracing::debug!(plugin = plugin_id, "{message}"),
            LogLevel::Info => tracing::info!(plugin = plugin_id, "{message}"),
            LogLevel::Warn => tracing::warn!(plugin = plugin_id, "{message}"),
            LogLevel::Error => tracing::error!(plugin = plugin_id, "{message}"),
        }
    }
    #[cfg(not(feature = "tracing"))]
    {
        let label = match level {
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
        };
        eprintln!("[{label}] {plugin_id}: {message}");
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
fn compute_cache_key(wasm_bytes: &[u8], manifest_dir: &Path, options_json: &str) -> Result<u64> {
    let mut writer = HashWriter::new();
    writer.write_str("rpp.wasm.plugin.v1");

    writer.write_str("module");
    writer.write(wasm_bytes);

    let manifest_path = manifest_dir.join("plugin.toml");
    let manifest = std::fs::read(&manifest_path).map_err(|e| Error::io(&manifest_path, e))?;
    writer.write_str("plugin.toml");
    writer.write(&manifest);

    // Canonicalize via sorted-key JSON so formatting differences are ignored.
    writer.write_str("options");
    writer.write(canonical_json_str(options_json).as_bytes());

    Ok(writer.finish())
}

/// Re-serialize a JSON string into canonical (sorted-key) form.
fn canonical_json_str(json: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(value) => canonical_json(&value),
        Err(_) => json.to_string(),
    }
}

fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys
                .into_iter()
                .map(|k| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap_or_default(),
                        canonical_json(&map[k])
                    )
                })
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        serde_json::Value::Array(arr) => {
            let parts: Vec<String> = arr.iter().map(canonical_json).collect();
            format!("[{}]", parts.join(","))
        }
        other => other.to_string(),
    }
}
