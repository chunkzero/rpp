//! Per-file discovery, cache lookup, and worker processing.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::cache::{FileEntry, Fingerprint, Manifest, ObjectStore, OutputRef};
use crate::error::{Error, Result};

use super::cache_replay::materialize_file_entry;
use super::discovery::SourceFile;
use super::keys::{chain_for, chain_key, CompiledProcessor};
use super::worker::{Job, JobOutcome, WorkerPool};
use super::Engine;

pub(crate) struct FilePhaseStats {
    pub(crate) processed: usize,
    pub(crate) cached: usize,
    pub(crate) dropped: usize,
}

pub(crate) struct FilePhaseCtx<'a> {
    pub(crate) engine: &'a Engine,
    pub(crate) compiled: &'a [CompiledProcessor],
    pub(crate) sources: Vec<SourceFile>,
    pub(crate) store: &'a ObjectStore,
    pub(crate) prev: Option<&'a Manifest>,
    pub(crate) new_manifest: &'a mut Manifest,
    pub(crate) output: &'a mut super::generator::OutputSet,
    pub(crate) factories: &'a Arc<Vec<Arc<dyn crate::model::PluginFactory>>>,
}

pub(crate) fn process_files(ctx: FilePhaseCtx<'_>) -> Result<FilePhaseStats> {
    let FilePhaseCtx {
        engine,
        compiled,
        sources,
        store,
        prev,
        new_manifest,
        output,
        factories,
    } = ctx;
    let mut processed = 0usize;
    let mut cached = 0usize;
    let mut dropped = 0usize;
    // At most one job per worker is outstanding. Receive and publish before
    // reading another source, including bytes read during cache validation.
    let limit = engine.worker_count();
    let mut pool_slot = engine.pool.lock();
    let mut pool = pool_slot.take();
    let mut pending = BTreeMap::new();
    let mut completed = BTreeMap::new();

    for src in sources {
        let chain = chain_for(compiled, &src.rel);
        let ck = chain_key(&chain);
        let chain_cacheable = chain.iter().all(|step| step.cacheable);

        let candidate = prev
            .and_then(|m| m.files.get(&src.rel))
            .filter(|entry| chain_cacheable && entry.chain_key == ck);
        let mut read = None;
        let clean = match candidate {
            Some(entry) => {
                let (fp, contents) = src.fingerprint()?;
                let same = fp.xxh3 == entry.fingerprint.xxh3;
                read = Some((fp, contents));
                same
            }
            None => false,
        };

        if clean {
            let entry = candidate.expect("clean implies prev entry").clone();
            if materialize_file_entry(store, &engine.output, &entry, output, &src.rel)? {
                if entry.outputs.is_empty() {
                    dropped += 1;
                }
                cached += 1;
                new_manifest.files.insert(src.rel.clone(), entry);
                continue;
            }
        }

        let pool = pool.get_or_insert_with(|| WorkerPool::new(limit, Arc::clone(factories)));
        let (fp, contents) = match read {
            Some(read) => read,
            None => src.fingerprint()?,
        };
        pending.insert(src.rel.clone(), (ck, fp));
        pool.submit(Job {
            rel: src.rel.clone(),
            file: crate::model::PackFile::new(src.rel, contents),
            chain: Arc::new(chain),
        })?;
        if pending.len() == limit {
            collect_result(pool, store, &mut pending, &mut completed)?;
        }
    }

    if let Some(pool) = pool.as_ref() {
        while !pending.is_empty() {
            collect_result(pool, store, &mut pending, &mut completed)?;
        }
    }
    // Cache hits have claimed their paths first; dirty results claim in source
    // order, independent of worker completion order.
    for (rel, entry) in completed {
        for out in &entry.outputs {
            super::cache_replay::claim_source_output(&mut output.owners, &out.path, &rel)?;
            output.files.insert(
                out.path.clone(),
                super::generator::OutputContent::Object(out.object),
            );
        }
        processed += 1;
        dropped += usize::from(entry.outputs.is_empty());
        new_manifest.files.insert(rel, entry);
    }
    *pool_slot = pool;

    Ok(FilePhaseStats {
        processed,
        cached,
        dropped,
    })
}

fn collect_result(
    pool: &WorkerPool,
    store: &ObjectStore,
    pending: &mut BTreeMap<String, (u64, Fingerprint)>,
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

    #[test]
    fn submission_read_error_discards_pending_results() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("src");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("a.txt"), "old").unwrap();
        std::fs::write(source.join("b.txt"), "b").unwrap();
        let sources = super::super::discovery::discover(&source).unwrap();
        std::fs::remove_file(source.join("b.txt")).unwrap();
        let config = crate::config::Config::parse(
            r#"[pack]
name = "test"
"#,
            "rpp.toml",
        )
        .unwrap();
        let engine = Engine::builder(config)
            .project_root(dir.path())
            .build_engine()
            .unwrap();
        let store = ObjectStore::open(dir.path().join(".rpp/cache/objects")).unwrap();
        let mut manifest = Manifest::empty(0);
        let mut output = super::super::generator::OutputSet::default();
        let result = process_files(FilePhaseCtx {
            engine: &engine,
            compiled: &engine.compiled,
            sources,
            store: &store,
            prev: None,
            new_manifest: &mut manifest,
            output: &mut output,
            factories: &engine.factories,
        });
        assert!(result.is_err());
        assert!(engine.pool.lock().is_none());

        std::fs::write(source.join("a.txt"), "new").unwrap();
        engine.build().unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("dist/a.txt")).unwrap(),
            "new"
        );
    }
}
