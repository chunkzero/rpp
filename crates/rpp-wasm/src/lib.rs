//! WASIp2 component plugin host for rpp, built on wasmtime.
//!
//! This crate is a **standalone** host for `rpp:plugin@0.1.0` WASIp2 component
//! plugins. It does not depend on the `rpp` core crate; the core wraps this
//! host behind its `wasm` feature with an adapter implementing its plugin
//! traits (see `docs/SPEC.md` sections 2–3 and 5).
//!
//! # Overview
//!
//! * [`WasmEngine`] owns a shared, thread-safe wasmtime [`Engine`] configured
//!   for the component model with epoch-based interruption. It also manages a
//!   background "epoch ticker" thread so per-call deadlines are enforced.
//! * [`WasmEngine::load`] compiles a component once into a [`CompiledPlugin`]
//!   and caches its [`PluginInfo`] (obtained from a throwaway instantiation).
//! * [`CompiledPlugin::instantiate`] creates a fresh, isolated [`WasmInstance`]
//!   bound to a set of [`HostCallbacks`]. The WASI context has **no** filesystem
//!   preopens, **no** network, and **no** environment variables; stdout/stderr
//!   are inherited for debugging.
//! * [`WasmInstance::process`] and [`WasmInstance::generate`] drive the guest.
//!
//! # Generator-phase host functions
//!
//! The host functions `list_files`, `read_file`, `read_source`, `emit_file`,
//! and `remove_file` are only live while [`WasmInstance::generate`] is running.
//! When a guest calls them outside the generate phase the host returns
//! **empty/no-op** results (empty list, `None`, dropped writes) rather than
//! trapping the guest. `log` is always routed to the callback.
//!
//! # Limits
//!
//! [`Limits`] controls the per-call epoch deadline (default 60s) and the linear
//! memory cap (default 512 MiB) enforced via wasmtime's `StoreLimits`.

#![deny(missing_docs)]

mod bindings;
mod error;

use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::JoinHandle,
    time::Duration,
};

use wasmtime::{
    component::{Component, Linker, ResourceTable},
    Config, Engine, Store, StoreLimits, StoreLimitsBuilder,
};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

pub use crate::error::{Error, Result};

use crate::bindings::{guest, RppPlugin};

/// Default per-call epoch deadline.
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(60);
/// Default linear-memory cap (512 MiB).
pub const DEFAULT_MEMORY_LIMIT: usize = 512 * 1024 * 1024;

/// Granularity of the background epoch ticker. The per-call deadline is
/// rounded up to a whole number of ticks.
const EPOCH_TICK: Duration = Duration::from_millis(50);

/// Severity level for host-routed log messages.
///
/// Mirrors the `log-level` enum in the WIT `host` interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    /// Verbose diagnostic output.
    Debug,
    /// Informational messages.
    Info,
    /// Warnings that do not abort the build.
    Warn,
    /// Errors.
    Error,
}

/// Resource limits applied to every [`WasmInstance`].
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Maximum wall-clock time a single guest call may run before it is
    /// interrupted with [`Error::Timeout`].
    pub deadline: Duration,
    /// Maximum linear-memory size (in bytes) any guest memory may grow to.
    pub memory_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            deadline: DEFAULT_DEADLINE,
            memory_bytes: DEFAULT_MEMORY_LIMIT,
        }
    }
}

impl Limits {
    /// Number of epoch ticks corresponding to [`Limits::deadline`] (at least 1).
    fn epoch_ticks(&self) -> u64 {
        let nanos = self.deadline.as_nanos().max(1);
        let tick = EPOCH_TICK.as_nanos().max(1);
        nanos.div_ceil(tick).max(1) as u64
    }
}

/// A declared processor: its name, the globs it matches, and its priority
/// (lower runs first).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessorDef {
    /// Processor name (unique within the plugin).
    pub name: String,
    /// Glob patterns the processor matches.
    pub patterns: Vec<String>,
    /// Priority; lower values run first. Ties broken by plugin order.
    pub priority: i32,
}

/// Static description of a plugin, cached after a single throwaway
/// instantiation during [`WasmEngine::load`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginInfo {
    /// Plugin identifier, matching `^[a-z0-9][a-z0-9_-]*$`.
    pub id: String,
    /// Plugin version (valid semver).
    pub version: String,
    /// The processors this plugin declares.
    pub processors: Vec<ProcessorDef>,
    /// Whether the plugin exports a generator.
    pub has_generator: bool,
}

