//! Shared wasmtime engine.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread::JoinHandle;
use std::time::Duration;

use sha2::{Digest, Sha256};
use wasmtime::component::Component;
use wasmtime::{Config, Engine, Store};

use crate::store::StoreData;
use crate::types::{Limits, Permissions};
use crate::{CompiledComponent, Error, Result};

const EPOCH_TICK: Duration = Duration::from_millis(50);

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

pub(crate) fn ticks_for(deadline: Duration) -> u64 {
    let nanos = deadline.as_nanos().max(1);
    nanos.div_ceil(EPOCH_TICK.as_nanos().max(1)).max(1) as u64
}

impl Limits {
    pub(crate) fn epoch_ticks(&self) -> u64 {
        ticks_for(self.deadline)
    }
}

/// Shared engine used to compile component libraries.
#[derive(Clone)]
pub struct WasmEngine {
    pub(crate) engine: Engine,
    pub(crate) limits: Limits,
    _ticker: Arc<EpochTicker>,
    components: Arc<Mutex<HashMap<[u8; 32], Component>>>,
}

impl WasmEngine {
    /// Construct an engine with the given limits. When `cache_dir` is set, Wasmtime
    /// persists compiled code there; a relative path resolves against the current
    /// directory.
    pub fn new(limits: Limits, cache_dir: Option<&Path>) -> Result<Self> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.epoch_interruption(true);
        if let Some(cache_dir) = cache_dir {
            config.cache(Some(compilation_cache(cache_dir)?));
        }
        let engine = Engine::new(&config).map_err(Error::Engine)?;
        let ticker = EpochTicker::spawn(engine.clone()).map_err(Error::EpochTicker)?;
        Ok(Self {
            engine,
            limits,
            _ticker: Arc::new(ticker),
            components: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Compile a component and inspect its imports and exported functions.
    pub fn load(&self, wasm_path: &Path) -> Result<CompiledComponent> {
        let bytes = std::fs::read(wasm_path).map_err(|source| Error::Io {
            path: wasm_path.to_path_buf(),
            source,
        })?;
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        let component = {
            let mut components = self
                .components
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(component) = components.get(&digest) {
                component.clone()
            } else {
                let component =
                    Component::new(&self.engine, &bytes).map_err(|source| Error::Compile {
                        path: wasm_path.to_path_buf(),
                        source,
                    })?;
                components.insert(digest, component.clone());
                component
            }
        };
        Ok(CompiledComponent::new(self.clone(), component))
    }

    pub(crate) fn new_store(&self, permissions: Permissions) -> Result<Store<StoreData>> {
        let data = StoreData::new(permissions, self.limits.memory_bytes)?;
        let mut store = Store::new(&self.engine, data);
        store.limiter(|data| &mut data.limits);
        store.epoch_deadline_trap();
        store.set_epoch_deadline(self.limits.epoch_ticks());
        Ok(store)
    }
}

fn compilation_cache(cache_dir: &Path) -> Result<wasmtime::Cache> {
    let cache_dir: PathBuf = if cache_dir.is_absolute() {
        cache_dir.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(Error::CurrentDir)?
            .join(cache_dir)
    };
    let mut cache_config = wasmtime::CacheConfig::new();
    cache_config.with_directory(cache_dir);
    wasmtime::Cache::new(cache_config).map_err(Error::Engine)
}
