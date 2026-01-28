# Commit 5: Worker Pool and Process Phase

**Goal**: Parallel file processing with processor chaining.

## Files to Create

```
crates/rpp/src/
├── worker/
│   ├── mod.rs
│   ├── pool.rs
│   └── job.rs
├── build/
│   └── process.rs
```

## `worker/mod.rs`

```rust
//! Worker pool for parallel file processing.

mod job;
mod pool;

pub use job::{ProcessingJob, ProcessingResult};
pub use pool::WorkerPool;
```

## `worker/job.rs`

```rust
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
```

## `worker/pool.rs`

```rust
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use crossbeam_channel::{Receiver, Sender, unbounded};
use twox_hash::XxHash3_64;

use crate::build::{BuildError, ProcessedFile, Transformation};
use crate::plugin::{ProcessResult, ProcessingContext};

use super::job::{ProcessingJob, ProcessingResult};

/// Pool of worker threads for parallel file processing.
pub struct WorkerPool {
    workers: Vec<JoinHandle<()>>,
    job_tx: Sender<Option<ProcessingJob>>,
    result_rx: Receiver<ProcessingResult>,
    num_workers: usize,
}

impl WorkerPool {
    /// Create a new worker pool with the specified number of workers.
    pub fn new(num_workers: usize) -> Self {
        let (job_tx, job_rx) = unbounded::<Option<ProcessingJob>>();
        let (result_tx, result_rx) = unbounded::<ProcessingResult>();

        let job_rx = Arc::new(job_rx);

        let workers: Vec<_> = (0..num_workers)
            .map(|_| {
                let job_rx = Arc::clone(&job_rx);
                let result_tx = result_tx.clone();

                thread::spawn(move || {
                    Self::worker_loop(job_rx, result_tx);
                })
            })
            .collect();

        Self {
            workers,
            job_tx,
            result_rx,
            num_workers,
        }
    }

    fn worker_loop(
        job_rx: Arc<Receiver<Option<ProcessingJob>>>,
        result_tx: Sender<ProcessingResult>,
    ) {
        // Each worker can initialize its own LuaRuntime here if needed

        while let Ok(Some(job)) = job_rx.recv() {
            let result = Self::process_job(job);
            let _ = result_tx.send(result);
        }
    }

    fn process_job(job: ProcessingJob) -> ProcessingResult {
        let mut content = job.file.content;
        let mut output_path = job.file.relative_path.clone();
        let mut transformations = Vec::new();

        let config = toml::Value::Table(toml::map::Map::new());

        for processor in &job.processors {
            let input_hash = XxHash3_64::oneshot(&content);

            let ctx = ProcessingContext {
                path: &job.file.relative_path,
                content: &content,
                source_path: &job.file.source_path,
                config: &config,
            };

            match processor.process(&ctx) {
                Ok(ProcessResult::Continue { content: new_content, output_path: new_path }) => {
                    let output_hash = XxHash3_64::oneshot(&new_content);

                    transformations.push(Transformation {
                        processor: processor.name().to_string(),
                        version: processor.version().to_string(),
                        input_hash,
                        output_hash,
                    });

                    content = new_content;
                    if let Some(p) = new_path {
                        output_path = p;
                    }
                }
                Ok(ProcessResult::Skip) => {
                    // Skip this processor, continue with next
                    continue;
                }
                Ok(ProcessResult::Cancel) => {
                    return ProcessingResult::Cancelled {
                        path: job.file.source_path,
                    };
                }
                Err(e) => {
                    return ProcessingResult::Error {
                        path: job.file.source_path,
                        error: e,
                    };
                }
            }
        }

        ProcessingResult::Processed(ProcessedFile {
            source_path: job.file.source_path,
            output_path,
            content,
            transformations,
        })
    }

    /// Submit a job to the pool.
    pub fn submit(&self, job: ProcessingJob) -> Result<(), BuildError> {
        self.job_tx
            .send(Some(job))
            .map_err(|_| BuildError::Worker("Failed to submit job".into()))
    }

    /// Get an iterator over results.
    pub fn results(&self) -> impl Iterator<Item = ProcessingResult> + '_ {
        self.result_rx.iter()
    }

    /// Try to receive a result without blocking.
    pub fn try_recv(&self) -> Option<ProcessingResult> {
        self.result_rx.try_recv().ok()
    }

    /// Signal workers to shut down and wait for them.
    pub fn shutdown(self) {
        for _ in 0..self.num_workers {
            let _ = self.job_tx.send(None);
        }
        for worker in self.workers {
            let _ = worker.join();
        }
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        for _ in 0..self.num_workers {
            let _ = self.job_tx.send(None);
        }
    }
}
```

