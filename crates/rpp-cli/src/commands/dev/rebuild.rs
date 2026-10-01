//! Incremental rebuild loop and dev-session state for `rpp dev`.

use std::path::Path;
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
    discover: fn(&Path) -> Result<Project>,
    runtime: Runtime,
    wasm_engine: Option<WasmEngine>,
    watcher: DevWatcher,
    packs: PackStore,
}

enum Runtime {
    NeedsReload { config: bool },
    Ready(Box<Engine>),
}

impl DevSession {
    pub fn new(project: Project, watcher: DevWatcher, packs: PackStore) -> Self {
        Self {
            project,
            discover: Project::discover,
            runtime: Runtime::NeedsReload { config: false },
            wasm_engine: None,
            watcher,
            packs,
        }
    }

    /// Run the initial build and retain the engine + wasm runtime.
    pub fn initial_build(&mut self) -> Result<()> {
        self.rebuild_engine()?;
        let built = self.engine()?.build().context("initial build")?;
        self.packs.publish(&self.project.output_dir())?;
        tracing::info!(
            processed = built.processed,
            cached = built.cached,
            generated = built.generated,
            "Initial build complete"
        );
        Ok(())
    }

    fn engine(&self) -> Result<&Engine> {
        match &self.runtime {
            Runtime::Ready(engine) => Ok(engine),
            Runtime::NeedsReload { .. } => anyhow::bail!("engine needs reload"),
        }
    }

    /// Retry pending reloads without making the previous engine available.
    fn rebuild_engine(&mut self) -> Result<()> {
        let Runtime::NeedsReload { config } = self.runtime else {
            return Ok(());
        };
        let reloaded = if config {
            let reloaded =
                (self.discover)(&self.project.root).context("reloading the project config")?;
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
            // Watch replacement plugins before loading so repairing a broken
            // plugin triggers another attempt. Failure keeps the reload pending.
            self.watcher
                .set_plugin_dirs(local_plugin_dirs(&reloaded))
                .context("updating watched plugin directories")?;
            self.watcher
                .set_config_files(reloaded.config_files())
                .context("updating watched config files")?;
            if self.project.config.build.wasm.memory_limit_mb
                != reloaded.config.build.wasm.memory_limit_mb
                || self.project.config.build.wasm.execution_deadline_seconds
                    != reloaded.config.build.wasm.execution_deadline_seconds
            {
                self.wasm_engine = None;
            }
            Some(reloaded)
        } else {
            None
        };
        let project = reloaded.as_ref().unwrap_or(&self.project);
        let (engine, wasm) = project.build_engine_with_wasm(self.wasm_engine.take())?;
        if let Some(project) = reloaded {
            self.project = project;
        }
        self.wasm_engine = wasm;
        self.runtime = Runtime::Ready(Box::new(engine));
        Ok(())
    }

    /// Whether a changed source path is a plugin authoring input, which the engine only
    /// reads when its plugins are loaded.
    fn touches_authoring_source(&self, batch: &ChangeBatch) -> bool {
        let Runtime::Ready(engine) = &self.runtime else {
            return false;
        };
        let source_dir = self.project.source_dir();
        batch.paths.iter().any(|path| {
            path.strip_prefix(&source_dir).is_ok_and(|rel| {
                let rel = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                engine.is_authoring_source(&rel)
            })
        })
    }

    /// Perform one rebuild for a change batch. Returns the SSE payload when
    /// output files changed or archive publication recovered, otherwise `None`.
    pub fn rebuild_once(&mut self, batch: &ChangeBatch) -> Result<Option<String>> {
        let started = std::time::Instant::now();

        if batch.needs_engine_rebuild() || self.touches_authoring_source(batch) {
            let config =
                batch.kind_config || matches!(self.runtime, Runtime::NeedsReload { config: true });
            self.runtime = Runtime::NeedsReload { config };
        }
        self.rebuild_engine()?;
        let result = self.engine()?.build().context("incremental rebuild")?;

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

        tracing::info!(
            kind,
            elapsed = %ui::fmt_duration(started.elapsed()),
            changed = changed.len(),
            "Rebuilt resource pack"
        );

        // Retry publication even on no-op builds: a previous archive failure may
        // have left the engine cache ahead of the last published pack.
        let updated = self.packs.publish(&self.project.output_dir())?;
        if changed.is_empty() && !updated {
            return Ok(None);
        }
        let mut payload = if changed.is_empty() {
            serde_json::json!({ "type": "pack" })
        } else {
            serde_json::json!({ "type": "reload", "changed": changed })
        };
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
                tracing::warn!("Rebuild failed: {e:#}");
                let _ = reload_tx.send(
                    serde_json::json!({
                        "type": "build_error", "message": format!("{e:#}")
                    })
                    .to_string(),
                );
            }
            Err(e) => tracing::warn!("Rebuild task panicked: {e}"),
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
        session.discover = Project::discover_isolated;
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