/// Outcome of running a processor over a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessResult {
    /// The file was not changed.
    Unchanged,
    /// The file was changed (and possibly renamed).
    Modified {
        /// The (possibly new) output path.
        path: String,
        /// The new file contents.
        contents: Vec<u8>,
    },
    /// The file should be dropped from the output.
    Dropped,
}

/// Host functions a [`WasmInstance`] may call back into.
///
/// Mirrors the WIT `host` interface. The generator-phase methods are only
/// invoked while [`WasmInstance::generate`] is running; the host suppresses
/// calls made outside that window (see crate docs), so an implementation does
/// not need to guard against that itself.
pub trait HostCallbacks: Send {
    /// Emit a log message. Always invoked, in any phase.
    fn log(&mut self, level: LogLevel, message: &str);

    /// List output files matching an optional glob pattern.
    fn list_files(&mut self, pattern: Option<&str>) -> Vec<String>;

    /// Read a processed output file. `None` if it does not exist.
    fn read_file(&mut self, path: &str) -> Option<Vec<u8>>;

    /// Read a raw source file. `None` if it does not exist.
    fn read_source(&mut self, path: &str) -> Option<Vec<u8>>;

    /// Add or overwrite an output file.
    fn emit_file(&mut self, path: &str, contents: Vec<u8>);

    /// Remove an output file.
    fn remove_file(&mut self, path: &str);
}

/// Per-store data: WASI context, resource table, memory limits, the host
/// callbacks, and a flag tracking whether the generate phase is active.
pub(crate) struct StoreData {
    wasi: WasiCtx,
    table: ResourceTable,
    limits: StoreLimits,
    pub(crate) host: Box<dyn HostCallbacks>,
    pub(crate) in_generate: bool,
}

impl WasiView for StoreData {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

/// Background thread that increments the engine epoch on a fixed cadence so
/// per-call deadlines are enforced. Stops and joins on drop.
struct EpochTicker {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl EpochTicker {
    fn spawn(engine: Engine) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let handle = std::thread::Builder::new()
            .name("rpp-wasm-epoch".into())
            .spawn(move || {
                while !stop_thread.load(Ordering::Relaxed) {
                    std::thread::sleep(EPOCH_TICK);
                    engine.increment_epoch();
                }
            })?;
        Ok(Self {
            stop,
            handle: Some(handle),
        })
    }
}

impl Drop for EpochTicker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Shared wasmtime engine plus the configuration used to instantiate plugins.
///
/// Cloning is cheap (it shares the underlying [`Engine`] and ticker) and an
/// engine is safe to use from multiple threads.
#[derive(Clone)]
pub struct WasmEngine {
    engine: Engine,
    linker: Arc<Linker<StoreData>>,
    limits: Limits,
    // Kept alive for the lifetime of the engine; the ticker stops on drop.
    _ticker: Arc<EpochTicker>,
}

impl WasmEngine {
    /// Construct an engine with default [`Limits`].
    pub fn new() -> Result<Self> {
        Self::with_limits(Limits::default())
    }

    /// Construct an engine with custom [`Limits`].
    pub fn with_limits(limits: Limits) -> Result<Self> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.epoch_interruption(true);
        let engine = Engine::new(&config).map_err(Error::Engine)?;

        let mut linker: Linker<StoreData> = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(Error::Engine)?;
        bindings::rpp::plugin::host::add_to_linker::<_, wasmtime::component::HasSelf<StoreData>>(
            &mut linker,
            |data: &mut StoreData| data,
        )
        .map_err(Error::Engine)?;

        let ticker = EpochTicker::spawn(engine.clone()).map_err(Error::EpochTicker)?;

        Ok(Self {
            engine,
            linker: Arc::new(linker),
            limits,
            _ticker: Arc::new(ticker),
        })
    }

