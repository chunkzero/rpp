use std::sync::Arc;

use crate::build::{BuildError, BuildResult};
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

        // Create zip archive
        let zip_path = self.output_dir.join("pack.zip");
        writer.create_zip_archive(&zip_path)?;

        Ok(BuildResult {
            files_processed: process_result.processed.len(),
            files_cached: process_result.cached.len(),
            files_generated: generated_files.len(),
            files_cancelled: process_result.cancelled_count,
            duration: start.elapsed(),
        })
    }
}
