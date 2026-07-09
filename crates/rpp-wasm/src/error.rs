//! Error types for the WASM component host.

use std::path::PathBuf;

use thiserror::Error;

/// Result alias for the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors produced while compiling, instantiating, or running a component.
#[derive(Debug, Error)]
pub enum Error {
    /// The engine could not be constructed.
    #[error("failed to construct WASM engine: {0}")]
    Engine(#[source] wasmtime::Error),
    /// The epoch ticker thread could not be started.
    #[error("failed to start WASM epoch ticker: {0}")]
    EpochTicker(#[source] std::io::Error),
    /// Reading the component file failed.
    #[error("failed to read component file {path}: {source}")]
    Io {
        /// Component path.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The component failed to compile.
    #[error("failed to compile component {path}: {source}")]
    Compile {
        /// Component path.
        path: PathBuf,
        /// Compiler error.
        #[source]
        source: wasmtime::Error,
    },
    /// Instantiating the component failed.
    #[error("failed to instantiate component: {0}")]
    Instantiate(#[source] wasmtime::Error),
    /// The component imports a capability that was not granted.
    #[error("component requires denied capability `{0}`")]
    DeniedCapability(String),
    /// An exported function could not be found.
    #[error("component export `{0}` was not found")]
    MissingExport(String),
    /// A dynamic value did not match its WIT type.
    #[error("component value error: {0}")]
    Value(String),
    /// A guest call trapped.
    #[error("guest trapped: {0}")]
    Trap(#[source] wasmtime::Error),
    /// A guest call exceeded its deadline.
    #[error("guest call timed out after {0:?}")]
    Timeout(std::time::Duration),
    /// Configuring a WASI preopen failed.
    #[error("failed to configure WASI directory {path}: {source}")]
    WasiDirectory {
        /// Host directory.
        path: PathBuf,
        /// WASI configuration error.
        #[source]
        source: wasmtime::Error,
    },
}
