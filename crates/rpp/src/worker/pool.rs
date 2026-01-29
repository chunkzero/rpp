use crossbeam_channel::{unbounded, Receiver, Sender};
use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use twox_hash::XxHash3_64;

use crate::build::{BuildError, ProcessedFile, Transformation};
use crate::lua::LuaRuntime;
use crate::plugin::{Plugin, ProcessResult, ProcessingContext};

#[cfg(feature = "lua")]
use crate::plugin::LuaProcessor;

use super::job::{ProcessingJob, ProcessingResult};

// Private generation counter for hot reload
static PLUGIN_RELOAD_GENERATION: AtomicU64 = AtomicU64::new(0);

// Public API for invalidation
pub fn invalidate_lua_runtimes() {
    PLUGIN_RELOAD_GENERATION.fetch_add(1, Ordering::SeqCst);
}

struct RuntimeState {
    runtime: LuaRuntime,
    generation: u64,
}

thread_local! {
    static WORKER_RUNTIME: RefCell<Option<RuntimeState>> = RefCell::new(None);
}

fn with_runtime<F, R>(f: F) -> R
where
    F: FnOnce(&mut LuaRuntime) -> R,
{
    let current_gen = PLUGIN_RELOAD_GENERATION.load(Ordering::SeqCst);

    WORKER_RUNTIME.with(|cell| {
        let mut opt = cell.borrow_mut();

        // Check if runtime needs recreation
        let needs_refresh = match opt.as_ref() {
            Some(state) => state.generation != current_gen,
            None => true,
        };

        if needs_refresh {
            let runtime = LuaRuntime::new().expect("Failed to create worker Lua runtime");
            *opt = Some(RuntimeState {
                runtime,
                generation: current_gen,
            });
        }

        let state = opt.as_mut().unwrap();
        f(&mut state.runtime)
    })
}

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

        // Group consecutive Lua processors into batches, preserving order
        let mut i = 0;
        while i < job.processors.len() {
            let processor = &job.processors[i];

            #[cfg(feature = "lua")]
            {
                // Check if this is a Lua processor
                if let Some(lua_proc) = processor.as_any().downcast_ref::<LuaProcessor>() {
                    // Found Lua processor - collect consecutive batch
                    let mut lua_batch = vec![(
                        lua_proc.name().to_string(),
                        lua_proc.version().to_string(),
                        lua_proc.source().to_string(),
                    )];

                    i += 1;
                    while i < job.processors.len() {
                        if let Some(next_lua) =
                            job.processors[i].as_any().downcast_ref::<LuaProcessor>()
                        {
                            lua_batch.push((
                                next_lua.name().to_string(),
                                next_lua.version().to_string(),
                                next_lua.source().to_string(),
                            ));
                            i += 1;
                        } else {
                            break; // Hit non-Lua processor, stop batch
                        }
                    }

                    // Process Lua batch efficiently
                    let chain_result = with_runtime(|runtime| {
                        // Ensure all plugins loaded
                        for (name, version, source) in &lua_batch {
                            runtime.load_plugin_versioned(name, version, source)?;
                        }

                        // Process entire batch in one Lua call
                        let processor_ids: Vec<_> = lua_batch
                            .iter()
                            .map(|(name, version, _)| (name.clone(), version.clone()))
                            .collect();

                        runtime.process_chain(
                            &job.file.relative_path.to_string_lossy(),
                            &content,
                            &processor_ids,
                        )
                    });

                    match chain_result {
                        Ok(result) if result.cancelled => {
                            return ProcessingResult::Cancelled {
                                path: job.file.source_path,
                            };
                        }
                        Ok(result) if result.skipped_at.is_some() => {
                            // Skip means stop processing entirely
                            return ProcessingResult::Processed(ProcessedFile {
                                source_path: job.file.source_path,
                                output_path,
                                content,
                                transformations,
                                dependencies: Vec::new(),
                            });
                        }
                        Ok(result) => {
                            // Record transformations with per-step hashes
                            for (name, version, input_hash, output_hash) in result.transformations {
                                transformations.push(Transformation {
                                    processor: name,
                                    version,
                                    input_hash,
                                    output_hash,
                                });
                            }

                            content = result.content;
                            if let Some(p) = result.output_path {
                                output_path = std::path::PathBuf::from(p);
                            }
                        }
                        Err(e) => {
                            return ProcessingResult::Error {
                                path: job.file.source_path,
                                error: e,
                            };
                        }
                    }

                    continue; // Continue with next processor after batch
                }
            }

            // Non-Lua processor - process normally
            let input_hash = XxHash3_64::oneshot(&content);

            let ctx = ProcessingContext {
                path: &job.file.relative_path,
                content: &content,
                source_path: &job.file.source_path,
                config: &job.config,
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
                    // Skip means stop processing this file entirely
                    return ProcessingResult::Processed(ProcessedFile {
                        source_path: job.file.source_path,
                        output_path,
                        content,
                        transformations,
                        dependencies: Vec::new(),
                    });
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

            i += 1;
        }

        ProcessingResult::Processed(ProcessedFile {
            source_path: job.file.source_path,
            output_path,
            content,
            transformations,
            dependencies: Vec::new(), // TODO: Capture from SandboxContext when file API is exposed
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

        fn as_any(&self) -> &dyn std::any::Any {
            self
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
            config: toml::Value::Table(toml::map::Map::new()),
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
                config: toml::Value::Table(toml::map::Map::new()),
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
