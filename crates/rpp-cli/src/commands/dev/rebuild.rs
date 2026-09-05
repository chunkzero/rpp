//! Incremental rebuild loop and dev-session state for `rpp dev`.

use std::sync::Arc;

use anyhow::{Context, Result};
use rpp::engine::Engine;
use rpp_wasm::WasmEngine;
use tokio::sync::{broadcast, mpsc};

use crate::project::Project;
use crate::ui;

use super::pack::PackStore;
use super::watch::{local_plugin_dirs, ChangeBatch, DevWatcher};

/// Dev-server state reused across incremental rebuilds.
pub struct DevSession {
    project: Project,
    engine: Option<Engine>,
    wasm_engine: Option<WasmEngine>,
    watcher: DevWatcher,
    packs: PackStore,
}

impl DevSession {
    pub fn new(project: Project, watcher: DevWatcher, packs: PackStore) -> Self {
        Self {
            project,
            engine: None,
            wasm_engine: None,
            watcher,
            packs,
        }
    }

    /// Run the initial build and retain the engine + wasm runtime.
    pub fn initial_build(&mut self) -> Result<()> {
        let (engine, wasm) = self
            .project
            .build_engine_with_wasm(self.wasm_engine.take())?;
        self.wasm_engine = wasm;
        self.engine = Some(engine);
        let built = self
            .engine
            .as_ref()
            .unwrap()
            .build()
            .context("initial build")?;
        self.packs.publish(&self.project.output_dir())?;
        ui::success(format!(
            "initial build: {} processed, {} cached, {} generated",
            built.processed, built.cached, built.generated
        ));
        Ok(())
    }

    /// Rebuild the engine from scratch, reusing the wasm runtime when present.
    fn rebuild_engine(&mut self) -> Result<()> {
        let (engine, wasm) = self
            .project
            .build_engine_with_wasm(self.wasm_engine.take())?;
        self.wasm_engine = wasm;
        self.engine = Some(engine);
        Ok(())
    }

    /// Perform one rebuild for a change batch. Returns the SSE payload when
    /// output files changed, or `None` to suppress a reload event.
    pub fn rebuild_once(&mut self, batch: &ChangeBatch) -> Result<Option<String>> {
        let started = std::time::Instant::now();

        if batch.kind_config {
            let reloaded = Project::discover(&self.project.root).context("reloading rpp.toml")?;
            // The served directory, watched source tree, and listening address
            // are fixed for the session; everything else reloads in place.
            let fixed_changed = self.project.source_dir() != reloaded.source_dir()
                || self.project.output_dir() != reloaded.output_dir()
                || self.project.config.dev.host != reloaded.config.dev.host
                || self.project.config.dev.port != reloaded.config.dev.port;
            if fixed_changed {
                anyhow::bail!(
                    "`build.source`, `build.output`, or `[dev]` changed; restart `rpp dev` to apply"
                );
            }
            self.watcher
                .set_plugin_dirs(local_plugin_dirs(&reloaded))
                .context("updating watched plugin directories")?;
            if self.project.config.build.wasm.memory_limit_mb
                != reloaded.config.build.wasm.memory_limit_mb
                || self.project.config.build.wasm.execution_deadline_seconds
                    != reloaded.config.build.wasm.execution_deadline_seconds
            {
                self.wasm_engine = None;
            }
            self.project = reloaded;
        }

        if batch.needs_engine_rebuild() || self.engine.is_none() {
            self.rebuild_engine()?;
        }

        let engine = self.engine.as_ref().context("engine not initialized")?;
        let result = engine.build().context("incremental rebuild")?;

        let mut changed: Vec<String> = result.changes.written.clone();
        changed.extend(result.changes.removed.iter().cloned());
        changed.sort();
        changed.dedup();

        let kind = if batch.kind_config {
            "config"
        } else if batch.kind_plugin {
            "plugin"
        } else {
            "source"
        };

        ui::detail(format!(
            "rebuilt ({kind}) in {}: {} changed file{}",
            ui::fmt_duration(started.elapsed()),
            changed.len(),
            if changed.len() == 1 { "" } else { "s" }
        ));

        let updated = self.packs.publish(&self.project.output_dir())?;
        if changed.is_empty() && !updated {
            return Ok(None);
        }
        let mut payload = serde_json::json!({ "type": "reload", "changed": changed });
        if updated {
            payload["pack"] = self.packs.metadata();
        }
        Ok(Some(payload.to_string()))
    }
}

/// The rebuild loop: drains change batches, rebuilds, and broadcasts reloads.
pub async fn rebuild_loop(
    session: Arc<tokio::sync::Mutex<DevSession>>,
    mut fs_rx: mpsc::UnboundedReceiver<ChangeBatch>,
    reload_tx: broadcast::Sender<String>,
) {
    while let Some(mut batch) = fs_rx.recv().await {
        // Changes that arrived during the previous rebuild fold into one pass.
        while let Ok(more) = fs_rx.try_recv() {
            batch.merge(more);
        }
        let session = Arc::clone(&session);
        let result = tokio::task::spawn_blocking(move || {
            let mut guard = session.blocking_lock();
            guard.rebuild_once(&batch)
        })
        .await;

        match result {
            Ok(Ok(Some(payload))) => {
                let _ = reload_tx.send(payload);
            }
            Ok(Ok(None)) => {}
            Ok(Err(e)) => {
                ui::warn(format!("rebuild failed: {e:#}"));
                let _ = reload_tx.send(
                    serde_json::json!({
                        "type": "build_error", "message": format!("{e:#}")
                    })
                    .to_string(),
                );
            }
            Err(e) => ui::warn(format!("rebuild task panicked: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::watch::spawn_watcher;
    use super::*;

    #[test]
    fn rebuild_publishes_only_successful_pack_changes() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("src");
        std::fs::create_dir(&source).unwrap();
        let config = root.path().join("rpp.toml");
        std::fs::write(&config, "[pack]\nname = 'test'\n").unwrap();
        let file = source.join("pack.mcmeta");
        std::fs::write(&file, "{}").unwrap();
        let project = Project::discover_isolated(root.path()).unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let watcher = spawn_watcher(root.path(), &source, &config, vec![], tx).unwrap();
        let packs = PackStore::default();
        let mut session = DevSession::new(project, watcher, packs.clone());
        session.initial_build().unwrap();
        let original = packs.metadata();

        assert!(session
            .rebuild_once(&ChangeBatch::default())
            .unwrap()
            .is_none());
        std::fs::write(&file, "{\"changed\":true}").unwrap();
        let event = session
            .rebuild_once(&ChangeBatch::default())
            .unwrap()
            .unwrap();
        let event: serde_json::Value = serde_json::from_str(&event).unwrap();
        assert_eq!(event["type"], "reload");
        assert_eq!(event["changed"], serde_json::json!(["pack.mcmeta"]));
        assert_eq!(event["pack"], packs.metadata());
        assert_ne!(event["pack"], original);

        std::fs::write(&config, "invalid toml").unwrap();
        assert!(session
            .rebuild_once(&ChangeBatch {
                kind_config: true,
                ..Default::default()
            })
            .is_err());
        assert_eq!(packs.metadata(), event["pack"]);
    }
}
