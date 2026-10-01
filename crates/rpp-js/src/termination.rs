use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};

use crate::error::Error;

/// The first actual termination signal wins, independent of teardown duration.
#[derive(Clone, Default)]
pub(crate) struct Termination(Arc<AtomicU8>);

impl Termination {
    pub fn record(&self, reason: Reason) {
        let _ = self
            .0
            .compare_exchange(0, reason as u8, Ordering::AcqRel, Ordering::Acquire);
    }

    pub fn take(&self) -> Option<Error> {
        match self.0.swap(0, Ordering::AcqRel) {
            1 => Some(Error::Cancelled),
            2 => Some(Error::Deadline),
            3 => Some(Error::Heap),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Reason {
    Cancelled = 1,
    Deadline = 2,
    Heap = 3,
}
