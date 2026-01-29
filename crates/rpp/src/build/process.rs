use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::build::{BuildError, CachedEntry, FileEntry, FileIndex, ProcessedFile};
use crate::plugin::PluginRegistry;
use crate::worker::{ProcessingJob, ProcessingResult, WorkerPool};

/// Process phase: runs processors on all files in parallel.
pub struct ProcessPhase {
    num_workers: usize,
}

impl ProcessPhase {
    pub fn new(num_workers: usize) -> Self {
        Self { num_workers }
    }

    /// Process all files in the index using the worker pool.
    pub fn run(
        &self,
        index: FileIndex,
        registry: &Arc<PluginRegistry>,
        config: &toml::Value,
    ) -> Result<ProcessPhaseResult, BuildError> {
        let pool = WorkerPool::new(self.num_workers);

        let mut submitted = 0;
        // Track FileEntry by source_path so we can update cache later
        let mut file_entries: HashMap<PathBuf, FileEntry> = HashMap::new();

        // Submit all jobs
        for file in index.entries {
            let processors = registry.processors_for_file(&file.relative_path);
            let source_path = file.source_path.clone();
            // Store a clone of the FileEntry for cache updates
            file_entries.insert(source_path, file.clone());
            let job = ProcessingJob {
                file,
                processors,
                config: config.clone(),
            };
            pool.submit(job)?;
            submitted += 1;
        }

        // Collect results
        let mut processed = Vec::new();
        let mut cancelled = Vec::new();
        let mut errors = Vec::new();

        // Only collect results if we submitted jobs
        if submitted > 0 {
            let mut received = 0;
            for result in pool.results() {
                received += 1;

                match result {
                    ProcessingResult::Processed(file) => processed.push(file),
                    ProcessingResult::Cancelled { path } => cancelled.push(path),
                    ProcessingResult::Error { path, error } => {
                        errors.push((path, error));
                    }
                }

                if received >= submitted {
                    break;
                }
            }
        }

        pool.shutdown();

        // Report first error with path context
        if !errors.is_empty() {
            let (path, error) = errors.into_iter().next().unwrap();
            return Err(BuildError::ProcessingFailed {
                path,
                source: Box::new(error),
            });
        }

        Ok(ProcessPhaseResult {
            processed,
            cancelled_count: cancelled.len(),
            cached: index.cached,
            file_entries,
        })
    }
}

/// Result of the process phase.
pub struct ProcessPhaseResult {
    pub processed: Vec<ProcessedFile>,
    pub cancelled_count: usize,
    pub cached: Vec<CachedEntry>,
    /// Map of source_path -> FileEntry for cache updates
    pub file_entries: HashMap<PathBuf, FileEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{FileEntry, Fingerprint};
    use crate::plugin::{Plugin, ProcessResult, ProcessingContext, ProcessorPlugin};
    use std::path::PathBuf;

    struct TestProcessor {
        name: String,
        patterns: Vec<String>,
    }

    impl Plugin for TestProcessor {
        fn name(&self) -> &str {
            &self.name
        }

        fn version(&self) -> &str {
            "1.0.0"
        }
    }

    impl ProcessorPlugin for TestProcessor {
        fn patterns(&self) -> &[String] {
            &self.patterns
        }

        fn process(&self, ctx: &ProcessingContext) -> Result<ProcessResult, BuildError> {
            let mut new_content = ctx.content.to_vec();
            new_content.extend_from_slice(b"-processed");
            Ok(ProcessResult::continue_with(new_content))
        }
    }

    #[test]
    fn test_process_phase_basic() {
        let mut registry = PluginRegistry::new();
        registry.register_processor(Arc::new(TestProcessor {
            name: "test".into(),
            patterns: vec!["*.txt".into()],
        }));
        let registry = Arc::new(registry);

        let file = FileEntry {
            source_path: PathBuf::from("/test/file.txt"),
            relative_path: PathBuf::from("file.txt"),
            fingerprint: Fingerprint {
                mtime: 0,
                size: 5,
                hash: 0,
            },
            content: b"hello".to_vec(),
        };

        let index = FileIndex {
            entries: vec![file],
            cached: vec![],
        };

        let config = toml::Value::Table(toml::map::Map::new());
        let phase = ProcessPhase::new(2);
        let result = phase.run(index, &registry, &config).unwrap();

        assert_eq!(result.processed.len(), 1);
        assert_eq!(result.processed[0].content, b"hello-processed");
        assert_eq!(result.cancelled_count, 0);
        assert_eq!(result.file_entries.len(), 1);
    }

    #[test]
    fn test_process_phase_multiple_files() {
        let mut registry = PluginRegistry::new();
        registry.register_processor(Arc::new(TestProcessor {
            name: "test".into(),
            patterns: vec!["*.txt".into()],
        }));
        let registry = Arc::new(registry);

        let files: Vec<FileEntry> = (0..5)
            .map(|i| FileEntry {
                source_path: PathBuf::from(format!("/test/file{}.txt", i)),
                relative_path: PathBuf::from(format!("file{}.txt", i)),
                fingerprint: Fingerprint {
                    mtime: 0,
                    size: 5,
                    hash: 0,
                },
                content: format!("test{}", i).into_bytes(),
            })
            .collect();

        let index = FileIndex {
            entries: files,
            cached: vec![],
        };

        let config = toml::Value::Table(toml::map::Map::new());
        let phase = ProcessPhase::new(2);
        let result = phase.run(index, &registry, &config).unwrap();

        assert_eq!(result.processed.len(), 5);
        assert_eq!(result.cancelled_count, 0);
        assert_eq!(result.file_entries.len(), 5);
    }

    #[test]
    fn test_process_phase_with_cache() {
        let registry = Arc::new(PluginRegistry::new());

        let cached = vec![CachedEntry {
            source_path: PathBuf::from("/test/cached.txt"),
            output_path: PathBuf::from("cached.txt"),
            content: b"cached".to_vec(),
        }];

        let index = FileIndex {
            entries: vec![],
            cached: cached.clone(),
        };

        let config = toml::Value::Table(toml::map::Map::new());
        let phase = ProcessPhase::new(2);
        let result = phase.run(index, &registry, &config).unwrap();

        assert_eq!(result.processed.len(), 0);
        assert_eq!(result.cached.len(), 1);
        assert_eq!(result.cached[0].content, b"cached");
        assert_eq!(result.file_entries.len(), 0);
    }
}
