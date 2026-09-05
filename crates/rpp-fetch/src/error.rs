//! Crate-wide error type.

use std::path::PathBuf;

use thiserror::Error;

/// Result type alias used throughout this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors produced by plugin source resolution, caching, lockfile, and discovery.
#[derive(Debug, Error)]
pub enum Error {
    /// A plugin source string could not be parsed.
    #[error("invalid plugin source `{source_str}`: {reason}")]
    InvalidSource {
        /// The offending source string.
        source_str: String,
        /// A human-readable explanation of why parsing failed.
        reason: String,
    },

    /// A path source pointed at a directory that does not exist.
    #[error("plugin path `{0}` does not exist or is not a directory")]
    PathNotFound(PathBuf),

    /// A resolved plugin directory is missing its `plugin.toml` manifest.
    #[error("plugin directory `{0}` does not contain a plugin.toml")]
    MissingManifest(PathBuf),

    /// A `plugin.toml` manifest could not be parsed.
    #[error("failed to parse plugin.toml at `{path}`: {reason}")]
    InvalidManifest {
        /// Path to the manifest that failed to parse.
        path: PathBuf,
        /// Parse failure detail.
        reason: String,
    },

    /// The lockfile declared a `version` newer than this crate understands.
    #[error("rpp.lock version {found} is newer than supported version {supported}")]
    UnsupportedLockVersion {
        /// The version found in the lockfile.
        found: u32,
        /// The highest version this crate can read.
        supported: u32,
    },

    /// A lockfile could not be parsed as TOML.
    #[error("failed to parse lockfile `{path}`: {reason}")]
    InvalidLockfile {
        /// Path to the lockfile.
        path: PathBuf,
        /// Parse failure detail.
        reason: String,
    },

    /// A GitHub API request failed or returned an unexpected status.
    #[error("github request to `{url}` failed: {reason}")]
    GitHub {
        /// The URL that was requested.
        url: String,
        /// Failure detail (status code or transport error).
        reason: String,
    },

    /// A tarball entry attempted to escape the extraction root (path traversal).
    #[error("refusing to extract tar entry `{0}`: path escapes archive root")]
    UnsafeTarEntry(String),

    /// An I/O error occurred.
    #[error("{context}: {source}")]
    Io {
        /// What was being attempted when the error occurred.
        context: String,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
}

impl Error {
    /// Wrap an [`std::io::Error`] with a contextual message.
    pub(crate) fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Error::Io {
            context: context.into(),
            source,
        }
    }
}
