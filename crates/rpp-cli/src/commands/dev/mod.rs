//! `rpp dev`: watch sources + plugins, incrementally rebuild, serve the output
//! statically, and push live-reload events over SSE.

mod pack;
mod rebuild;
mod server;
mod watch;

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::broadcast;

use crate::luals;
use crate::project::Project;
use crate::ui;

use rebuild::{rebuild_loop, DevSession};
use server::serve_http;
use watch::{local_plugin_dirs, spawn_watcher};

/// Run the dev server (blocks until Ctrl-C).
pub fn run(dir: &Path) -> Result<()> {
    let project = Project::discover(dir)?;
    ui::intro("Start dev server");
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("starting the async runtime")?;
    rt.block_on(serve(project))
}

async fn serve(project: Project) -> Result<()> {
    let root = project.root.clone();
    let source_dir = project.source_dir();
    let output_dir = project.output_dir();
    let config_path = project.config_path();
    let host = project.config.dev.host.clone();
    let port = project.config.dev.port;
    let open_browser = project.config.dev.open;
    let plugin_dirs = local_plugin_dirs(&project);

    let _ = luals::write_if_stale(&project.root.join(".rpp").join("api"));

    // Watch before the initial build so edits made during it are not lost;
    // they queue in the channel until the rebuild loop starts.
    let (fs_tx, fs_rx) = tokio::sync::mpsc::unbounded_channel();
    let watcher = spawn_watcher(&root, &source_dir, &config_path, plugin_dirs, fs_tx)?;
    let packs = pack::PackStore::default();
    let session = Arc::new(tokio::sync::Mutex::new(DevSession::new(
        project,
        watcher,
        packs.clone(),
    )));
    let (reload_tx, _) = broadcast::channel::<String>(64);

    {
        let session = Arc::clone(&session);
        tokio::task::spawn_blocking(move || {
            let mut guard = session.blocking_lock();
            guard.initial_build()
        })
        .await
        .context("join initial build")??;
    }

    {
        let session = Arc::clone(&session);
        let reload_tx = reload_tx.clone();
        tokio::spawn(rebuild_loop(session, fs_rx, reload_tx));
    }

    serve_http(output_dir, reload_tx, packs, &host, port, open_browser).await
}

#[cfg(test)]
mod tests {
    use super::watch::{classify_path, ChangeKind};
    use std::path::PathBuf;

    #[test]
    fn classify_kinds() {
        let src = PathBuf::from("/proj/src");
        let cfg = PathBuf::from("/proj/rpp.toml");
        let plugins = vec![PathBuf::from("/proj/plugins/hello")];

        assert_eq!(
            classify_path(&PathBuf::from("/proj/rpp.toml"), &src, &cfg, &plugins),
            Some(ChangeKind::Config)
        );
        assert_eq!(
            classify_path(
                &PathBuf::from("/proj/plugins/hello/init.lua"),
                &src,
                &cfg,
                &plugins
            ),
            Some(ChangeKind::Plugin)
        );
        assert_eq!(
            classify_path(
                &PathBuf::from("/proj/src/assets/x.json"),
                &src,
                &cfg,
                &plugins
            ),
            Some(ChangeKind::Source)
        );
        assert_eq!(
            classify_path(&PathBuf::from("/proj/other/x"), &src, &cfg, &plugins),
            None
        );
    }
}
