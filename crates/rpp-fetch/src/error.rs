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

    /// A `plugin.toml` or `rpp.json` manifest could not be parsed.
    #[error("failed to parse `{path}`: {reason}")]
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

    /// A package directory is missing its `rpp.json` manifest.
    #[error("`{0}` does not contain an rpp.json")]
    MissingPackageManifest(PathBuf),

    /// A registry request failed or returned malformed data.
    #[error("registry request to `{url}` failed: {reason}")]
    Registry {
        /// The URL that was requested.
        url: String,
        /// Failure detail.
        reason: String,
    },

    /// The registry has no plugin with this name.
    #[error("plugin `{0}` is not in the registry")]
    UnknownPackage(String),

    /// A dependency in `rpp.json` has an invalid name or spec.
    #[error("invalid dependency `{name}` = `{spec}`: {reason}")]
    InvalidDependency {
        /// Dependency name.
        name: String,
        /// The spec as written.
        spec: String,
        /// Why it is invalid.
        reason: String,
    },

    /// No published, non-yanked version satisfies the requested range.
    #[error("no published version of `{name}` matches `{requested}`")]
    NoMatchingVersion {
        /// Dependency name.
        name: String,
        /// The requested range.
        requested: String,
    },

    /// A matching version exists but doesn't support this rpp version.
    #[error("`{name}` {version} requires rpp {requires}, but this is rpp {rpp}; {hint}")]
    Incompatible {
        /// Dependency name.
        name: String,
        /// The newest matching (or pinned) version.
        version: semver::Version,
        /// The rpp range that version supports.
        requires: semver::VersionReq,
        /// The running rpp version.
        rpp: semver::Version,
        /// What the user can do about it.
        hint: String,
    },

    /// A downloaded archive did not match its recorded SHA-256.
    #[error("archive for `{name}` {version} has SHA-256 {actual}, expected {expected}")]
    HashMismatch {
        /// Package name.
        name: String,
        /// Package version.
        version: semver::Version,
        /// The recorded hash.
        expected: String,
        /// The hash of the downloaded bytes.
        actual: String,
    },

    /// A package's `rpp.json` doesn't declare the expected name or version.
    #[error("package at `{path}` is `{found}`, expected `{expected}`")]
    PackageMismatch {
        /// The package directory.
        path: PathBuf,
        /// Expected `name` (and version, for registry packages).
        expected: String,
        /// What the manifest declares.
        found: String,
    },

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
