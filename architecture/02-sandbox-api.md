# Commit 2: Sandbox API Implementation

**Goal**: Create sandboxed context APIs for Lua plugins.

## Files to Create

```
crates/rpp/src/
├── sandbox/
│   ├── mod.rs
│   ├── file_api.rs
│   ├── hash_api.rs
│   ├── json_api.rs
│   └── log_api.rs
```

## `sandbox/mod.rs`

```rust
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
```

## `sandbox/file_api.rs`

```rust
use std::fs;
use std::path::{Path, PathBuf};
use crate::build::BuildError;

/// Sandboxed file API with path validation.
pub struct FileApi {
    source_root: PathBuf,
    output_root: PathBuf,
}

impl FileApi {
    pub fn new(source_root: PathBuf, output_root: PathBuf) -> Self {
        Self { source_root, output_root }
    }

    /// Read a file from the source directory (read-only).
    pub fn read_source(&self, path: &str) -> Result<Vec<u8>, BuildError> {
        let full_path = self.validate_source_path(path)?;
        fs::read(&full_path).map_err(|e| BuildError::FileRead {
            path: full_path,
            source: e,
        })
    }

    /// Read a file from the output directory.
    pub fn read_output(&self, path: &str) -> Result<Vec<u8>, BuildError> {
        let full_path = self.validate_output_path(path)?;
        fs::read(&full_path).map_err(|e| BuildError::FileRead {
            path: full_path,
            source: e,
        })
    }

    /// Write a file to the output directory.
    pub fn write_file(&self, path: &str, content: &[u8]) -> Result<(), BuildError> {
        let full_path = self.validate_output_path(path)?;

        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).map_err(|e| BuildError::FileWrite {
                path: full_path.clone(),
                source: e,
            })?;
        }

        fs::write(&full_path, content).map_err(|e| BuildError::FileWrite {
            path: full_path,
            source: e,
        })
    }

    /// Delete a file from the output directory.
    pub fn delete_file(&self, path: &str) -> Result<(), BuildError> {
        let full_path = self.validate_output_path(path)?;
        if full_path.exists() {
            fs::remove_file(&full_path).map_err(|e| BuildError::FileWrite {
                path: full_path,
                source: e,
            })?;
        }
        Ok(())
    }

    /// Check if a file exists in source.
    pub fn exists_source(&self, path: &str) -> bool {
        self.validate_source_path(path)
            .map(|p| p.exists())
            .unwrap_or(false)
    }

    /// Check if a file exists in output.
    pub fn exists_output(&self, path: &str) -> bool {
        self.validate_output_path(path)
            .map(|p| p.exists())
            .unwrap_or(false)
    }

    /// List files in a source directory.
    pub fn list_source_dir(&self, path: &str) -> Result<Vec<String>, BuildError> {
        let full_path = self.validate_source_path(path)?;
        self.list_dir_impl(&full_path)
    }

    /// List files in an output directory.
    pub fn list_output_dir(&self, path: &str) -> Result<Vec<String>, BuildError> {
        let full_path = self.validate_output_path(path)?;
        self.list_dir_impl(&full_path)
    }

    fn list_dir_impl(&self, path: &Path) -> Result<Vec<String>, BuildError> {
        let entries = fs::read_dir(path).map_err(|e| BuildError::FileRead {
            path: path.to_path_buf(),
            source: e,
        })?;

        let mut files = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| BuildError::FileRead {
                path: path.to_path_buf(),
                source: e,
            })?;
            if let Some(name) = entry.file_name().to_str() {
                files.push(name.to_string());
            }
        }
        Ok(files)
    }

    fn validate_source_path(&self, path: &str) -> Result<PathBuf, BuildError> {
        self.validate_path_within(&self.source_root, path)
    }

    fn validate_output_path(&self, path: &str) -> Result<PathBuf, BuildError> {
        self.validate_path_within(&self.output_root, path)
    }

    /// Validate path is within root and doesn't escape via traversal.
    fn validate_path_within(&self, root: &Path, path: &str) -> Result<PathBuf, BuildError> {
        let requested = PathBuf::from(path);

        // Reject absolute paths
        if requested.is_absolute() {
            return Err(BuildError::Plugin(format!(
                "Absolute paths not allowed: {}", path
            )));
        }

        let full_path = root.join(&requested);

        // Check for directory traversal
        for component in requested.components() {
            use std::path::Component;
            match component {
                Component::ParentDir => {
                    return Err(BuildError::Plugin(format!(
                        "Path traversal not allowed: {}", path
                    )));
                }
                Component::Prefix(_) => {
                    return Err(BuildError::Plugin(format!(
                        "Invalid path component: {}", path
                    )));
                }
                _ => {}
            }
        }

        // If file exists, verify canonical path is within root
        if full_path.exists() {
            let canonical = full_path.canonicalize().map_err(|e| BuildError::FileRead {
                path: full_path.clone(),
                source: e,
            })?;
            let canonical_root = root.canonicalize().map_err(|e| BuildError::FileRead {
                path: root.to_path_buf(),
                source: e,
            })?;

            if !canonical.starts_with(&canonical_root) {
                return Err(BuildError::Plugin(format!(
                    "Path escapes sandbox: {}", path
                )));
            }
        }

        Ok(full_path)
    }
}
```

