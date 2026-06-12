//! WASIp2 component plugin host for rpp, built on wasmtime.
//!
//! This crate is a **standalone** host for `rpp:plugin@0.1.0` WASIp2 component
//! plugins. It does not depend on the `rpp` core crate; the core wraps this
//! host behind its `wasm` feature with an adapter implementing its plugin
//! traits (see `docs/SPEC.md` sections 2–3 and 5).
//!
//! # Overview
//!
//! * [`WasmEngine`] owns a shared, thread-safe wasmtime [`Engine`](wasmtime::Engine) configured
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
mod convert;
mod engine;
mod error;
mod instance;
mod store;
mod types;

pub use engine::{CompiledPlugin, WasmEngine};
pub use error::{Error, Result};
pub use instance::WasmInstance;
pub use types::{
    HostCallbacks, Limits, LogLevel, PluginInfo, ProcessResult, ProcessorDef, DEFAULT_DEADLINE,
    DEFAULT_MEMORY_LIMIT,
};

pub(crate) use store::StoreData;
