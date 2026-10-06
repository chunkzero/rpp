//! Per-file cache lookup and worker processing.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::cache::FileEntry;
use crate::error::{Error, Result};
use crate::source::SourceFile;

use super::keys::{chain_for, chain_key};
use super::output::OutputContent;
use super::session::BuildSession;
use super::worker::{Job, JobOutcome, WorkerContext, WorkerPool};

/// Per-file results gathered from the pool, keyed by source path so they apply in source order.
#[derive(Default)]
struct Collected {
    /// Chain keys of submitted files without a result yet.
    pending: BTreeMap<String, u64>,
    hits: BTreeMap<String, (FileEntry, Vec<OutputContent>)>,
    processed: BTreeMap<String, FileEntry>,
    /// The failure of the first source, in source order, among those that failed.
    error: Option<(String, Error)>,
}

impl BuildSession<'_> {
    /// Serve each source from the cache or run its processor chain on the worker pool.
    ///
    /// Cache hits claim their output paths first, then processed results, each in source
    /// order, independent of worker completion order.
    pub(super) fn process_files(&mut self, sources: Vec<SourceFile>) -> Result<()> {
        let engine = self.engine;
        // At most one job per worker is outstanding, bounding the source and output
        // bytes held for validation and processing.
        let limit = engine.worker_count();
        let mut pool_slot = engine.pool.lock();
        let pool = pool_slot.take().unwrap_or_else(|| {
            let context = WorkerContext {
                factories: Arc::clone(&engine.factories),
                store: self.store.clone(),
                output_dir: engine.output.clone(),
            };
            WorkerPool::new(limit, Arc::new(context))
        });
        let mut collected = Collected::default();

        for source in sources {
            if collected.error.is_some() {
                break;
            }
            let chain = chain_for(&engine.compiled, &source.rel);
            let key = chain_key(&chain);
            let cacheable = chain.iter().all(|step| step.cacheable);
            let cached = self
                .prev
                .and_then(|manifest| manifest.files.get(&source.rel))
                .filter(|entry| cacheable && entry.chain_key == key)
                .cloned();
            collected.pending.insert(source.rel.clone(), key);
            pool.submit(Job {
                source,
                chain,
                cached,
            })?;
            if collected.pending.len() == limit {
                collected.receive(&pool)?;
            }
        }
        while !collected.pending.is_empty() {
            collected.receive(&pool)?;
        }
        if let Some((_, error)) = collected.error {
            return Err(error);
        }

        for (rel, (entry, contents)) in collected.hits {
            self.apply_cached(rel, entry, contents)?;
        }
        self.record_processed(collected.processed)?;
        *pool_slot = Some(pool);
        Ok(())
    }

    /// Add processed results to the output.
    fn record_processed(&mut self, completed: BTreeMap<String, FileEntry>) -> Result<()> {
        for (rel, entry) in completed {
            for out in &entry.outputs {
                self.output
                    .insert_source(&rel, &out.path, OutputContent::Object(out.object))?;
            }
            self.stats.processed += 1;
            self.stats.dropped += usize::from(entry.outputs.is_empty());
            self.manifest.files.insert(rel, entry);
        }
        Ok(())
    }
}

impl Collected {
    /// Receive one result from `pool`.
    fn receive(&mut self, pool: &WorkerPool) -> Result<()> {
        let result = pool
            .recv()
            .ok_or_else(|| Error::Build("worker pool closed early".into()))?;
        let rel = result.rel;
        let chain_key = self
            .pending
            .remove(&rel)
            .ok_or_else(|| Error::Build(format!("unknown result for {rel}")))?;
        match result.outcome {
            Ok(JobOutcome::Cached { entry, contents }) => {
                self.hits.insert(rel, (entry, contents));
            }
            Ok(JobOutcome::Processed {
                fingerprint,
                outputs,
            }) => {
                let entry = FileEntry {
                    fingerprint,
                    chain_key,
                    outputs,
                };
                self.processed.insert(rel, entry);
            }
            Err(error) => {
                if self.error.as_ref().is_none_or(|(first, _)| rel < *first) {
                    self.error = Some((rel, error));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::{Manifest, ObjectStore};
    use crate::engine::Engine;

    #[test]
    fn submission_read_error_discards_pending_results() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("src");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("a.txt"), "old").unwrap();
        std::fs::write(source.join("b.txt"), "b").unwrap();
        let sources = crate::source::discover(&source).unwrap();
        std::fs::remove_file(source.join("b.txt")).unwrap();
        let config = crate::config::Config::new("test", 34);
        let engine = Engine::builder(config)
            .project_root(dir.path())
            .build_engine()
            .unwrap();
        let store = ObjectStore::open(dir.path().join(".rpp/cache/objects")).unwrap();
        let mut session = BuildSession::new(&engine, store, None, Manifest::empty(0), &sources);
        assert!(session.process_files(sources).is_err());
        assert!(engine.pool.lock().is_none());

        std::fs::write(source.join("a.txt"), "new").unwrap();
        engine.build().unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("dist/a.txt")).unwrap(),
            "new"
        );
    }
}
