//! Crate-level error type.

use std::path::PathBuf;

use thiserror::Error;

/// Convenience alias for results produced by this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// The unified error type for the `rpp` core library.
#[derive(Debug, Error)]
pub enum Error {
    /// Failed to read or write a file on disk.
    #[error("io error at {path}: {source}")]
    Io {
        /// The path being operated on.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// An I/O error without an associated path.
    #[error("io error: {0}")]
    IoBare(#[source] std::io::Error),

    /// Failed to convert or validate `rpp.config.ts`.
    #[error("invalid config {path}: {message}")]
    Config {
        /// The config file path.
        path: PathBuf,
        /// A human-readable description of the problem.
        message: String,
    },

    /// Failed to parse or validate an `rpp.json` manifest.
    #[error("invalid plugin manifest {path}: {message}")]
    Manifest {
        /// The manifest path.
        path: PathBuf,
        /// A human-readable description of the problem.
        message: String,
    },

    /// A plugin failed while loading (parse/sandbox setup/validation).
    #[error("plugin `{plugin}` failed to load: {message}")]
    PluginLoad {
        /// The plugin id (or path if the id is unknown).
        plugin: String,
        /// Details, including any language-level traceback.
        message: String,
    },

    /// A processor raised an error while running over a file.
    #[error("plugin `{plugin}` processor `{processor}` failed on `{file}`: {message}")]
    Processor {
        /// The owning plugin id.
        plugin: String,
        /// The processor name.
        processor: String,
        /// The file path being processed.
        file: String,
        /// Details, including any language-level traceback.
        message: String,
    },

    /// A generator raised an error while running.
    #[error("plugin `{plugin}` generator failed: {message}")]
    Generator {
        /// The owning plugin id.
        plugin: String,
        /// Details, including any language-level traceback.
        message: String,
    },

    /// A lifecycle hook (`on_start`/`on_finish`) raised an error.
    #[error("plugin `{plugin}` hook `{hook}` failed: {message}")]
    Hook {
        /// The owning plugin id.
        plugin: String,
        /// The hook name.
        hook: String,
        /// Details, including any language-level traceback.
        message: String,
    },

    /// The build engine itself failed (cache, worker pool, discovery).
    #[error("build error: {0}")]
    Build(String),
}

impl Error {
    /// Construct an [`Error::Io`] with a path for context.
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}
