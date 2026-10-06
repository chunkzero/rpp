//! Shared wasmtime engine.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};
use twox_hash::XxHash3_128;
use wasmtime::component::{Component, Val};
use wasmtime::{Config, Engine, Store};

use crate::store::StoreData;
use crate::types::{Limits, Permissions};
use crate::{CompiledComponent, Error, Result};

const EPOCH_TICK: Duration = Duration::from_millis(50);

/// Artifacts not used for this long are removed when a new artifact is written.
const ARTIFACT_MAX_IDLE: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// A loaded artifact's modification time is refreshed once it is this old.
const ARTIFACT_TOUCH_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
const ARTIFACT_EXTENSION: &str = "cwasm";
/// Length of the little-endian XXH3-128 checksum of the serialized component that ends each
/// artifact.
const CHECKSUM_LEN: usize = 16;

/// Advances the engine epoch every `EPOCH_TICK` until dropped. Dropping wakes the
/// thread immediately instead of waiting out the current tick.
struct EpochTicker {
    stop: Option<Sender<()>>,
    handle: Option<JoinHandle<()>>,
}

impl EpochTicker {
    fn spawn(engine: Engine) -> std::io::Result<Self> {
        let (stop, stopped) = mpsc::channel::<()>();
        let handle = std::thread::Builder::new()
            .name("rpp-wasm-epoch".into())
            .spawn(move || {
                while let Err(RecvTimeoutError::Timeout) = stopped.recv_timeout(EPOCH_TICK) {
                    engine.increment_epoch();
                }
            })?;
        Ok(Self {
            stop: Some(stop),
            handle: Some(handle),
        })
    }
}

impl Drop for EpochTicker {
    fn drop(&mut self) {
        self.stop.take();
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
    artifacts: Option<Arc<Artifacts>>,
}

/// How [`WasmEngine::component`] produced a component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    Memory,
    Artifact,
    Compiled,
}

impl WasmEngine {
    /// Construct an engine with the given limits.
    ///
    /// When `cache_dir` is set, compiled components are stored there as uncompressed
    /// `<key>.cwasm` artifacts keyed by the component's SHA-256 and this engine's
    /// compilation settings, and later loads map them instead of compiling. Each artifact
    /// ends with a checksum of its contents, and one that does not match is recompiled and
    /// replaced. A relative path resolves against the current directory. The directory is
    /// created if missing; failing to create it is an error. Artifacts unused for 30 days are
    /// removed when a new one is written.
    ///
    /// The checksum detects damage, not tampering: artifacts are native code, so `cache_dir`
    /// must only be writable by the user running rpp, like Wasmtime's own cache directory.
    pub fn new(limits: Limits, cache_dir: Option<&Path>) -> Result<Self> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.epoch_interruption(true);
        let engine = Engine::new(&config).map_err(Error::Engine)?;
        let artifacts = cache_dir
            .map(|dir| Artifacts::new(dir, &engine).map(Arc::new))
            .transpose()?;
        let ticker = EpochTicker::spawn(engine.clone()).map_err(Error::EpochTicker)?;
        Ok(Self {
            engine,
            limits,
            _ticker: Arc::new(ticker),
            components: Arc::new(Mutex::new(HashMap::new())),
            artifacts,
        })
    }

    /// Compile a component and inspect its imports and exported functions.
    pub fn load(&self, wasm_path: &Path) -> Result<CompiledComponent> {
        let (component, digest, _) = self.component(wasm_path)?;
        Ok(CompiledComponent::new(self.clone(), component, digest))
    }

    fn component(&self, wasm_path: &Path) -> Result<(Component, [u8; 32], Origin)> {
        let bytes = std::fs::read(wasm_path).map_err(|source| Error::Io {
            path: wasm_path.to_path_buf(),
            source,
        })?;
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        let mut components = self
            .components
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(component) = components.get(&digest) {
            return Ok((component.clone(), digest, Origin::Memory));
        }
        let artifact = self
            .artifacts
            .as_ref()
            .map(|artifacts| artifacts.path(&digest));
        let cached = artifact
            .as_ref()
            .and_then(|path| Artifacts::load(&self.engine, path));
        let (component, origin) = match cached {
            Some(component) => (component, Origin::Artifact),
            None => {
                let component =
                    Component::new(&self.engine, &bytes).map_err(|source| Error::Compile {
                        path: wasm_path.to_path_buf(),
                        source,
                    })?;
                if let (Some(artifacts), Some(path)) = (&self.artifacts, &artifact) {
                    artifacts.store(path, &component);
                }
                (component, Origin::Compiled)
            }
        };
        components.insert(digest, component.clone());
        Ok((component, digest, origin))
    }

    pub(crate) fn new_store(&self, permissions: Permissions) -> Result<Store<StoreData>> {
        let data = StoreData::new(permissions, self.limits.memory_bytes)?;
        let mut store = Store::new(&self.engine, data);
        store.limiter(|data| &mut data.limits);
        store.epoch_deadline_trap();
        store.set_epoch_deadline(self.limits.epoch_ticks());
        // Hostcall fuel counts the host bytes allocated when lifting one call's results, and a
        // lifted `list<u8>` element costs a whole `Val`. Scaling by the memory limit lets any
        // output that fits in guest memory lift, while aliased lists still hit a finite cap.
        store.set_hostcall_fuel(self.limits.memory_bytes.saturating_mul(size_of::<Val>()));
        Ok(store)
    }
}

