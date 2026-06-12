//! Error types for the WASM plugin host.

use std::path::PathBuf;

use thiserror::Error;

/// Result alias for the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors produced while compiling, instantiating, or running a WASM plugin.
#[derive(Debug, Error)]
pub enum Error {
    /// The engine could not be constructed (invalid wasmtime config).
    #[error("failed to construct WASM engine: {0}")]
    Engine(#[source] anyhow_compat::Error),

    /// The epoch ticker thread could not be started.
    #[error("failed to start WASM epoch ticker: {0}")]
    EpochTicker(#[source] std::io::Error),

    /// Reading the component file from disk failed.
    #[error("failed to read component file {path}: {source}")]
    Io {
        /// The path that could not be read.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The component failed to compile.
    #[error("failed to compile component {path}: {source}")]
    Compile {
        /// The component path.
        path: PathBuf,
        /// The underlying wasmtime error.
        #[source]
        source: anyhow_compat::Error,
    },

    /// Instantiating the component (linking + running `_start`) failed.
    #[error("failed to instantiate component: {0}")]
    Instantiate(#[source] anyhow_compat::Error),

    /// The plugin metadata returned by `get-info` was invalid.
    #[error("invalid plugin metadata: {0}")]
    InvalidInfo(String),

    /// A guest call trapped (panic, unreachable, oom, etc.).
    #[error("guest trapped: {0}")]
    Trap(#[source] anyhow_compat::Error),

    /// A guest call exceeded its epoch deadline.
    #[error("guest call timed out after {0:?}")]
    Timeout(std::time::Duration),

    /// The guest returned an `Err(string)` from a fallible export.
    #[error("guest error: {0}")]
    GuestError(String),

    /// The requested processor name is not declared by the plugin.
    #[error("unknown processor: {0}")]
    UnknownProcessor(String),
}

/// Re-export of `anyhow::Error` under a stable name. wasmtime returns
/// `anyhow::Error` from fallible APIs; we keep it boxed in our error type
/// without taking a direct `anyhow` dependency in our public surface.
pub(crate) mod anyhow_compat {
    pub use wasmtime::Error;
}
