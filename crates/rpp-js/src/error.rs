//! Error types for bundling and execution.

use thiserror::Error;

/// Result alias for the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors from bundling, loading or calling JavaScript.
#[derive(Debug, Error)]
pub enum Error {
    /// Bundling failed; the message lists every diagnostic with its source location.
    #[error("bundling failed:\n{0}")]
    Bundle(String),
    /// A JavaScript exception, from module evaluation or a call. `message` is the
    /// exception's message; `stack` is its stack with locations mapped to the original
    /// sources through the bundle's source map (e.g. `at transform (src/plugin.ts:12:5)`).
    #[error("{message}\n{stack}")]
    JavaScript {
        /// Exception message, e.g. `TypeError: x is not a function`.
        message: String,
        /// Source-mapped stack, one frame per line; empty when unavailable.
        stack: String,
    },
    /// The bundle has no exported function with this name.
    #[error("the plugin does not export a function named `{0}`")]
    MissingExport(String),
    /// Execution exceeded [`crate::Limits::time`]. The runtime is unusable afterwards.
    #[error("JavaScript execution timed out")]
    Deadline,
    /// Execution exceeded [`crate::Limits::heap_bytes`]. The runtime is unusable afterwards.
    #[error("JavaScript heap limit exceeded")]
    Heap,
    /// The [`crate::Cancellation`] fired. The runtime is unusable afterwards.
    #[error("JavaScript execution was cancelled")]
    Cancelled,
    /// A previous call ended in [`Error::Deadline`], [`Error::Heap`] or [`Error::Cancelled`].
    #[error("the JavaScript runtime was terminated by an earlier call")]
    Terminated,
    /// Invalid input or output: non-JSON results, limits out of range, oversized logs.
    #[error("{0}")]
    Invalid(String),
    /// The executor or watchdog thread could not be created.
    #[error("failed to start the JavaScript engine: {0}")]
    Io(#[from] std::io::Error),
}
