//! Values passed between the host and JavaScript.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

/// Resource limits for one runtime, applied to module evaluation and to each call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// V8 heap plus `ArrayBuffer` backing stores, in bytes. At least 16 MiB.
    pub heap_bytes: usize,
    /// Wall-clock budget for one evaluation or call, including awaited promises.
    pub time: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            heap_bytes: 256 * 1024 * 1024,
            time: Duration::from_secs(30),
        }
    }
}

/// What `Date` and `Math.random` observe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clock {
    /// `Date.now()` and `new Date()` return `timestamp_ms`; `Math.random` is SplitMix64
    /// seeded with `seed`. Two runs with the same values observe the same sequence.
    Fixed {
        /// Milliseconds since the Unix epoch.
        timestamp_ms: i64,
        /// Random seed.
        seed: u64,
    },
    /// The real clock and a nondeterministic `Math.random`.
    Real,
}

impl Default for Clock {
    fn default() -> Self {
        Self::Fixed {
            timestamp_ms: 0,
            seed: 0,
        }
    }
}

/// A cancellation flag shared across threads. Cancelling terminates the running call.
#[derive(Debug, Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    /// A fresh, uncancelled flag.
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cancellation.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    /// Whether cancellation was requested.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// Severity of a `console` message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    /// `console.debug`.
    Debug,
    /// `console.log` and `console.info`.
    Info,
    /// `console.warn`.
    Warn,
    /// `console.error`.
    Error,
}

/// One `console` message. Arguments are joined with spaces; non-strings are formatted
/// like `JSON.stringify`, falling back to `String(value)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Log {
    /// Severity.
    pub level: LogLevel,
    /// Formatted message.
    pub message: String,
}

/// One invocation of an exported function: `export(args, bytes)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Call<'a> {
    /// Name of the exported function.
    pub export: &'a str,
    /// First argument, as JSON.
    pub args: Value,
    /// Second argument as a `Uint8Array`, or `undefined` when `None`.
    pub bytes: Option<Vec<u8>>,
    /// Clock for this call.
    pub clock: Clock,
}

/// The settled result of a call.
#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    /// The return value as JSON; `null` when the function returned `undefined` or a
    /// `Uint8Array`.
    pub value: Value,
    /// The return value when it was a `Uint8Array`.
    pub bytes: Option<Vec<u8>>,
    /// `console` messages emitted during the call, in order.
    pub logs: Vec<Log>,
}

/// A host function's result.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HostReply {
    /// JSON result.
    pub value: Value,
    /// Binary result, delivered to JavaScript as a `Uint8Array`.
    pub bytes: Option<Vec<u8>>,
}

/// Host functions available to JavaScript during [`crate::Runtime::call`] as
/// `__rpp.call(name, value, bytes?)`, which returns `{ value, bytes }` synchronously
/// (`bytes` is `undefined` when absent) or throws an `Error` with the returned message.
/// Not available during module evaluation.
pub trait Host {
    /// Handle one host call.
    ///
    /// # Errors
    ///
    /// The message becomes a JavaScript `Error`.
    fn call(
        &mut self,
        name: &str,
        value: Value,
        bytes: Option<Vec<u8>>,
    ) -> Result<HostReply, String>;
}
