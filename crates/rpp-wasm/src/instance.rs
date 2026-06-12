//! Live WASM plugin instances.

use std::time::Duration;

use wasmtime::Store;

use crate::bindings::RppPlugin;
use crate::convert::convert_process_result;
use crate::error::{Error, Result};
use crate::store::StoreData;
use crate::types::{HostCallbacks, LogLevel, ProcessResult};

/// A live, isolated plugin instance bound to one set of host callbacks.
///
/// Not `Sync`: drive a single instance from one thread at a time. Create one
/// instance per worker thread via [`crate::CompiledPlugin::instantiate`].
pub struct WasmInstance {
    pub(crate) store: Store<StoreData>,
    pub(crate) instance: RppPlugin,
    pub(crate) epoch_ticks: u64,
    pub(crate) deadline: Duration,
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
        let file = crate::bindings::guest::FileData {
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
pub(crate) struct NoopCallbacks;

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
pub(crate) fn map_timeout(err: wasmtime::Error, deadline: Duration) -> Error {
    if let Some(trap) = err.downcast_ref::<wasmtime::Trap>() {
        if *trap == wasmtime::Trap::Interrupt {
            return Error::Timeout(deadline);
        }
    }
    Error::Trap(err)
}
