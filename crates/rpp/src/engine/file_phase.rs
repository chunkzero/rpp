//! Per-file cache lookup and worker processing.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::cache::{FileEntry, Fingerprint, ObjectStore, OutputRef};
use crate::error::{Error, Result};
use crate::model::PackFile;
use crate::source::SourceFile;

use super::keys::{chain_for, chain_key};
use super::output::OutputContent;
use super::session::BuildSession;
use super::worker::{Job, JobOutcome, WorkerPool};

/// Dirty files submitted to the pool, keyed by source path: their chain key and fingerprint.
type Pending = BTreeMap<String, (u64, Fingerprint)>;

impl BuildSession<'_> {
    /// Serve each source from the cache or run its processor chain on the worker pool.
    pub(super) fn process_files(&mut self, sources: Vec<SourceFile>) -> Result<()> {
        let engine = self.engine;
        // At most one job per worker is outstanding. Receive and publish before
        // reading another source, including bytes read during cache validation.
        let limit = engine.worker_count();
        let mut pool_slot = engine.pool.lock();
        let mut pool = pool_slot.take();
        let mut pending = Pending::new();
        let mut completed = BTreeMap::new();

        for src in sources {
            let chain = chain_for(&engine.compiled, &src.rel);
            let key = chain_key(&chain);
            let cacheable = chain.iter().all(|step| step.cacheable);
            let Some((fingerprint, contents)) = self.replay_or_read(&src, key, cacheable)? else {
                continue;
            };
            let pool =
                pool.get_or_insert_with(|| WorkerPool::new(limit, Arc::clone(&engine.factories)));
            pending.insert(src.rel.clone(), (key, fingerprint));
            pool.submit(Job {
                file: PackFile::new(src.rel.clone(), contents),
                rel: src.rel,
                chain: Arc::new(chain),
            })?;
            if pending.len() == limit {
                collect_result(pool, &self.store, &mut pending, &mut completed)?;
            }
        }

        if let Some(pool) = pool.as_ref() {
            while !pending.is_empty() {
                collect_result(pool, &self.store, &mut pending, &mut completed)?;
            }
        }
        self.record_processed(completed)?;
        *pool_slot = pool;
        Ok(())
    }

    /// Serve `src` from the previous build when its chain and contents are unchanged.
    ///
    /// Returns the fingerprinted contents when the file must be processed instead.
    fn replay_or_read(
        &mut self,
        src: &SourceFile,
        chain_key: u64,
        cacheable: bool,
    ) -> Result<Option<(Fingerprint, Vec<u8>)>> {
        let candidate = self
            .prev
            .and_then(|manifest| manifest.files.get(&src.rel))
            .filter(|entry| cacheable && entry.chain_key == chain_key);
        let (fingerprint, contents) = src.fingerprint()?;
        if let Some(entry) = candidate {
            if fingerprint.xxh3 == entry.fingerprint.xxh3 && self.replay_file(&src.rel, entry)? {
                return Ok(None);
            }
        }
        Ok(Some((fingerprint, contents)))
    }

    /// Add processed results to the output. Cache hits have claimed their paths first; dirty
    /// results claim in source order, independent of worker completion order.
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

fn collect_result(
    pool: &WorkerPool,
    store: &ObjectStore,
    pending: &mut Pending,
    completed: &mut BTreeMap<String, FileEntry>,
) -> Result<()> {
    let outcome = pool
        .recv()
        .ok_or_else(|| Error::Build("worker pool closed early".into()))??;
    let (rel, outputs) = match outcome {
        JobOutcome::Produced { rel, file } => {
            let object = store.put(&file.contents)?;
            (
                rel,
                vec![OutputRef {
                    path: file.path,
                    object,
                }],
            )
        }
        JobOutcome::Dropped { rel } => (rel, Vec::new()),
    };
    let (chain_key, fingerprint) = pending
        .remove(&rel)
        .ok_or_else(|| Error::Build(format!("unknown result for {rel}")))?;
    completed.insert(
        rel,
        FileEntry {
            fingerprint,
            chain_key,
            outputs,
        },
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::Manifest;
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
        let config = crate::config::Config::new("test");
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