## `sandbox/hash_api.rs`

```rust
use twox_hash::XxHash3_64;
use sha2::{Sha256, Digest};
use md5::Md5;

/// Hashing API for plugins.
pub struct HashApi;

impl HashApi {
    pub fn new() -> Self { Self }

    /// Fast non-cryptographic hash (xxhash3_64).
    pub fn xxhash3(&self, data: &[u8]) -> u64 {
        XxHash3_64::oneshot(data)
    }

    /// SHA-256 cryptographic hash (lowercase hex).
    pub fn sha256(&self, data: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data);
        format!("{:x}", hasher.finalize())
    }

    /// MD5 hash (lowercase hex). For compatibility only.
    pub fn md5(&self, data: &[u8]) -> String {
        let mut hasher = Md5::new();
        hasher.update(data);
        format!("{:x}", hasher.finalize())
    }
}

impl Default for HashApi {
    fn default() -> Self { Self::new() }
}
```

## `sandbox/json_api.rs`

```rust
use serde_json::Value;
use crate::build::BuildError;

/// JSON encoding/decoding API for plugins.
pub struct JsonApi;

impl JsonApi {
    pub fn new() -> Self { Self }

    /// Decode JSON bytes to a Value.
    pub fn decode(&self, data: &[u8]) -> Result<Value, BuildError> {
        serde_json::from_slice(data)
            .map_err(|e| BuildError::Plugin(format!("JSON decode error: {}", e)))
    }

    /// Decode JSON string to a Value.
    pub fn decode_str(&self, s: &str) -> Result<Value, BuildError> {
        serde_json::from_str(s)
            .map_err(|e| BuildError::Plugin(format!("JSON decode error: {}", e)))
    }

    /// Encode a Value to pretty-printed JSON bytes.
    pub fn encode(&self, value: &Value) -> Result<Vec<u8>, BuildError> {
        serde_json::to_vec_pretty(value)
            .map_err(|e| BuildError::Plugin(format!("JSON encode error: {}", e)))
    }

    /// Encode a Value to compact JSON bytes.
    pub fn encode_compact(&self, value: &Value) -> Result<Vec<u8>, BuildError> {
        serde_json::to_vec(value)
            .map_err(|e| BuildError::Plugin(format!("JSON encode error: {}", e)))
    }

    /// Encode a Value to a pretty JSON string.
    pub fn encode_string(&self, value: &Value) -> Result<String, BuildError> {
        serde_json::to_string_pretty(value)
            .map_err(|e| BuildError::Plugin(format!("JSON encode error: {}", e)))
    }

    /// Encode a Value to a compact JSON string.
    pub fn encode_string_compact(&self, value: &Value) -> Result<String, BuildError> {
        serde_json::to_string(value)
            .map_err(|e| BuildError::Plugin(format!("JSON encode error: {}", e)))
    }
}

impl Default for JsonApi {
    fn default() -> Self { Self::new() }
}
```

## `sandbox/log_api.rs`

```rust
use tracing::{debug, error, info, warn};

/// Logging API for plugins.
pub struct LogApi {
    plugin_name: String,
}

impl LogApi {
    pub fn new() -> Self {
        Self { plugin_name: String::from("unknown") }
    }

    pub fn with_plugin_name(name: String) -> Self {
        Self { plugin_name: name }
    }

    pub fn debug(&self, message: &str) {
        debug!(plugin = %self.plugin_name, "{}", message);
    }

    pub fn info(&self, message: &str) {
        info!(plugin = %self.plugin_name, "{}", message);
    }

    pub fn warn(&self, message: &str) {
        warn!(plugin = %self.plugin_name, "{}", message);
    }

    pub fn error(&self, message: &str) {
        error!(plugin = %self.plugin_name, "{}", message);
    }
}

impl Default for LogApi {
    fn default() -> Self { Self::new() }
}
```

## Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_path_traversal_blocked() {
        let source = TempDir::new().unwrap();
        let output = TempDir::new().unwrap();
        let api = FileApi::new(source.path().to_path_buf(), output.path().to_path_buf());

        // Should fail
        assert!(api.validate_source_path("../etc/passwd").is_err());
        assert!(api.validate_source_path("foo/../../bar").is_err());
        assert!(api.validate_source_path("/etc/passwd").is_err());

        // Should succeed
        assert!(api.validate_source_path("foo/bar.txt").is_ok());
    }

    #[test]
    fn test_xxhash3() {
        let api = HashApi::new();
        assert_eq!(api.xxhash3(b"hello"), api.xxhash3(b"hello"));
        assert_ne!(api.xxhash3(b"hello"), api.xxhash3(b"world"));
    }

    #[test]
    fn test_sha256() {
        let api = HashApi::new();
        assert_eq!(
            api.sha256(b"hello"),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn test_json_roundtrip() {
        let api = JsonApi::new();
        let original = r#"{"key": "value", "num": 42}"#;
        let value = api.decode_str(original).unwrap();
        let encoded = api.encode_string_compact(&value).unwrap();
        assert_eq!(encoded, r#"{"key":"value","num":42}"#);
    }
}
```

## Dependencies

```toml
# Add to crates/rpp/Cargo.toml
sha2 = "0.10"
md5 = "0.7"
```

## Verification

```bash
cargo check -p rpp
cargo test -p rpp sandbox
```
