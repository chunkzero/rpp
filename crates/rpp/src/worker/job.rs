use std::path::PathBuf;
use std::sync::Arc;

use crate::build::{BuildError, FileEntry, ProcessedFile};
use crate::plugin::ProcessorPlugin;

/// A job submitted to the worker pool.
pub struct ProcessingJob {
    /// File to process
    pub file: FileEntry,
    /// Processors to apply (in priority order)
    pub processors: Vec<Arc<dyn ProcessorPlugin>>,
}

/// Result of processing a job.
pub enum ProcessingResult {
    /// File was successfully processed
    Processed(ProcessedFile),
    /// File was cancelled by a processor
    Cancelled { path: PathBuf },
    /// Processing failed
    Error { path: PathBuf, error: BuildError },
}
