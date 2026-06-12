//! Public types for the WASM plugin host.

use std::time::Duration;

/// Default per-call epoch deadline.
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(60);
/// Default linear-memory cap (512 MiB).
pub const DEFAULT_MEMORY_LIMIT: usize = 512 * 1024 * 1024;

/// Severity level for host-routed log messages.
///
/// Mirrors the `log-level` enum in the WIT `host` interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    /// Verbose diagnostic output.
    Debug,
    /// Informational messages.
    Info,
    /// Warnings that do not abort the build.
    Warn,
    /// Errors.
    Error,
}

/// Resource limits applied to every [`crate::WasmInstance`].
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Maximum wall-clock time a single guest call may run before it is
    /// interrupted with [`crate::Error::Timeout`].
    pub deadline: Duration,
    /// Maximum linear-memory size (in bytes) any guest memory may grow to.
    pub memory_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            deadline: DEFAULT_DEADLINE,
            memory_bytes: DEFAULT_MEMORY_LIMIT,
        }
    }
}

/// A declared processor: its name, the globs it matches, and its priority
/// (lower runs first).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessorDef {
    /// Processor name (unique within the plugin).
    pub name: String,
    /// Glob patterns the processor matches.
    pub patterns: Vec<String>,
    /// Priority; lower values run first. Ties broken by plugin order.
    pub priority: i32,
}

/// Static description of a plugin, cached after a single throwaway
/// instantiation during [`crate::WasmEngine::load`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginInfo {
    /// Plugin identifier, matching `^[a-z0-9][a-z0-9_-]*$`.
    pub id: String,
    /// Plugin version (valid semver).
    pub version: String,
    /// The processors this plugin declares.
    pub processors: Vec<ProcessorDef>,
    /// Whether the plugin exports a generator.
    pub has_generator: bool,
}

/// Outcome of running a processor over a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessResult {
    /// The file was not changed.
    Unchanged,
    /// The file was changed (and possibly renamed).
    Modified {
        /// The (possibly new) output path.
        path: String,
        /// The new file contents.
        contents: Vec<u8>,
    },
    /// The file should be dropped from the output.
    Dropped,
}

/// Host functions a [`crate::WasmInstance`] may call back into.
///
/// Mirrors the WIT `host` interface. The generator-phase methods are only
/// invoked while [`crate::WasmInstance::generate`] is running; the host
/// suppresses calls made outside that window (see crate docs), so an
/// implementation does not need to guard against that itself.
pub trait HostCallbacks: Send {
    /// Emit a log message. Always invoked, in any phase.
    fn log(&mut self, level: LogLevel, message: &str);

    /// List output files matching an optional glob pattern.
    fn list_files(&mut self, pattern: Option<&str>) -> Vec<String>;

    /// Read a processed output file. `None` if it does not exist.
    fn read_file(&mut self, path: &str) -> Option<Vec<u8>>;

    /// Read a raw source file. `None` if it does not exist.
    fn read_source(&mut self, path: &str) -> Option<Vec<u8>>;

    /// Add or overwrite an output file.
    fn emit_file(&mut self, path: &str, contents: Vec<u8>);

    /// Remove an output file.
    fn remove_file(&mut self, path: &str);
}
