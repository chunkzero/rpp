//! State shared by the phases of one build.

use crate::cache::{Manifest, ObjectStore};
use crate::error::Result;
use crate::model::BuildStats;
use crate::source::SourceFile;

use super::external::PublicationPlan;
use super::output::{OutputContent, OutputSet};
use super::result::ChangeReport;
use super::{boundary, output_sync, Engine};

/// One build: the previous manifest it replays from and the output and manifest it produces.
///
/// The file phase (`process_files`) runs before generators (`run_generators`); `finish`
/// publishes the result.
pub(super) struct BuildSession<'a> {
    pub(super) engine: &'a Engine,
    pub(super) store: ObjectStore,
    /// The previous manifest, if its global key still matches.
    pub(super) prev: Option<&'a Manifest>,
    pub(super) manifest: Manifest,
    pub(super) output: OutputSet,
    /// Relative paths of every pack source, visible to generators.
    pub(super) source_files: Vec<String>,
    pub(super) stats: BuildStats,
}

impl<'a> BuildSession<'a> {
    pub(super) fn new(
        engine: &'a Engine,
        store: ObjectStore,
        prev: Option<&'a Manifest>,
        manifest: Manifest,
        sources: &[SourceFile],
    ) -> Self {
        Self {
            engine,
            store,
            prev,
            manifest,
            output: OutputSet::default(),
            source_files: sources.iter().map(|source| source.rel.clone()).collect(),
            stats: BuildStats::default(),
        }
    }

    /// Add the `pack.mcmeta` generated from the config to the output.
    pub(super) fn insert_pack_metadata(&mut self) -> Result<()> {
        let object = self.store.put(&self.engine.config.pack.mcmeta())?;
        self.output
            .insert_config("pack.mcmeta", OutputContent::Object(object));
        Ok(())
    }

    /// Publish external outputs and the pack output, then persist the manifest and drop
    /// unreferenced CAS objects.
    pub(super) fn finish(self) -> Result<ChangeReport> {
        let engine = self.engine;
        let (config, root) = (&engine.config, engine.project_root.as_path());
        boundary::validate_destinations(config, root, &engine.factories)?;
        let external = PublicationPlan::prepare(config, root, &self.manifest, &self.store)?;
        external.record_recovery(root)?;
        let mut changes = output_sync::sync_output(
            config,
            &engine.output,
            &self.output,
            &self.store,
            engine.worker_count(),
        )?;
        changes.external = external.publish(root)?;

        self.manifest.save(&engine.manifest_path())?;
        let mut live = self.manifest.live_objects();
        live.extend(self.output.files().values().map(|content| match content {
            OutputContent::Object(key) | OutputContent::Linked { key, .. } => *key,
        }));
        self.store.gc(&live)?;
        Ok(changes)
    }
}
