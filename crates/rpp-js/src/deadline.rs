use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use deno_core::v8;

use crate::model::Cancellation;
use crate::termination::{Reason, Termination};

struct Active {
    handle: v8::IsolateHandle,
    cancellation: Cancellation,
    termination: Termination,
}

struct Shared {
    origin: Instant,
    at: AtomicU64,
    active: Mutex<Option<Active>>,
    stopped: AtomicBool,
}

impl Shared {
    fn now(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_micros()).unwrap_or(u64::MAX)
    }
}

/// One watchdog serves every runtime of an engine. It sleeps indefinitely while idle.
pub(crate) struct Deadline {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl Deadline {
    pub(crate) fn new() -> std::io::Result<Self> {
        let shared = Arc::new(Shared {
            origin: Instant::now(),
            at: AtomicU64::new(0),
            active: Mutex::new(None),
            stopped: AtomicBool::new(false),
        });
        let watch = Arc::clone(&shared);
        let thread = thread::Builder::new()
            .name("rpp-js-deadline".into())
            .spawn(move || {
                while !watch.stopped.load(Ordering::Acquire) {
                    if watch.at.load(Ordering::Acquire) == 0 {
                        thread::park();
                        continue;
                    }
                    thread::park_timeout(Duration::from_millis(2));
                    // Hold the lock through termination so disarming cannot race with
                    // a stale timeout that interrupts the next call.
                    let active = watch.active.lock().unwrap();
                    if let Some(active) = active.as_ref() {
                        let cancelled = active.cancellation.is_cancelled();
                        if cancelled || watch.now() >= watch.at.load(Ordering::Acquire) {
                            active.termination.record(if cancelled {
                                Reason::Cancelled
                            } else {
                                Reason::Deadline
                            });
                            active.handle.terminate_execution();
                            watch.at.store(0, Ordering::Release);
                        }
                    }
                }
            })?;
        Ok(Self {
            shared,
            thread: Some(thread),
        })
    }

    pub(crate) fn arm(
        &self,
        handle: v8::IsolateHandle,
        cancellation: Cancellation,
        budget: Duration,
        termination: Termination,
    ) -> Guard<'_> {
        let mut active = self.shared.active.lock().unwrap();
        assert!(active.is_none(), "only one execution can run at a time");
        *active = Some(Active {
            handle,
            cancellation,
            termination,
        });
        let micros = u64::try_from(budget.as_micros()).unwrap_or(u64::MAX).max(1);
        self.shared
            .at
            .store(self.shared.now().saturating_add(micros), Ordering::Release);
        self.thread
            .as_ref()
            .expect("watchdog thread")
            .thread()
            .unpark();
        Guard(self)
    }
}

pub(crate) struct Guard<'a>(&'a Deadline);

impl Drop for Guard<'_> {
    fn drop(&mut self) {
        let mut active = self.0.shared.active.lock().unwrap();
        self.0.shared.at.store(0, Ordering::Release);
        active.take();
    }
}

impl Drop for Deadline {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}
