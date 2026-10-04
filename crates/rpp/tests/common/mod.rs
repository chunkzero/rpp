//! Shared test helpers for building projects on disk.

#![allow(dead_code)]

pub mod engine;
#[cfg(feature = "js")]
pub mod js;
pub mod mock;

use std::path::{Path, PathBuf};

use tempfile::TempDir;

/// A scratch project (src dir) for end-to-end engine tests.
pub struct Project {
    pub dir: TempDir,
}

impl Project {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        Project { dir }
    }

    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    pub fn src(&self) -> PathBuf {
        self.dir.path().join("src")
    }

    /// Write a source file (relative to `src/`).
    pub fn write_src(&self, rel: &str, contents: &str) {
        let path = self.src().join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    /// Read an output file (relative to `dist/`).
    pub fn read_out(&self, rel: &str) -> Option<String> {
        std::fs::read_to_string(self.dir.path().join("dist").join(rel)).ok()
    }

    /// Whether an output file exists.
    pub fn out_exists(&self, rel: &str) -> bool {
        self.dir.path().join("dist").join(rel).exists()
    }

    /// A default config with `pack.name = "test-pack"`.
    pub fn config(&self) -> rpp::config::Config {
        rpp::config::Config::new("test-pack", 34)
    }
}
