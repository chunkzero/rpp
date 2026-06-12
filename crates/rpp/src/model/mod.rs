//! Core, runtime-agnostic plugin model (spec §3).
//!
//! The build pipeline is defined entirely in terms of these traits. Lua and
//! WASM runtimes are implementations of [`PluginFactory`] / [`PluginInstance`];
//! the build engine implements [`GeneratorHost`].

use crate::error::Result;

/// A file flowing through the pipeline.
///
/// Paths are relative and use forward slashes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackFile {
    /// The relative, forward-slash output path.
    pub path: String,
    /// The file contents.
    pub contents: Vec<u8>,
}

impl PackFile {
    /// Construct a [`PackFile`].
    pub fn new(path: impl Into<String>, contents: Vec<u8>) -> Self {
        Self {
            path: path.into(),
            contents,
        }
    }
}

/// The outcome of running a single processor over a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessOutcome {
    /// The file was not modified.
    Unchanged,
    /// The file's path and/or contents were modified.
    Modified,
    /// The file was dropped; it is excluded from output and the chain stops.
    Dropped,
}

/// A processor declaration: its name, the globs it matches, and its priority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessorDef {
    /// Processor name (unique within a plugin).
    pub name: String,
    /// Glob patterns selecting which files this processor runs on.
    pub patterns: Vec<String>,
    /// Priority; lower runs first. Defaults to `0`.
    pub priority: i32,
}

/// Aggregate build statistics handed to `on_build_finish`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuildStats {
    /// Number of source files processed this build (not served from cache).
    pub processed: usize,
    /// Number of source files served from the cache.
    pub cached: usize,
    /// Number of generator output files produced.
    pub generated: usize,
    /// Number of files dropped by processors.
    pub dropped: usize,
}

/// A static, shareable description of a configured plugin plus a way to
/// instantiate per-worker live instances. One per `[[plugin]]` entry.
pub trait PluginFactory: Send + Sync {
    /// The plugin id.
    fn id(&self) -> &str;
    /// The plugin version string.
    fn version(&self) -> &str;
    /// A hash covering plugin code AND its options; feeds cache invalidation.
    fn cache_key(&self) -> u64;
    /// The processors this plugin declares.
    fn processors(&self) -> &[ProcessorDef];
    /// Whether this plugin has a generator phase.
    fn has_generator(&self) -> bool;
    /// Instantiate a live instance for one worker thread.
    fn instantiate(&self) -> Result<Box<dyn PluginInstance>>;
}

/// A live plugin instance bound to a single thread.
pub trait PluginInstance: Send {
    /// Run one named processor over a file, mutating it in place.
    fn process(&mut self, processor: &str, file: &mut PackFile) -> Result<ProcessOutcome>;
    /// Run the generator phase (sequential, after all processing).
    fn generate(&mut self, host: &mut dyn GeneratorHost) -> Result<()>;
    /// Lifecycle hook fired before processing begins.
    fn on_build_start(&mut self) -> Result<()>;
    /// Lifecycle hook fired after the build completes.
    fn on_build_finish(&mut self, stats: &BuildStats) -> Result<()>;
}

/// The kind of read a generator performed, used to record its dependency set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadKind {
    /// A `list_files(glob)` query.
    List,
    /// A `read_file(path)` of a processed output.
    File,
    /// A `read_source(path)` of a raw source file.
    Source,
}

/// What generators may do; implemented by the build engine.
///
/// Every read is recorded so the generator can be incrementally invalidated when
/// any of its inputs change.
pub trait GeneratorHost {
    /// List processed output files matching an optional glob. Recorded as a dep.
    fn list_files(&mut self, glob: Option<&str>) -> Vec<String>;
    /// Read a processed output file. Recorded as a dep.
    fn read_file(&mut self, path: &str) -> Option<Vec<u8>>;
    /// Read a raw source file. Recorded as a dep.
    fn read_source(&mut self, path: &str) -> Option<Vec<u8>>;
    /// Add or overwrite an output file.
    fn emit(&mut self, path: &str, contents: Vec<u8>);
    /// Drop an output file.
    fn remove(&mut self, path: &str);
}
