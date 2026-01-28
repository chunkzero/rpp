use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use viz::Router;

use rpp::build::BuildEngine;

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
        let output_dir = self.config.output_dir.clone();
        let broadcaster_clone = Arc::clone(&broadcaster);

        let app = Router::new()
            .get("/events", move |_| {
                let bc = Arc::clone(&broadcaster_clone);
                async move { sse::sse_handler(bc).await }
            })
            .get("/*path", move |req| {
                let dir = output_dir.clone();
                async move { serve_static(req, dir).await }
            });

        // Start HTTP server
        let addr = format!("{}:{}", self.config.host, self.config.port);
        tracing::info!("Starting dev server at http://{}", addr);

        let server = tokio::spawn(async move {
            let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
            viz::serve(listener, app).await
        });

        // Watch for file changes
        while let Some(event) = watch_rx.recv().await {
            match event {
                WatchEvent::SourceChanged(path) => {
                    tracing::info!("File changed: {}", path.display());

                    match self.build_engine.build() {
                        Ok(result) => {
                            tracing::info!("Rebuilt {} files", result.files_processed);
                            broadcaster
                                .broadcast(sse::ReloadEvent::FileChanged { paths: vec![path] })
                                .await;
                        }
                        Err(e) => {
                            tracing::error!("Build failed: {}", e);
                        }
                    }
                }
                WatchEvent::PluginChanged(path) => {
                    if self.config.hot_reload {
                        tracing::info!("Plugin changed: {}", path.display());
                        broadcaster.broadcast(sse::ReloadEvent::FullReload).await;
                    }
                }
                WatchEvent::ConfigChanged => {
                    tracing::info!("Config changed, full rebuild required");
                    broadcaster.broadcast(sse::ReloadEvent::FullReload).await;
                }
            }
        }

        server.await??;
        Ok(())
    }
}

async fn serve_static(req: viz::Request, output_dir: PathBuf) -> viz::Result<viz::Response> {
    use http_body_util::Full;
    use viz::{IntoResponse, RequestExt};

    let path: String = req.param("path").unwrap_or_default();
    let file_path = output_dir.join(path.trim_start_matches('/'));

    if file_path.is_file() {
        let content = tokio::fs::read(&file_path)
            .await
            .map_err(|e| viz::Error::boxed(e))?;

        let mime = mime_guess::from_path(&file_path).first_or_octet_stream();

        let body = viz::Body::Full(Full::new(bytes::Bytes::from(content)));

        Ok(viz::Response::builder()
            .header("Content-Type", mime.as_ref())
            .body(body)?)
    } else {
        Err(viz::StatusCode::NOT_FOUND.into_error())
    }
}
