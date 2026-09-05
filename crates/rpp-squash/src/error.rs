//! Error types for the squash crate.

use std::path::PathBuf;

use thiserror::Error;

/// Result alias used throughout this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors produced by squash operations.
///
/// Recoverable per-file problems (e.g. invalid JSON) are *not* errors: they are
/// collected as warnings in [`crate::SquashReport`] and the file is passed
/// through unchanged. Variants here represent failures that abort the operation.
#[derive(Debug, Error)]
pub enum Error {
    /// An I/O operation failed.
    #[error("io error at {path}: {source}")]
    Io {
        /// The path being operated on.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// A strip glob pattern was invalid.
    #[error("invalid strip glob pattern {pattern:?}: {source}")]
    InvalidGlob {
        /// The offending pattern.
        pattern: String,
        /// The underlying globset error.
        #[source]
        source: globset::Error,
    },

    /// Writing the zip archive failed.
    #[error("failed to write zip archive {path}: {source}")]
    Zip {
        /// The archive path.
        path: PathBuf,
        /// The underlying zip error.
        #[source]
        source: zip::result::ZipError,
    },

    /// The configured PackSquash binary could not be found.
    #[error("packsquash binary not found — install it or use engine = \"builtin\"")]
    PackSquashNotFound,

    /// The PackSquash binary exited with a non-zero status.
    #[error("packsquash failed (exit {code}): {stderr}")]
    PackSquashFailed {
        /// Reported exit code, or -1 if terminated by a signal.
        code: i32,
        /// Captured standard error output.
        stderr: String,
    },
}

impl Error {
    /// Build an [`Error::Io`] from a path and source error.
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}