/// The precompiled artifact directory of one engine configuration.
struct Artifacts {
    dir: PathBuf,
    /// Wasmtime's compilation settings and this crate's version. Wasmtime also rejects
    /// artifacts from another Wasmtime version when deserializing.
    engine_key: [u8; 32],
}

impl Artifacts {
    fn new(dir: &Path, engine: &Engine) -> Result<Self> {
        let dir = if dir.is_absolute() {
            dir.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(Error::CurrentDir)?
                .join(dir)
        };
        std::fs::create_dir_all(&dir).map_err(|source| Error::CacheDir {
            path: dir.clone(),
            source,
        })?;
        let mut hasher = Sha256Hasher(Sha256::new());
        engine.precompile_compatibility_hash().hash(&mut hasher);
        env!("CARGO_PKG_VERSION").hash(&mut hasher);
        Ok(Self {
            dir,
            engine_key: hasher.0.finalize().into(),
        })
    }

    fn path(&self, digest: &[u8; 32]) -> PathBuf {
        let key = Sha256::new()
            .chain_update(self.engine_key)
            .chain_update(digest)
            .finalize();
        let name: String = key.iter().map(|byte| format!("{byte:02x}")).collect();
        self.dir.join(name).with_extension(ARTIFACT_EXTENSION)
    }

    /// Map the artifact at `path` when its checksum matches, refreshing its modification time
    /// first.
    fn load(engine: &Engine, path: &Path) -> Option<Component> {
        let bytes = std::fs::read(path).ok()?;
        let (serialized, checksum) = bytes.split_at(bytes.len().checked_sub(CHECKSUM_LEN)?);
        if XxHash3_128::oneshot(serialized).to_le_bytes() != checksum {
            return None;
        }
        Self::touch(path);
        // SAFETY: the file was verified above. Artifacts are written by `Artifacts::store`
        // through exclusively created temporary files that are fully written and flushed before
        // they are renamed into place, so a file replacing it between the check and the mapping
        // is also complete, and no artifact is modified in place; see `WasmEngine::new` for the
        // trust placed in the cache directory. Wasmtime ignores the trailing checksum.
        unsafe { Component::deserialize_file(engine, path) }.ok()
    }

    /// Best-effort atomic write of `component` with its checksum to `path`, then removal of
    /// idle artifacts.
    fn store(&self, path: &Path, component: &Component) {
        let Ok(mut bytes) = component.serialize() else {
            return;
        };
        bytes.extend_from_slice(&XxHash3_128::oneshot(&bytes).to_le_bytes());
        let written = tempfile::Builder::new()
            .suffix(".tmp")
            .tempfile_in(&self.dir)
            .and_then(|mut temp| {
                temp.write_all(&bytes)?;
                temp.as_file().sync_all()?;
                temp.persist(path).map_err(|error| error.error)
            });
        if written.is_ok() {
            self.prune(path);
        }
    }

