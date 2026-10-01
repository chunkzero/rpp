//! TypeScript bundling (Rolldown) and sandboxed execution (V8 through `deno_core`)
//! for rpp plugins.
//!
//! [`bundle`] turns an entry module into one ESM file with a source map. An
//! [`Engine`] owns the per-thread executor and watchdog; each [`Runtime`] is one
//! isolate holding one evaluated bundle, and [`Runtime::call`] invokes its exports.
//! Runtimes are `!Send`: create, call and drop them on their engine's thread.

#![deny(missing_docs)]

mod bundle;
mod engine;
mod error;
mod model;
mod sourcemap;

pub use bundle::{bundle, Bundle, BundleRequest};
pub use engine::{Engine, Runtime};
pub use error::{Error, Result};
pub use model::{Call, Cancellation, Clock, Host, HostReply, Limits, Log, LogLevel, Output};
