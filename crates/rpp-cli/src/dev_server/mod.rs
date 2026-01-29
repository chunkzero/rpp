use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use axum::{
    Router,
    routing::get,
};
use tower_http::services::ServeDir;

use rpp::build::BuildEngine;
use rpp::worker::pool;

mod sse;
mod watcher;

use sse::SseBroadcaster;
use watcher::{FileWatcher, WatchEvent};

pub struct DevServer {
    build_engine: BuildEngine,
    config: ServerConfig,
}

pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub source_dir: PathBuf,
    pub output_dir: PathBuf,
    pub plugin_dir: Option<PathBuf>,
    pub hot_reload: bool,
}

impl DevServer {
    pub fn new(build_engine: BuildEngine, config: ServerConfig) -> Self {
        Self {
            build_engine,
            config,
        }
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        // Initial build
        tracing::info!("Running initial build...");
        let result = self.build_engine.build()?;
        tracing::info!(
            "Built {} files ({} cached, {} generated)",
            result.files_processed,
            result.files_cached,
            result.files_generated
        );

        // Set up file watcher
        let (watch_tx, mut watch_rx) = mpsc::unbounded_channel();
        let mut watcher = FileWatcher::new(watch_tx)?;
        watcher.watch(&self.config.source_dir)?;
        if let Some(ref plugin_dir) = self.config.plugin_dir {
            watcher.watch(plugin_dir)?;
        }

        // Set up SSE broadcaster
        let broadcaster = Arc::new(SseBroadcaster::new());

        // Set up HTTP routes
        let app = Router::new()
            .route("/events", get(sse::sse_handler))
            .nest_service("/", ServeDir::new(&self.config.output_dir))
            .with_state(Arc::clone(&broadcaster));

        // Start HTTP server
        let addr = format!("{}:{}", self.config.host, self.config.port);
        tracing::info!("Starting dev server at http://{}", addr);

        let listener = tokio::net::TcpListener::bind(&addr).await?;
        tokio::spawn(async move {
            axum::serve(listener, app).await
        });

        // Watch for file changes with debouncing
        const DEBOUNCE_MS: u64 = 100;
        let mut pending_changes: HashSet<PathBuf> = HashSet::new();
        let mut needs_full_reload = false;

        loop {
            tokio::select! {
                Some(event) = watch_rx.recv() => {
                    match event {
                        WatchEvent::SourceChanged(path) => {
                            tracing::debug!("File changed: {}", path.display());
                            pending_changes.insert(path);
                        }
                        WatchEvent::PluginChanged(path) => {
                            if self.config.hot_reload {
                                tracing::info!("Plugin changed: {}", path.display());

                                // Invalidate all worker runtime caches
                                pool::invalidate_lua_runtimes();

                                needs_full_reload = true;
                            }
                        }
                        WatchEvent::ConfigChanged => {
                            tracing::debug!("Config changed");
                            needs_full_reload = true;
                        }
                    }

                    // Start debounce timer after receiving any event
                    tokio::time::sleep(Duration::from_millis(DEBOUNCE_MS)).await;

                    // Drain any additional events that arrived during debounce
                    while let Ok(event) = watch_rx.try_recv() {
                        match event {
                            WatchEvent::SourceChanged(path) => {
                                pending_changes.insert(path);
                            }
                            WatchEvent::PluginChanged(_) | WatchEvent::ConfigChanged => {
                                needs_full_reload = true;
                            }
                        }
                    }

                    // Perform rebuild with coalesced changes
                    if needs_full_reload {
                        tracing::info!("Config or plugin changed, triggering full reload");
                        broadcaster.broadcast(sse::ReloadEvent::FullReload).await;
                        needs_full_reload = false;
                        pending_changes.clear();
                    } else if !pending_changes.is_empty() {
                        let paths: Vec<_> = pending_changes.drain().collect();
                        tracing::info!("Rebuilding {} changed file(s)", paths.len());

                        match self.build_engine.build() {
                            Ok(result) => {
                                tracing::info!("Rebuilt {} files", result.files_processed);
                                broadcaster
                                    .broadcast(sse::ReloadEvent::FileChanged { paths })
                                    .await;
                            }
                            Err(e) => {
                                tracing::error!("Build failed: {}", e);
                            }
                        }
                    }
                }
            }
        }
    }
}
