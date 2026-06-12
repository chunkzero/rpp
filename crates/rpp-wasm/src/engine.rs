//! Shared wasmtime engine and compiled plugin handles.

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
    component::{Component, Linker},
    Config, Engine, Store,
};
use wasmtime_wasi::p2;

use crate::bindings::{self, RppPlugin};
use crate::convert::{convert_info, validate_plugin_info};
use crate::error::{Error, Result};
use crate::instance::{map_timeout, NoopCallbacks};
use crate::store::StoreData;
use crate::types::{HostCallbacks, Limits, PluginInfo};
use crate::WasmInstance;

/// Granularity of the background epoch ticker. The per-call deadline is
/// rounded up to a whole number of ticks.
const EPOCH_TICK: Duration = Duration::from_millis(50);

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

impl Limits {
    /// Number of epoch ticks corresponding to [`Limits::deadline`] (at least 1).
    pub(crate) fn epoch_ticks(&self) -> u64 {
        let nanos = self.deadline.as_nanos().max(1);
        let tick = EPOCH_TICK.as_nanos().max(1);
        nanos.div_ceil(tick).max(1) as u64
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
        p2::add_to_linker_sync(&mut linker).map_err(Error::Engine)?;
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
    pub(crate) fn new_store(&self, host: Box<dyn HostCallbacks>) -> Store<StoreData> {
        let data = StoreData::new(host, self.limits.memory_bytes);
        let mut store = Store::new(&self.engine, data);
        store.limiter(|data| &mut data.limits);
        // Trap (rather than yield) when the deadline is exceeded, and arm the
        // first deadline. It is re-armed before every guest call.
        store.epoch_deadline_trap();
        store.set_epoch_deadline(self.limits.epoch_ticks());
        store
    }

    pub(crate) fn linker(&self) -> &Linker<StoreData> {
        &self.linker
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
        let instance = RppPlugin::instantiate(&mut store, &self.component, self.engine.linker())
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
