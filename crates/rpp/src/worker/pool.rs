use crossbeam_channel::{unbounded, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
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
                Ok(ProcessResult::Continue {
                    content: new_content,
                    output_path: new_path,
                }) => {
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
    pub fn shutdown(mut self) {
        for _ in 0..self.num_workers {
            let _ = self.job_tx.send(None);
        }
        // Drain the workers vec to join them before drop runs
        while let Some(worker) = self.workers.pop() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{FileEntry, Fingerprint};
    use crate::plugin::{Plugin, ProcessResult, ProcessingContext, ProcessorPlugin};
    use std::path::PathBuf;

    struct TestProcessor {
        name: String,
        version: String,
    }

    impl Plugin for TestProcessor {
        fn name(&self) -> &str {
            &self.name
        }

        fn version(&self) -> &str {
            &self.version
        }
    }

    impl ProcessorPlugin for TestProcessor {
        fn patterns(&self) -> &[String] {
            &[]
        }

        fn process(
            &self,
            ctx: &ProcessingContext,
        ) -> Result<ProcessResult, crate::build::BuildError> {
            // Simple processor that just appends to content
            let mut new_content = ctx.content.to_vec();
            new_content.extend_from_slice(b"-processed");
            Ok(ProcessResult::continue_with(new_content))
        }
    }

    #[test]
    fn test_worker_pool_basic() {
        let pool = WorkerPool::new(2);

        let file = FileEntry {
            source_path: PathBuf::from("/test/file.txt"),
            relative_path: PathBuf::from("file.txt"),
            fingerprint: Fingerprint {
                mtime: 0,
                size: 5,
                hash: 0,
            },
            content: b"hello".to_vec(),
        };

        let processor: Arc<dyn ProcessorPlugin> = Arc::new(TestProcessor {
            name: "test".into(),
            version: "1.0.0".into(),
        });

        let job = ProcessingJob {
            file,
            processors: vec![processor],
        };

        pool.submit(job).unwrap();

        let result = pool.results().next().unwrap();
        match result {
            ProcessingResult::Processed(processed) => {
                assert_eq!(processed.content, b"hello-processed");
                assert_eq!(processed.transformations.len(), 1);
                assert_eq!(processed.transformations[0].processor, "test");
            }
            _ => panic!("Expected Processed result"),
        }

        pool.shutdown();
    }

    #[test]
    fn test_worker_pool_multiple_jobs() {
        let pool = WorkerPool::new(2);

        let processor: Arc<dyn ProcessorPlugin> = Arc::new(TestProcessor {
            name: "test".into(),
            version: "1.0.0".into(),
        });

        // Submit multiple jobs
        for i in 0..5 {
            let file = FileEntry {
                source_path: PathBuf::from(format!("/test/file{}.txt", i)),
                relative_path: PathBuf::from(format!("file{}.txt", i)),
                fingerprint: Fingerprint {
                    mtime: 0,
                    size: 5,
                    hash: 0,
                },
                content: format!("test{}", i).into_bytes(),
            };

            let job = ProcessingJob {
                file,
                processors: vec![Arc::clone(&processor)],
            };

            pool.submit(job).unwrap();
        }

        // Collect all results
        let mut results = Vec::new();
        for _ in 0..5 {
            if let Some(result) = pool.results().next() {
                results.push(result);
            }
        }

        assert_eq!(results.len(), 5);
        pool.shutdown();
    }
}
