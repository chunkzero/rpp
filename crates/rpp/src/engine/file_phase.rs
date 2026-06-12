//! Per-file discovery, cache lookup, and worker processing.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::cache::{FileEntry, Fingerprint, Manifest, ObjectStore, OutputRef};
use crate::error::{Error, Result};

use super::cache_replay::materialize_file_entry;
use super::discovery::SourceFile;
use super::keys::{chain_for, chain_key, ChainStep, CompiledProcessor};
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
    pub(crate) source_owners: &'a mut BTreeMap<String, String>,
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
        source_owners,
        factories,
    } = ctx;
    let mut processed = 0usize;
    let mut cached = 0usize;
    let mut dropped = 0usize;
    let mut dirty: Vec<(SourceFile, Arc<Vec<ChainStep>>, u64)> = Vec::new();

    for src in sources {
        let chain = chain_for(compiled, &src.rel);
        let ck = chain_key(&chain);

        let prev_entry = prev.and_then(|m| m.files.get(&src.rel));
        let clean = match prev_entry {
            Some(entry) => {
                entry.chain_key == ck
                    && (src.matches_fast(&entry.fingerprint) || {
                        match src.fingerprint() {
                            Ok((fp, _)) => fp.xxh3 == entry.fingerprint.xxh3,
                            Err(_) => false,
                        }
                    })
            }
            None => false,
        };

        if clean {
            let entry = prev_entry.expect("clean implies prev entry").clone();
            if materialize_file_entry(store, &entry, output, source_owners, &src.rel)? {
                if entry.outputs.is_empty() {
                    dropped += 1;
                }
                cached += 1;
                new_manifest.files.insert(src.rel.clone(), entry);
                continue;
            }
        }

        dirty.push((src, Arc::new(chain), ck));
    }

    if !dirty.is_empty() {
        let pool = WorkerPool::new(engine.worker_count(), Arc::clone(factories));
        let mut pending: BTreeMap<String, (u64, Fingerprint)> = BTreeMap::new();
        let mut submitted = 0usize;

        for (src, chain, ck) in &dirty {
            let (fp, contents) = src.fingerprint()?;
            pending.insert(src.rel.clone(), (*ck, fp));
            pool.submit(Job {
                rel: src.rel.clone(),
                file: crate::model::PackFile::new(src.rel.clone(), contents),
                chain: Arc::clone(chain),
            })?;
            submitted += 1;
        }

        let mut outcomes = Vec::with_capacity(submitted);
        for _ in 0..submitted {
            let outcome = match pool.recv() {
                Some(r) => r?,
                None => return Err(Error::Build("worker pool closed early".into())),
            };
            outcomes.push(outcome);
        }
        outcomes.sort_by(|a, b| outcome_rel(a).cmp(outcome_rel(b)));

        for outcome in outcomes {
            match outcome {
                JobOutcome::Produced { rel, file } => {
                    let (ck, fp) = pending
                        .get(&rel)
                        .cloned()
                        .ok_or_else(|| Error::Build(format!("unknown result for {rel}")))?;
                    let object = store.put(&file.contents)?;
                    super::cache_replay::claim_source_output(source_owners, &file.path, &rel)?;
                    output.files.insert(
                        file.path.clone(),
                        super::generator::OutputContent::from_bytes(file.contents),
                    );
                    new_manifest.files.insert(
                        rel,
                        FileEntry {
                            fingerprint: fp,
                            chain_key: ck,
                            outputs: vec![OutputRef {
                                path: file.path,
                                object,
                            }],
                        },
                    );
                    processed += 1;
                }
                JobOutcome::Dropped { rel } => {
                    let (ck, fp) = pending
                        .get(&rel)
                        .cloned()
                        .ok_or_else(|| Error::Build(format!("unknown result for {rel}")))?;
                    new_manifest.files.insert(
                        rel,
                        FileEntry {
                            fingerprint: fp,
                            chain_key: ck,
                            outputs: Vec::new(),
                        },
                    );
                    processed += 1;
                    dropped += 1;
                }
            }
        }

        pool.shutdown();
    }

    Ok(FilePhaseStats {
        processed,
        cached,
        dropped,
    })
}

fn outcome_rel(outcome: &JobOutcome) -> &str {
    match outcome {
        JobOutcome::Produced { rel, .. } | JobOutcome::Dropped { rel } => rel,
    }
}
