//! Worker pool for parallel file processing.

mod job;
mod pool;

pub use job::{ProcessingJob, ProcessingResult};
pub use pool::WorkerPool;
