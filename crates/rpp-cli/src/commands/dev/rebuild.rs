//! Incremental rebuild loop and dev-session state for `rpp dev`.

use std::sync::Arc;

use anyhow::{Context, Result};
use rpp::engine::Engine;
use rpp_wasm::WasmEngine;
use tokio::sync::{broadcast, mpsc};

use crate::project::Project;
use crate::ui;

use super::watch::{local_plugin_dirs, ChangeBatch};

/// Dev-server state reused across incremental rebuilds.
pub struct DevSession {
    project: Project,
    engine: Option<Engine>,
    wasm_engine: Option<WasmEngine>,
}

impl DevSession {
    pub fn new(project: Project) -> Self {
        Self {
            project,
            engine: None,
            wasm_engine: None,
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
        ui::success(format!(
            "initial build: {} processed, {} cached, {} generated",
            built.processed, built.cached, built.generated
        ));
        if let Some(overrides) = self.project.plugin_options_summary() {
            ui::detail(format!("plugin options: {overrides}"));
        }
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
            let mut reloaded =
                Project::discover(&self.project.root).context("reloading rpp.toml")?;
            reloaded.plugin_options = self.project.plugin_options.clone();
            let topology_changed = self.project.source_dir() != reloaded.source_dir()
                || self.project.output_dir() != reloaded.output_dir()
                || self.project.config.dev.host != reloaded.config.dev.host
                || self.project.config.dev.port != reloaded.config.dev.port
                || local_plugin_dirs(&self.project) != local_plugin_dirs(&reloaded);
            if topology_changed {
                anyhow::bail!(
                    "source, output, dev address, or local plugin paths changed; restart `rpp dev`"
                );
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

        reload_payload_if_changed(&changed)
    }
}

/// Build the JSON SSE payload `{"type":"reload","changed":[...]}` when needed.
pub fn reload_payload(changed: &[String]) -> String {
    serde_json::json!({ "type": "reload", "changed": changed }).to_string()
}

/// Return an SSE payload only when at least one output file changed.
pub fn reload_payload_if_changed(changed: &[String]) -> Result<Option<String>> {
    if changed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(reload_payload(changed)))
    }
}

/// The rebuild loop: drains change batches, rebuilds, and broadcasts reloads.
pub async fn rebuild_loop(
    session: Arc<tokio::sync::Mutex<DevSession>>,
    mut fs_rx: mpsc::UnboundedReceiver<ChangeBatch>,
    reload_tx: broadcast::Sender<String>,
) {
    while let Some(batch) = fs_rx.recv().await {
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
            Ok(Err(e)) => ui::warn(format!("rebuild failed: {e:#}")),
            Err(e) => ui::warn(format!("rebuild task panicked: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_shape() {
        let p = reload_payload(&["a.json".to_string(), "b.png".to_string()]);
        let v: serde_json::Value = serde_json::from_str(&p).unwrap();
        assert_eq!(v["type"], "reload");
        assert_eq!(v["changed"][0], "a.json");
        assert_eq!(v["changed"][1], "b.png");
    }

    #[test]
    fn reload_suppressed_when_unchanged() {
        assert!(reload_payload_if_changed(&[]).unwrap().is_none());
    }

    #[test]
    fn reload_emitted_when_changed() {
        let payload = reload_payload_if_changed(&["x.json".to_string()])
            .unwrap()
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(v["changed"], serde_json::json!(["x.json"]));
    }
}