## `build/process.rs`

```rust
use std::sync::Arc;

use crate::build::{BuildError, CachedEntry, FileIndex, ProcessedFile};
use crate::plugin::PluginRegistry;
use crate::worker::{ProcessingJob, ProcessingResult, WorkerPool};

/// Process phase: runs processors on all files in parallel.
pub struct ProcessPhase {
    num_workers: usize,
}

impl ProcessPhase {
    pub fn new(num_workers: usize) -> Self {
        Self { num_workers }
    }

    /// Process all files in the index using the worker pool.
    pub fn run(
        &self,
        index: FileIndex,
        registry: &Arc<PluginRegistry>,
    ) -> Result<ProcessPhaseResult, BuildError> {
        let pool = WorkerPool::new(self.num_workers);

        let mut submitted = 0;

        // Submit all jobs
        for file in index.entries {
            let processors = registry.processors_for_file(&file.relative_path);
            let job = ProcessingJob { file, processors };
            pool.submit(job)?;
            submitted += 1;
        }

        // Collect results
        let mut processed = Vec::new();
        let mut cancelled = Vec::new();
        let mut errors = Vec::new();

        let mut received = 0;
        for result in pool.results() {
            received += 1;

            match result {
                ProcessingResult::Processed(file) => processed.push(file),
                ProcessingResult::Cancelled { path } => cancelled.push(path),
                ProcessingResult::Error { path, error } => {
                    errors.push((path, error));
                }
            }

            if received >= submitted {
                break;
            }
        }

        pool.shutdown();

        // Report first error
        if !errors.is_empty() {
            let (_, error) = errors.into_iter().next().unwrap();
            return Err(error);
        }

        Ok(ProcessPhaseResult {
            processed,
            cancelled_count: cancelled.len(),
            cached: index.cached,
        })
    }
}

/// Result of the process phase.
pub struct ProcessPhaseResult {
    pub processed: Vec<ProcessedFile>,
    pub cancelled_count: usize,
    pub cached: Vec<CachedEntry>,
}
```

## Processing Flow

```
                    ┌─────────────┐
                    │  FileIndex  │
                    │  .entries   │
                    └──────┬──────┘
                           │
         ┌─────────────────┼─────────────────┐
         │                 │                 │
         ▼                 ▼                 ▼
    ┌─────────┐       ┌─────────┐       ┌─────────┐
    │ Worker1 │       │ Worker2 │       │ WorkerN │
    │         │       │         │       │         │
    │ ┌─────┐ │       │ ┌─────┐ │       │ ┌─────┐ │
    │ │ Job │ │       │ │ Job │ │       │ │ Job │ │
    │ └──┬──┘ │       │ └──┬──┘ │       │ └──┬──┘ │
    │    │    │       │    │    │       │    │    │
    │    ▼    │       │    ▼    │       │    ▼    │
    │ Proc A  │       │ Proc A  │       │ Proc A  │
    │    │    │       │    │    │       │    │    │
    │    ▼    │       │    ▼    │       │    ▼    │
    │ Proc B  │       │ Proc B  │       │ Proc B  │
    │    │    │       │    │    │       │    │    │
    │    ▼    │       │    ▼    │       │    ▼    │
    │ Result  │       │ Result  │       │ Result  │
    └────┬────┘       └────┬────┘       └────┬────┘
         │                 │                 │
         └─────────────────┼─────────────────┘
                           │
                           ▼
                  ┌────────────────┐
                  │ ProcessPhase   │
                  │   Result       │
                  └────────────────┘
```

## Update `build/mod.rs`

Add exports:

```rust
mod process;
pub use process::{ProcessPhase, ProcessPhaseResult};
```

## Verification

```bash
cargo check -p rpp
cargo test -p rpp worker
cargo test -p rpp process
```
