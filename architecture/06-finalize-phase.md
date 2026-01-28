# Commit 6: Finalize Phase and Output

**Goal**: Run generators and write output files.

## Files to Create

```
crates/rpp/src/
├── build/
│   ├── finalize.rs
│   └── output.rs
```

## `build/finalize.rs`

```rust
use std::sync::Arc;

use crate::build::{BuildError, BuildResult, GeneratedFile};
use crate::plugin::{GeneratorContext, PluginRegistry};

use super::output::OutputWriter;
use super::process::ProcessPhaseResult;

/// Finalize phase: runs generators and writes output.
pub struct FinalizePhase {
    output_dir: std::path::PathBuf,
}

impl FinalizePhase {
    pub fn new(output_dir: std::path::PathBuf) -> Self {
        Self { output_dir }
    }

    /// Run finalization: generators, then write all files to output.
    pub fn run(
        &self,
        process_result: ProcessPhaseResult,
        registry: &Arc<PluginRegistry>,
        config: &toml::Value,
    ) -> Result<BuildResult, BuildError> {
        let start = std::time::Instant::now();

        let writer = OutputWriter::new(self.output_dir.clone());

        // Run generators sequentially (they may have interdependencies)
        let generator_ctx = GeneratorContext {
            processed_files: &process_result.processed,
            config,
            output_dir: &self.output_dir,
        };

        let mut generated_files = Vec::new();
        for generator in registry.generators() {
            let files = generator.generate(&generator_ctx)?;
            generated_files.extend(files);
        }

        // Write processed files
        for file in &process_result.processed {
            writer.write_processed(file)?;
        }

        // Write generated files
        for file in &generated_files {
            writer.write_generated(file)?;
        }

        // Write cached files (from cache, not source)
        for cached in &process_result.cached {
            writer.write_cached(cached)?;
        }

        Ok(BuildResult {
            files_processed: process_result.processed.len(),
            files_cached: process_result.cached.len(),
            files_generated: generated_files.len(),
            files_cancelled: process_result.cancelled_count,
            duration: start.elapsed(),
        })
    }
}
```

## `build/output.rs`

```rust
use std::fs;
use std::path::{Path, PathBuf};

use crate::build::{BuildError, CachedEntry, GeneratedFile, ProcessedFile};

/// Writes files to the output directory.
pub struct OutputWriter {
    output_dir: PathBuf,
}

impl OutputWriter {
    pub fn new(output_dir: PathBuf) -> Self {
        Self { output_dir }
    }

    /// Write a processed file to output.
    pub fn write_processed(&self, file: &ProcessedFile) -> Result<(), BuildError> {
        let output_path = self.output_dir.join(&file.output_path);
        self.write_file(&output_path, &file.content)
    }

    /// Write a generated file to output.
    pub fn write_generated(&self, file: &GeneratedFile) -> Result<(), BuildError> {
        let output_path = self.output_dir.join(&file.path);
        self.write_file(&output_path, &file.content)
    }

    /// Write a cached file to output.
    pub fn write_cached(&self, cached: &CachedEntry) -> Result<(), BuildError> {
        let output_path = self.output_dir.join(&cached.output_path);
        self.write_file(&output_path, &cached.content)
    }

    fn write_file(&self, path: &Path, content: &[u8]) -> Result<(), BuildError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| BuildError::FileWrite {
                path: path.to_path_buf(),
                source: e,
            })?;
        }

        fs::write(path, content).map_err(|e| BuildError::FileWrite {
            path: path.to_path_buf(),
            source: e,
        })
    }

    /// Clean the output directory.
    pub fn clean(&self) -> Result<(), BuildError> {
        if self.output_dir.exists() {
            fs::remove_dir_all(&self.output_dir).map_err(|e| BuildError::FileWrite {
                path: self.output_dir.clone(),
                source: e,
            })?;
        }
        Ok(())
    }
}
```

## Finalize Flow

```
┌──────────────────────────────────────────────────────────────┐
│                      FinalizePhase                            │
│                                                               │
│  ┌─────────────────────────────────────────────────────────┐ │
│  │ 1. Run Generators (Sequential)                          │ │
│  │                                                         │ │
│  │    GeneratorContext {                                   │ │
│  │      processed_files: &[ProcessedFile],                 │ │
│  │      config: &toml::Value,                              │ │
│  │      output_dir: &Path,                                 │ │
│  │    }                                                    │ │
│  │                                                         │ │
│  │    for generator in registry.generators():              │ │
│  │        generated_files.extend(generator.generate(ctx))  │ │
│  └─────────────────────────────────────────────────────────┘ │
│                           │                                   │
│                           ▼                                   │
│  ┌─────────────────────────────────────────────────────────┐ │
│  │ 2. Write Processed Files                                │ │
│  │                                                         │ │
│  │    for file in processed:                               │ │
│  │        write(output_dir / file.output_path, file.content)│
│  └─────────────────────────────────────────────────────────┘ │
│                           │                                   │
│                           ▼                                   │
│  ┌─────────────────────────────────────────────────────────┐ │
│  │ 3. Write Generated Files                                │ │
│  │                                                         │ │
│  │    for file in generated:                               │ │
│  │        write(output_dir / file.path, file.content)      │ │
│  └─────────────────────────────────────────────────────────┘ │
│                           │                                   │
│                           ▼                                   │
│  ┌─────────────────────────────────────────────────────────┐ │
│  │ 4. Write Cached Files                                   │ │
│  │                                                         │ │
│  │    for cached in cached_entries:                        │ │
│  │        write(output_dir / output_path, cached.content) │ │
│  └─────────────────────────────────────────────────────────┘ │
│                           │                                   │
│                           ▼                                   │
│  ┌─────────────────────────────────────────────────────────┐ │
│  │ 5. Return BuildResult                                   │ │
│  │                                                         │ │
│  │    BuildResult {                                        │ │
│  │      files_processed,                                   │ │
│  │      files_cached,                                      │ │
│  │      files_generated,                                   │ │
│  │      files_cancelled,                                   │ │
│  │      duration,                                          │ │
│  │    }                                                    │ │
│  └─────────────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────────┘
```

## Update `build/mod.rs`

Add exports:

```rust
mod finalize;
mod output;

pub use finalize::FinalizePhase;
pub use output::OutputWriter;
```

## Verification

```bash
cargo check -p rpp
cargo test -p rpp finalize
cargo test -p rpp output
```
