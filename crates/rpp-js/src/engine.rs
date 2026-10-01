//! Per-thread engine and isolate-backed runtimes.

use std::marker::PhantomData;

use crate::bundle::Bundle;
use crate::error::Result;
use crate::model::{Call, Cancellation, Clock, Host, Limits, Log, Output};

/// The per-thread executor and deadline watchdog shared by its runtimes.
///
/// `!Send`: create it on the thread that will own its runtimes.
pub struct Engine {
    _thread: PhantomData<*const ()>,
}

impl Engine {
    /// Initialize V8. Call once on the main thread before creating engines on other
    /// threads; repeated calls are harmless and [`Engine::new`] calls it too.
    pub fn init_platform() {
        todo!()
    }

    /// Create an engine for the current thread.
    ///
    /// # Errors
    ///
    /// [`crate::Error::Io`] when the executor or watchdog cannot start.
    pub fn new() -> Result<Self> {
        todo!()
    }

    /// Create an isolate, install the globals, and evaluate `bundle` as a module
    /// (awaiting top-level `await`) under `limits`, `clock` and `cancellation`.
    /// `name` labels the module in stacks. Returns the runtime and the `console`
    /// output of evaluation.
    ///
    /// # Errors
    ///
    /// [`crate::Error::JavaScript`] when evaluation throws; [`crate::Error::Deadline`],
    /// [`crate::Error::Heap`] or [`crate::Error::Cancelled`] when it is terminated;
    /// [`crate::Error::Invalid`] for limits below the minimum heap.
    pub fn load(
        &self,
        name: &str,
        bundle: &Bundle,
        limits: Limits,
        clock: Clock,
        cancellation: &Cancellation,
    ) -> Result<(Runtime, Vec<Log>)> {
        let _ = (name, bundle, limits, clock, cancellation);
        todo!()
    }
}

/// One isolate with one evaluated bundle. Module state persists between calls.
///
/// `!Send`: use and drop it on the thread of the [`Engine`] that loaded it.
pub struct Runtime {
    _thread: PhantomData<*const ()>,
}

impl Runtime {
    /// Call `call.export(call.args, call.bytes)` with `host` available, run the event
    /// loop until the returned value (or promise) settles, and convert it to [`Output`].
    /// `engine` must be the engine that loaded this runtime.
    ///
    /// # Errors
    ///
    /// [`crate::Error::MissingExport`]; [`crate::Error::JavaScript`] for exceptions,
    /// rejections, and promises that can never settle; [`crate::Error::Invalid`] for
    /// results that are not JSON or a `Uint8Array`; [`crate::Error::Deadline`],
    /// [`crate::Error::Heap`] or [`crate::Error::Cancelled`] when terminated, after
    /// which every call returns [`crate::Error::Terminated`].
    pub fn call(
        &mut self,
        engine: &Engine,
        call: Call<'_>,
        host: &mut dyn Host,
        cancellation: &Cancellation,
    ) -> Result<Output> {
        let _ = (engine, call, host, cancellation);
        todo!()
    }

    /// Whether a call was terminated, making this runtime unusable.
    pub fn is_terminated(&self) -> bool {
        todo!()
    }
}
