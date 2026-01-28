//! Sandboxed API for Lua plugins.
//!
//! Provides safe, tracked access to file operations, hashing,
//! JSON encoding/decoding, and logging.

mod file_api;
mod hash_api;
mod json_api;
mod log_api;

pub use file_api::FileApi;
pub use hash_api::HashApi;
pub use json_api::JsonApi;
pub use log_api::LogApi;

use std::cell::RefCell;
use std::path::PathBuf;

/// Sandboxed context provided to Lua plugins.
/// Tracks all file operations for cache invalidation.
pub struct SandboxContext {
    pub file: FileApi,
    pub hash: HashApi,
    pub json: JsonApi,
    pub log: LogApi,

    /// Files read during this operation (for dependency tracking)
    read_files: RefCell<Vec<PathBuf>>,
    /// Files written during this operation
    written_files: RefCell<Vec<PathBuf>>,
}

impl SandboxContext {
    pub fn new(source_root: PathBuf, output_root: PathBuf) -> Self {
        Self {
            file: FileApi::new(source_root, output_root),
            hash: HashApi::new(),
            json: JsonApi::new(),
            log: LogApi::new(),
            read_files: RefCell::new(Vec::new()),
            written_files: RefCell::new(Vec::new()),
        }
    }

    /// Record that a file was read (for dependency tracking).
    pub fn record_read(&self, path: PathBuf) {
        self.read_files.borrow_mut().push(path);
    }

    /// Record that a file was written.
    pub fn record_write(&self, path: PathBuf) {
        self.written_files.borrow_mut().push(path);
    }

    /// Get all files read during this context's lifetime.
    pub fn read_files(&self) -> Vec<PathBuf> {
        self.read_files.borrow().clone()
    }

    /// Get all files written during this context's lifetime.
    pub fn written_files(&self) -> Vec<PathBuf> {
        self.written_files.borrow().clone()
    }
}
