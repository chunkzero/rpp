//! Build result types reported to consumers (dev server, squash).

use std::time::Duration;

/// What changed on disk during a build.
///
/// Consumers (the dev server, incremental squash) use this to do minimal work.
#[derive(Debug, Clone, Default)]
pub struct ChangeReport {
    /// Output-relative paths written or overwritten this build.
    pub written: Vec<String>,
    /// Output-relative paths removed because they became stale.
    pub removed: Vec<String>,
}

/// The outcome of a build.
#[derive(Debug, Clone)]
pub struct BuildResult {
    /// Source files processed this build (not served from cache).
    pub processed: usize,
    /// Source files served from the cache.
    pub cached: usize,
    /// Generator output files produced.
    pub generated: usize,
    /// Files dropped by processors.
    pub dropped: usize,
    /// Total wall-clock build duration.
    pub duration: Duration,
    /// The set of output paths written/removed.
    pub changes: ChangeReport,
}
