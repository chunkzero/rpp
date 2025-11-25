use std::sync::Arc;
use std::thread::JoinHandle;

use crossbeam_channel::{Receiver, Sender};
use parking_lot::Mutex;
use twox_hash::XxHash3_64;

use crate::compile::cache::{Fingerprint, SourceState};
use crate::compile::context::{BuildContext, SimpleBuildContext};
use crate::compile::event::{BuildEvent, EventHandler};

pub type EventHandlerProvider = Box<dyn (Fn() -> Box<dyn EventHandler>) + Send + Sync>;

pub struct Job {
    pub path: std::path::PathBuf,
    pub path_str: String,
    pub mtime: u64,
    pub size: u64,
}

pub struct JobResult {
    pub path: String,
    pub state: SourceState,
}

pub struct WorkerPool {
    event_handler_providers: Arc<Vec<EventHandlerProvider>>,
    _workers: Mutex<Vec<Worker>>,
    job_tx: Sender<Option<Job>>,
    result_rx: Receiver<JobResult>,
    max_workers: usize,
}

struct Worker {
    join_handle: JoinHandle<()>,
}

impl WorkerPool {
    pub fn new(event_handlers: Vec<EventHandlerProvider>, max_workers: usize) -> Self {
        let (job_tx, job_rx) = crossbeam_channel::unbounded::<Option<Job>>();
        let (result_tx, _result_rx) = crossbeam_channel::unbounded::<JobResult>();

        let event_handler_providers = Arc::new(event_handlers);
        let mut workers = Vec::new();

        for thread_id in 0..max_workers {
            let providers = Arc::clone(&event_handler_providers);
            let job_rx = job_rx.clone();
            let result_tx = result_tx.clone();

            let join_handle = std::thread::spawn(move || {
                let handlers: Vec<Box<dyn EventHandler>> = providers.iter().map(|p| p()).collect();

                while let Ok(Some(job)) = job_rx.recv() {
                    let mut context = SimpleBuildContext {
                        path: job.path_str.clone(),
                        mtime: job.mtime,
                        size: job.size,
                        hash: 0,
                        dependencies: Vec::new(),
                        outputs: Vec::new(),
                    };

                    for handler in &handlers {
                        let _ = handler.handle_event(thread_id, BuildEvent::Begin(&context));
                    }

                    if let Ok(contents) = std::fs::read(&job.path) {
                        context.set_hash(XxHash3_64::oneshot(&contents));

                        for handler in &handlers {
                            let _ =
                                handler.handle_event(thread_id, BuildEvent::ProcessFile(&context));
                        }
                    }

                    for handler in &handlers {
                        let _ = handler.handle_event(thread_id, BuildEvent::End(&context));
                    }

                    let result = JobResult {
                        path: job.path_str,
                        state: SourceState {
                            fingerprint: Fingerprint {
                                mtime: context.mtime(),
                                size: context.size(),
                                hash: context.hash(),
                            },
                            dependencies: context.dependencies().to_vec(),
                            outputs: context.outputs().to_vec(),
                        },
                    };

                    let _ = result_tx.send(result);
                }
            });

            workers.push(Worker { join_handle });
        }

        Self {
            event_handler_providers,
            _workers: Mutex::new(workers),
            job_tx,
            result_rx: _result_rx,
            max_workers,
        }
    }

    pub fn submit_job(&self, job: Job) -> Result<(), crossbeam_channel::SendError<Option<Job>>> {
        self.job_tx.send(Some(job))
    }

    pub fn recv_result(&self) -> Result<JobResult, crossbeam_channel::RecvError> {
        self.result_rx.recv()
    }

    pub fn try_recv_result(&self) -> Result<JobResult, crossbeam_channel::TryRecvError> {
        self.result_rx.try_recv()
    }

    pub fn iter_results(&self) -> impl Iterator<Item = JobResult> + '_ {
        self.result_rx.iter()
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        for _ in 0..self.max_workers {
            let _ = self.job_tx.send(None);
        }
    }
}