    /// The limits applied to instances created from this engine.
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Compile a component from disk and cache its [`PluginInfo`].
    ///
    /// This performs a throwaway instantiation to call `get-info` once. The
    /// returned [`CompiledPlugin`] is cheap to re-instantiate.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`]/[`Error::Compile`] on read/compile failure,
    /// [`Error::Instantiate`]/[`Error::Trap`] if the validation instance fails,
    /// or [`Error::InvalidInfo`] if the plugin reports a malformed id/version.
    pub fn load(&self, wasm_path: &Path) -> Result<CompiledPlugin> {
        let bytes = std::fs::read(wasm_path).map_err(|source| Error::Io {
            path: wasm_path.to_path_buf(),
            source,
        })?;
        let component = Component::new(&self.engine, &bytes).map_err(|source| Error::Compile {
            path: wasm_path.to_path_buf(),
            source,
        })?;

        let info = self.validate_info(&component)?;

        Ok(CompiledPlugin {
            engine: self.clone(),
            component,
            info,
            path: wasm_path.to_path_buf(),
        })
    }

    /// Instantiate the component once with no-op callbacks to read `get-info`.
    fn validate_info(&self, component: &Component) -> Result<PluginInfo> {
        let mut store = self.new_store(Box::new(NoopCallbacks));
        let deadline = self.limits.deadline;
        let instance = RppPlugin::instantiate(&mut store, component, &self.linker)
            .map_err(|error| map_timeout(error, deadline))?;
        store.set_epoch_deadline(self.limits.epoch_ticks());
        let raw = instance
            .rpp_plugin_guest()
            .call_get_info(&mut store)
            .map_err(|error| map_timeout(error, deadline))?;
        let info = convert_info(raw);
        validate_plugin_info(&info)?;
        Ok(info)
    }

    /// Build a fresh store with an empty WASI context and the configured limits.
    fn new_store(&self, host: Box<dyn HostCallbacks>) -> Store<StoreData> {
        // Empty WASI context: no preopens, no env, no network. Inherit
        // stdout/stderr so guest debug prints surface during development.
        let wasi = WasiCtxBuilder::new()
            .inherit_stdout()
            .inherit_stderr()
            .build();

        let store_limits = StoreLimitsBuilder::new()
            .memory_size(self.limits.memory_bytes)
            .memories(8)
            .tables(64)
            .table_elements(1_000_000)
            .instances(128)
            .build();

        let data = StoreData {
            wasi,
            table: ResourceTable::new(),
            limits: store_limits,
            host,
            in_generate: false,
        };

        let mut store = Store::new(&self.engine, data);
        store.limiter(|data| &mut data.limits);
        // Trap (rather than yield) when the deadline is exceeded, and arm the
        // first deadline. It is re-armed before every guest call.
        store.epoch_deadline_trap();
        store.set_epoch_deadline(self.limits.epoch_ticks());
        store
    }
}

/// A compiled component plus its cached static metadata.
///
/// `Send + Sync` and cheap to [`instantiate`](CompiledPlugin::instantiate)
/// repeatedly (e.g. once per worker thread).
#[derive(Clone)]
pub struct CompiledPlugin {
    engine: WasmEngine,
    component: Component,
    info: PluginInfo,
    path: PathBuf,
}

impl CompiledPlugin {
    /// The plugin's cached static description.
    pub fn info(&self) -> &PluginInfo {
        &self.info
    }

    /// The path the component was loaded from.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Instantiate a fresh, isolated instance and run `configure` once.
    ///
    /// `options_json` is the plugin's options encoded as JSON (`{}` if none).
    /// `host` receives all host callbacks for this instance.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Instantiate`] if linking fails, or
    /// [`Error::Trap`]/[`Error::Timeout`] if `configure` traps or times out.
    pub fn instantiate(
        &self,
        options_json: &str,
        host: impl HostCallbacks + 'static,
    ) -> Result<WasmInstance> {
        let epoch_ticks = self.engine.limits.epoch_ticks();
        let deadline = self.engine.limits.deadline;
        let mut store = self.engine.new_store(Box::new(host));
        let instance = RppPlugin::instantiate(&mut store, &self.component, &self.engine.linker)
            .map_err(|error| map_timeout(error, deadline))?;

        store.set_epoch_deadline(epoch_ticks);
        instance
            .rpp_plugin_guest()
            .call_configure(&mut store, options_json)
            .map_err(|error| map_timeout(error, deadline))?;

        Ok(WasmInstance {
            store,
            instance,
            epoch_ticks,
            deadline,
        })
    }
}

/// A live, isolated plugin instance bound to one set of host callbacks.
///
/// Not `Sync`: drive a single instance from one thread at a time. Create one
/// instance per worker thread via [`CompiledPlugin::instantiate`].
pub struct WasmInstance {
    store: Store<StoreData>,
    instance: RppPlugin,
    epoch_ticks: u64,
    deadline: Duration,
}