        std::fs::write(&file, "{\"recovered\":true}").unwrap();
        session.engine().unwrap().build().unwrap();
        let recovered = session
            .rebuild_once(&ChangeBatch::default())
            .unwrap()
            .unwrap();
        let recovered: serde_json::Value = serde_json::from_str(&recovered).unwrap();
        assert_eq!(recovered["type"], "pack");
        assert!(recovered.get("changed").is_none());
        assert_eq!(recovered["pack"], packs.metadata());

        std::fs::write(&config, "invalid toml").unwrap();
        assert!(session
            .rebuild_once(&ChangeBatch {
                kind_config: true,
                ..Default::default()
            })
            .is_err());
        assert_eq!(packs.metadata(), recovered["pack"]);
    }

    fn plugin_config(name: &str) -> String {
        format!("[pack]\nname = 'test'\n[[plugin]]\nsource = 'path:{name}'\n")
    }

    fn write_plugin(root: &std::path::Path, name: &str, contents: &str) {
        let plugin = root.join(name);
        std::fs::create_dir_all(&plugin).unwrap();
        std::fs::write(
            plugin.join("plugin.toml"),
            format!("[plugin]\nid = '{name}'\nversion = '1.0.0'\n"),
        )
        .unwrap();
        std::fs::write(plugin.join("init.lua"), contents).unwrap();
    }

    fn generator(contents: &str) -> String {
        format!(
            "local p = require('rpp').plugin()\n\
             p:generator('value', function(ctx) ctx:emit('value.txt', '{contents}') end)\n\
             return p"
        )
    }

    fn plugin_session(root: &std::path::Path) -> DevSession {
        let source = root.join("src");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("pack.mcmeta"), "{}").unwrap();
        std::fs::write(root.join("rpp.toml"), plugin_config("original")).unwrap();
        write_plugin(root, "original", &generator("original"));
        let project = Project::discover_isolated(root).unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let watcher = spawn_watcher(
            root,
            &source,
            &root.join("rpp.toml"),
            local_plugin_dirs(&project),
            tx,
        )
        .unwrap();
        let mut session = DevSession::new(project, watcher, PackStore::default());
        session.discover = Project::discover_isolated;
        session.initial_build().unwrap();
        session
    }

    #[test]
    fn failed_plugin_reload_retries_on_source_events_and_recovers() {
        let root = tempfile::tempdir().unwrap();
        let mut session = plugin_session(root.path());
        let original_pack = session.packs.metadata();
        write_plugin(root.path(), "original", "error('broken plugin')");
        assert!(session
            .rebuild_once(&ChangeBatch {
                kind_plugin: true,
                ..Default::default()
            })
            .is_err());
        std::fs::write(root.path().join("src/new.txt"), "new").unwrap();
        assert!(session.rebuild_once(&ChangeBatch::default()).is_err());
        assert_eq!(session.packs.metadata(), original_pack);
        assert!(!session.project.output_dir().join("new.txt").exists());

        write_plugin(root.path(), "original", &generator("repaired"));
        assert!(session
            .rebuild_once(&ChangeBatch::default())
            .unwrap()
            .is_some());
        assert_eq!(
            std::fs::read_to_string(session.project.output_dir().join("value.txt")).unwrap(),
            "repaired"
        );
        assert_ne!(session.packs.metadata(), original_pack);
    }

    #[test]
    fn failed_config_reload_stays_pending_until_replacement_plugin_recovers() {
        let root = tempfile::tempdir().unwrap();
        let mut session = plugin_session(root.path());
        let original_pack = session.packs.metadata();
        let config = root.path().join("rpp.toml");
        let config_batch = ChangeBatch {
            kind_config: true,
            ..Default::default()
        };

        std::fs::write(&config, "invalid toml").unwrap();
        assert!(session.rebuild_once(&config_batch).is_err());
        std::fs::write(root.path().join("src/new.txt"), "new").unwrap();
        assert!(session.rebuild_once(&ChangeBatch::default()).is_err());
        assert_eq!(session.packs.metadata(), original_pack);

        write_plugin(root.path(), "replacement", "error('broken replacement')");
        std::fs::write(&config, plugin_config("replacement")).unwrap();
        assert!(session.rebuild_once(&ChangeBatch::default()).is_err());
        assert!(session.rebuild_once(&ChangeBatch::default()).is_err());
        assert_eq!(session.packs.metadata(), original_pack);
        assert!(!session.project.output_dir().join("new.txt").exists());

        write_plugin(root.path(), "replacement", &generator("replacement"));
        assert!(session
            .rebuild_once(&ChangeBatch {
                kind_plugin: true,
                ..Default::default()
            })
            .unwrap()
            .is_some());
        assert_eq!(
            std::fs::read_to_string(session.project.output_dir().join("value.txt")).unwrap(),
            "replacement"
        );
        assert_ne!(session.packs.metadata(), original_pack);
    }
}
