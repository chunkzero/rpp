//! Worker pool: one set of plugin instances per worker thread (spec §4, §7).

use std::path::PathBuf;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use crossbeam_channel::{bounded, Receiver, Sender};

use crate::cache::{FileEntry, Fingerprint, ObjectStore, OutputRef};
use crate::engine::cache_replay::cached_outputs;
use crate::engine::keys::ChainStep;
use crate::engine::output::OutputContent;
use crate::error::{Error, Result};
use crate::model::{PackFile, PluginFactory, PluginInstance, ProcessOutcome};
use crate::source::SourceFile;
use crate::util::path::validate_relative;

/// What every worker shares: the plugin factories and where results live.
pub(crate) struct WorkerContext {
    pub(crate) factories: Arc<Vec<Arc<dyn PluginFactory>>>,
    pub(crate) store: ObjectStore,
    /// The pack output directory, consulted when validating cached outputs.
    pub(crate) output_dir: PathBuf,
}

/// A unit of work: one source file, its processor chain, and its reusable cache entry.
pub(crate) struct Job {
    pub(crate) source: SourceFile,
    /// The ordered processor chain.
    pub(crate) chain: Vec<ChainStep>,
    /// The previous entry, when its chain still applies and is cacheable.
    pub(crate) cached: Option<FileEntry>,
}

/// The outcome of one job.
pub(crate) enum JobOutcome {
    /// The source is unchanged and every cached output resolved, in `entry.outputs` order.
    Cached {
        entry: FileEntry,
        contents: Vec<OutputContent>,
    },
    /// The chain ran; `outputs` is empty when a processor dropped the file.
    Processed {
        fingerprint: Fingerprint,
        outputs: Vec<OutputRef>,
    },
}

/// A job's source path and outcome.
pub(crate) struct JobResult {
    pub(crate) rel: String,
    pub(crate) outcome: Result<JobOutcome>,
}

/// A pool of worker threads, each holding live plugin instances.
pub(crate) struct WorkerPool {
    workers: Vec<JoinHandle<()>>,
    job_tx: Option<Sender<Job>>,
    result_rx: Receiver<JobResult>,
}

impl WorkerPool {
    /// Spawn `count` workers.
    ///
    /// Each worker instantiates a factory on its own thread (plugin runtimes are per-worker)
    /// the first time a chain uses it; an instantiation error fails that job.
    pub(crate) fn new(count: usize, context: Arc<WorkerContext>) -> Self {
        let (job_tx, job_rx) = bounded::<Job>(count);
        let (result_tx, result_rx) = bounded::<JobResult>(count);

        let mut workers = Vec::with_capacity(count);
        for _ in 0..count {
            let job_rx = job_rx.clone();
            let result_tx = result_tx.clone();
            let context = Arc::clone(&context);
            workers.push(thread::spawn(move || {
                worker_loop(&job_rx, &result_tx, &context);
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

fn worker_loop(job_rx: &Receiver<Job>, result_tx: &Sender<JobResult>, context: &WorkerContext) {
    // Live instances by factory position, created on first use.
    let mut instances: Vec<Option<Box<dyn PluginInstance>>> =
        context.factories.iter().map(|_| None).collect();
    while let Ok(job) = job_rx.recv() {
        let rel = job.source.rel.clone();
        let outcome = run_job(context, &mut instances, job);
        if result_tx.send(JobResult { rel, outcome }).is_err() {
            break;
        }
    }
}

/// Serve the job from its cache entry, or run its chain and store the result.
fn run_job(
    context: &WorkerContext,
    instances: &mut [Option<Box<dyn PluginInstance>>],
    job: Job,
) -> Result<JobOutcome> {
    let Job {
        source,
        chain,
        cached,
    } = job;
    let (fingerprint, contents) = source.fingerprint()?;
    if let Some(entry) = cached.filter(|entry| entry.fingerprint.xxh3 == fingerprint.xxh3) {
        if let Some(contents) = cached_outputs(&context.store, &context.output_dir, &entry.outputs)
        {
            return Ok(JobOutcome::Cached { entry, contents });
        }
    }

    let file = PackFile::new(source.rel.clone(), contents);
    let outputs = match run_chain(&context.factories, instances, &chain, &source.rel, file)? {
        Some(file) => vec![OutputRef {
            object: context.store.put(&file.contents)?,
            path: file.path,
        }],
        None => Vec::new(),
    };
    Ok(JobOutcome::Processed {
        fingerprint,
        outputs,
    })
}

/// Apply `chain` to `file`, returning `None` when a processor drops it.
fn run_chain(
    factories: &[Arc<dyn PluginFactory>],
    instances: &mut [Option<Box<dyn PluginInstance>>],
    chain: &[ChainStep],
    rel: &str,
    mut file: PackFile,
) -> Result<Option<PackFile>> {
    for step in chain {
        let instance = match &mut instances[step.plugin_index] {
            Some(instance) => instance,
            slot => slot.insert(factories[step.plugin_index].instantiate()?),
        };
        let outcome = instance.process(&step.processor, &mut file)?;
        if let Err(message) = validate_relative(&file.path) {
            return Err(Error::Processor {
                plugin: step.plugin_id.clone(),
                processor: step.processor.clone(),
                file: rel.to_string(),
                message,
            });
        }
        match outcome {
            ProcessOutcome::Dropped => return Ok(None),
            ProcessOutcome::Modified | ProcessOutcome::Unchanged => {}
        }
    }
    Ok(Some(file))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_pool_drains_full_result_channel() {
        let dir = tempfile::tempdir().unwrap();
        let context = WorkerContext {
            factories: Arc::new(Vec::new()),
            store: ObjectStore::open(dir.path().join("objects")).unwrap(),
            output_dir: dir.path().join("dist"),
        };
        let pool = WorkerPool::new(1, Arc::new(context));
        for index in 0..3 {
            let abs = dir.path().join(format!("{index}.txt"));
            std::fs::write(&abs, b"x").unwrap();
            pool.submit(Job {
                source: SourceFile {
                    rel: format!("{index}.txt"),
                    abs,
                    size: 1,
                    mtime_ns: 0,
                },
                chain: Vec::new(),
                cached: None,
            })
            .unwrap();
        }
        // Shutdown must unblock workers even when the caller never receives.
        drop(pool);
    }
}