impl WasmInstance {
    /// Run a named processor over a single file.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Trap`]/[`Error::Timeout`] on a trap/timeout, or
    /// [`Error::GuestError`] for a guest-reported failure (e.g. an unknown
    /// processor name rejected by the guest).
    pub fn process(
        &mut self,
        processor: &str,
        path: &str,
        contents: &[u8],
    ) -> Result<ProcessResult> {
        let file = guest::FileData {
            path: path.to_string(),
            contents: contents.to_vec(),
        };
        self.store.set_epoch_deadline(self.epoch_ticks);
        let result = self
            .instance
            .rpp_plugin_guest()
            .call_process(&mut self.store, processor, &file)
            .map_err(|e| map_timeout(e, self.deadline))?;
        match result {
            Ok(r) => Ok(convert_process_result(r)),
            Err(msg) => Err(Error::GuestError(msg)),
        }
    }

    /// Run the generator phase. During this call the generator-phase host
    /// callbacks are live.
    ///
    /// # Errors
    ///
    /// [`Error::Trap`]/[`Error::Timeout`] on trap/timeout, or
    /// [`Error::GuestError`] for a guest-reported failure.
    pub fn generate(&mut self) -> Result<()> {
        self.store.data_mut().in_generate = true;
        self.store.set_epoch_deadline(self.epoch_ticks);
        let result = self
            .instance
            .rpp_plugin_guest()
            .call_generate(&mut self.store)
            .map_err(|e| map_timeout(e, self.deadline));
        self.store.data_mut().in_generate = false;
        match result? {
            Ok(()) => Ok(()),
            Err(msg) => Err(Error::GuestError(msg)),
        }
    }
}

/// No-op callbacks used for the throwaway validation instantiation.
struct NoopCallbacks;

impl HostCallbacks for NoopCallbacks {
    fn log(&mut self, _level: LogLevel, _message: &str) {}
    fn list_files(&mut self, _pattern: Option<&str>) -> Vec<String> {
        Vec::new()
    }
    fn read_file(&mut self, _path: &str) -> Option<Vec<u8>> {
        None
    }
    fn read_source(&mut self, _path: &str) -> Option<Vec<u8>> {
        None
    }
    fn emit_file(&mut self, _path: &str, _contents: Vec<u8>) {}
    fn remove_file(&mut self, _path: &str) {}
}

/// Convert a wasmtime call error, distinguishing epoch-deadline traps
/// (timeouts) from ordinary traps. Used where the per-call deadline is known.
fn map_timeout(err: wasmtime::Error, deadline: Duration) -> Error {
    if let Some(trap) = err.downcast_ref::<wasmtime::Trap>() {
        if *trap == wasmtime::Trap::Interrupt {
            return Error::Timeout(deadline);
        }
    }
    Error::Trap(err)
}

fn convert_info(info: guest::PluginInfo) -> PluginInfo {
    PluginInfo {
        id: info.id,
        version: info.version,
        processors: info
            .processors
            .into_iter()
            .map(|p| ProcessorDef {
                name: p.name,
                patterns: p.patterns,
                priority: p.priority,
            })
            .collect(),
        has_generator: info.has_generator,
    }
}

fn convert_process_result(r: guest::ProcessResult) -> ProcessResult {
    match r {
        guest::ProcessResult::Unchanged => ProcessResult::Unchanged,
        guest::ProcessResult::Modified(f) => ProcessResult::Modified {
            path: f.path,
            contents: f.contents,
        },
        guest::ProcessResult::Dropped => ProcessResult::Dropped,
    }
}

/// Validate a plugin id (`^[a-z0-9][a-z0-9_-]*$`) and a semver version.
fn validate_plugin_info(info: &PluginInfo) -> Result<()> {
    if !is_valid_id(&info.id) {
        return Err(Error::InvalidInfo(format!(
            "plugin id {:?} does not match ^[a-z0-9][a-z0-9_-]*$",
            info.id
        )));
    }
    if semver::Version::parse(&info.version).is_err() {
        return Err(Error::InvalidInfo(format!(
            "plugin version {:?} is not valid semver",
            info.version
        )));
    }
    Ok(())
}

fn is_valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}