    /// Remove artifacts and leftover temporary files idle for [`ARTIFACT_MAX_IDLE`].
    fn prune(&self, keep: &Path) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let ours = path
                .extension()
                .is_some_and(|ext| ext == ARTIFACT_EXTENSION || ext == "tmp");
            if !ours || path == keep {
                continue;
            }
            let idle = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|modified| SystemTime::now().duration_since(modified).ok());
            if idle.is_some_and(|idle| idle > ARTIFACT_MAX_IDLE) {
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    /// Refresh `path`'s modification time when it is older than [`ARTIFACT_TOUCH_AFTER`],
    /// so artifacts in use are not pruned.
    fn touch(path: &Path) {
        let mut options = std::fs::File::options();
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;

            /// Attribute access is not subject to the share mode of Wasmtime's mapping handles.
            const FILE_WRITE_ATTRIBUTES: u32 = 0x100;
            options.access_mode(FILE_WRITE_ATTRIBUTES);
        }
        // Setting explicit times requires owning the file, not write access.
        #[cfg(not(windows))]
        options.read(true);
        let Ok(file) = options.open(path) else {
            return;
        };
        let stale = file
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age > ARTIFACT_TOUCH_AFTER);
        if stale {
            let _ = file.set_modified(SystemTime::now());
        }
    }
}

/// Feeds [`Hash`] output into SHA-256 so engine keys are stable across processes.
struct Sha256Hasher(Sha256);

impl Hasher for Sha256Hasher {
    fn write(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    fn finish(&self) -> u64 {
        unreachable!("only the SHA-256 state is read")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine(cache: &Path) -> WasmEngine {
        WasmEngine::new(Limits::default(), Some(cache)).unwrap()
    }

    fn artifacts(cache: &Path) -> Vec<PathBuf> {
        let mut paths: Vec<_> = std::fs::read_dir(cache)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        paths.sort();
        paths
    }

    #[test]
    fn artifacts_are_reused_across_engines() {
        let dir = tempfile::tempdir().unwrap();
        let (wasm, cache) = (dir.path().join("c.wat"), dir.path().join("cache"));
        std::fs::write(&wasm, "(component)").unwrap();

        let first = engine(&cache);
        assert_eq!(first.component(&wasm).unwrap().2, Origin::Compiled);
        assert_eq!(first.component(&wasm).unwrap().2, Origin::Memory);
        let stored = artifacts(&cache);
        assert_eq!(stored.len(), 1);

        assert_eq!(engine(&cache).component(&wasm).unwrap().2, Origin::Artifact);

        std::fs::write(&wasm, "(component (core module))").unwrap();
        assert_eq!(engine(&cache).component(&wasm).unwrap().2, Origin::Compiled);
        assert_eq!(artifacts(&cache).len(), 2);
    }

    #[test]
    fn corrupt_artifacts_are_recompiled() {
        let dir = tempfile::tempdir().unwrap();
        let (wasm, cache) = (dir.path().join("c.wat"), dir.path().join("cache"));
        std::fs::write(&wasm, "(component (core module (func (export \"f\"))))").unwrap();
        engine(&cache).component(&wasm).unwrap();
        let [artifact] = artifacts(&cache).try_into().unwrap();
        let mut bytes = std::fs::read(&artifact).unwrap();
        let middle = bytes.len() / 2;
        bytes[middle] ^= 0xff;
        std::fs::write(&artifact, &bytes).unwrap();

        assert_eq!(engine(&cache).component(&wasm).unwrap().2, Origin::Compiled);
        assert_eq!(engine(&cache).component(&wasm).unwrap().2, Origin::Artifact);
        assert_eq!(artifacts(&cache), [artifact]);
    }
}
