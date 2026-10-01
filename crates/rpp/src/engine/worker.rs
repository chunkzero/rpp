//! Worker pool: one set of plugin instances per worker thread (spec §4, §7).

use std::sync::Arc;
use std::thread::{self, JoinHandle};

use crossbeam_channel::{bounded, Receiver, Sender};

use crate::engine::keys::ChainStep;
use crate::error::{Error, Result};
use crate::model::{PackFile, PluginFactory, PluginInstance, ProcessOutcome};
use crate::util::path::validate_relative;

/// A unit of work: one file plus the ordered processor chain to apply.
pub(crate) struct Job {
    /// The source file's relative path (for diagnostics and result routing).
    pub(crate) rel: String,
    /// The file to process (mutated in place by the chain).
    pub(crate) file: PackFile,
    /// The ordered processor chain.
    pub(crate) chain: Arc<Vec<ChainStep>>,
}

/// The outcome of processing one job.
pub(crate) enum JobOutcome {
    /// The file survived (possibly modified, possibly renamed).
    Produced { rel: String, file: PackFile },
    /// The file was dropped by a processor.
    Dropped { rel: String },
}

type JobResult = Result<JobOutcome>;

/// A pool of worker threads, each holding live plugin instances.
pub(crate) struct WorkerPool {
    workers: Vec<JoinHandle<()>>,
    job_tx: Option<Sender<Job>>,
    result_rx: Receiver<JobResult>,
}

impl WorkerPool {
    /// Spawn `count` workers, each instantiating every factory once.
    ///
    /// Instantiation happens on the worker thread (plugin runtimes are per-worker).
    /// If any worker fails to instantiate, that error surfaces on the result
    /// channel as the first job is awaited.
    pub(crate) fn new(count: usize, factories: Arc<Vec<Arc<dyn PluginFactory>>>) -> Self {
        let (job_tx, job_rx) = bounded::<Job>(count);
        let (result_tx, result_rx) = bounded::<JobResult>(count);
        let job_rx = Arc::new(job_rx);

        let mut workers = Vec::with_capacity(count);
        for _ in 0..count {
            let job_rx = Arc::clone(&job_rx);
            let result_tx = result_tx.clone();
            let factories = Arc::clone(&factories);
            workers.push(thread::spawn(move || {
                worker_loop(job_rx, result_tx, factories);
            }));
        }

        WorkerPool {
            workers,
            job_tx: Some(job_tx),
            result_rx,
        }
    }

    /// Submit a job to the pool.
    pub(crate) fn submit(&self, job: Job) -> Result<()> {
        self.job_tx
            .as_ref()
            .ok_or_else(|| Error::Build("worker pool already shut down".into()))?
            .send(job)
            .map_err(|_| Error::Build("worker pool disconnected".into()))
    }

    /// Receive the next result, blocking until one is available.
    pub(crate) fn recv(&self) -> Option<JobResult> {
        self.result_rx.recv().ok()
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        self.job_tx = None;
        // Drain bounded results before joining, including on a failed build.
        while self.result_rx.recv().is_ok() {}
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

fn worker_loop(
    job_rx: Arc<Receiver<Job>>,
    result_tx: Sender<JobResult>,
    factories: Arc<Vec<Arc<dyn PluginFactory>>>,
) {
    // Instantiate one live instance per factory, indexed by factory position.
    let mut instances: Vec<Box<dyn PluginInstance>> = Vec::with_capacity(factories.len());
    for factory in factories.iter() {
        match factory.instantiate() {
            Ok(inst) => instances.push(inst),
            Err(e) => {
                // Report the load error and stop this worker.
                let _ = result_tx.send(Err(e));
                return;
            }
        }
    }

    while let Ok(job) = job_rx.recv() {
        let result = run_chain(&mut instances, job);
        if result_tx.send(result).is_err() {
            break;
        }
    }
}

fn run_chain(instances: &mut [Box<dyn PluginInstance>], job: Job) -> JobResult {
    let Job {
        rel,
        mut file,
        chain,
    } = job;

    for step in chain.iter() {
        let instance = &mut instances[step.plugin_index];
        let outcome = instance.process(&step.processor, &mut file)?;
        if let Err(message) = validate_relative(&file.path) {
            return Err(Error::Processor {
                plugin: step.plugin_id.clone(),
                processor: step.processor.clone(),
                file: rel.clone(),
                message,
            });
        }
        match outcome {
            ProcessOutcome::Dropped => {
                return Ok(JobOutcome::Dropped { rel });
            }
            ProcessOutcome::Modified | ProcessOutcome::Unchanged => {}
        }
    }

    Ok(JobOutcome::Produced { rel, file })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_pool_drains_full_result_channel() {
        let pool = WorkerPool::new(1, Arc::new(Vec::new()));
        for index in 0..3 {
            let rel = format!("{index}.txt");
            pool.submit(Job {
                file: PackFile::new(rel.clone(), vec![b'x']),
                rel,
                chain: Arc::new(Vec::new()),
            })
            .unwrap();
        }
        // Shutdown must unblock workers even when the caller never receives.
        drop(pool);
    }
}
