# Commit 9: Dev Server

**Goal**: HTTP server with file watching and hot reload.

## Files to Create

```
crates/rpp-cli/src/
├── dev_server/
│   ├── mod.rs
│   ├── watcher.rs
│   └── sse.rs
```

## `dev_server/mod.rs`

```rust
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use viz::Router;

use rpp::build::BuildEngine;

mod watcher;
mod sse;

use watcher::{FileWatcher, WatchEvent};
use sse::SseBroadcaster;

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
        Self { build_engine, config }
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
                            broadcaster.broadcast(sse::ReloadEvent::FileChanged {
                                paths: vec![path],
                            }).await;
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

async fn serve_static(
    req: viz::Request,
    output_dir: PathBuf,
) -> viz::Result<viz::Response> {
    use viz::types::PathInfo;

    let path: PathInfo = req.extract().await?;
    let file_path = output_dir.join(path.as_str().trim_start_matches('/'));

    if file_path.is_file() {
        let content = tokio::fs::read(&file_path).await
            .map_err(|e| viz::Error::Io(e))?;

        let mime = mime_guess::from_path(&file_path)
            .first_or_octet_stream();

        Ok(viz::Response::builder()
            .header("Content-Type", mime.as_ref())
            .body(content.into())?)
    } else {
        Err(viz::StatusCode::NOT_FOUND.into_error())
    }
}
```

## `dev_server/watcher.rs`

```rust
use std::path::PathBuf;
use notify::{Watcher, RecommendedWatcher, RecursiveMode, Event, EventKind};
use tokio::sync::mpsc::UnboundedSender;

pub enum WatchEvent {
    SourceChanged(PathBuf),
    PluginChanged(PathBuf),
    ConfigChanged,
}

pub struct FileWatcher {
    watcher: RecommendedWatcher,
    _source_dirs: Vec<PathBuf>,
}

impl FileWatcher {
    pub fn new(tx: UnboundedSender<WatchEvent>) -> anyhow::Result<Self> {
        let watcher = notify::recommended_watcher(move |res: Result<Event, _>| {
            if let Ok(event) = res {
                if let Some(path) = event.paths.first() {
                    let watch_event = if path.extension().map(|e| e == "lua").unwrap_or(false) {
                        WatchEvent::PluginChanged(path.clone())
                    } else if path.file_name().map(|n| n == "rpp.toml").unwrap_or(false) {
                        WatchEvent::ConfigChanged
                    } else {
                        match event.kind {
                            EventKind::Create(_) |
                            EventKind::Modify(_) |
                            EventKind::Remove(_) => {
                                WatchEvent::SourceChanged(path.clone())
                            }
                            _ => return,
                        }
                    };
                    let _ = tx.send(watch_event);
                }
            }
        })?;

        Ok(Self {
            watcher,
            _source_dirs: Vec::new(),
        })
    }

    pub fn watch(&mut self, path: &PathBuf) -> anyhow::Result<()> {
        self.watcher.watch(path, RecursiveMode::Recursive)?;
        Ok(())
    }
}
```

## `dev_server/sse.rs`

```rust
use std::sync::Arc;
use tokio::sync::broadcast;
use viz::{Response, Result};

pub enum ReloadEvent {
    FullReload,
    FileChanged { paths: Vec<std::path::PathBuf> },
}

pub struct SseBroadcaster {
    tx: broadcast::Sender<String>,
}

impl SseBroadcaster {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(16);
        Self { tx }
    }

    pub async fn broadcast(&self, event: ReloadEvent) {
        let msg = match event {
            ReloadEvent::FullReload => "event: reload\ndata: full\n\n".to_string(),
            ReloadEvent::FileChanged { paths } => {
                let paths_str: Vec<_> = paths.iter()
                    .map(|p| p.to_string_lossy().to_string())
                    .collect();
                format!("event: reload\ndata: {}\n\n", paths_str.join(","))
            }
        };
        let _ = self.tx.send(msg);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }
}

pub async fn sse_handler(broadcaster: Arc<SseBroadcaster>) -> Result<Response> {
    use futures_util::stream::StreamExt;
    use tokio_stream::wrappers::BroadcastStream;

    let rx = broadcaster.subscribe();
    let stream = BroadcastStream::new(rx)
        .filter_map(|msg| async { msg.ok() });

    Ok(Response::builder()
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .body(viz::Body::from_stream(stream))?)
}
```

## Dev Server Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│                         Dev Server                                │
│                                                                   │
│  ┌──────────────┐                                                │
│  │ File Watcher │◀── Watches: source files, plugins, config      │
│  │   (notify)   │                                                │
│  └──────┬───────┘                                                │
│         │                                                         │
│         │ WatchEvent (tokio::mpsc)                                │
│         ▼                                                         │
│  ┌──────────────┐                                                │
│  │  Event       │── SourceChanged → incremental rebuild          │
│  │  Handler     │── PluginChanged → reload plugin + rebuild      │
│  │              │── ConfigChanged → full rebuild                 │
│  └──────┬───────┘                                                │
│         │                                                         │
│         │ Rebuild                                                 │
│         ▼                                                         │
│  ┌──────────────┐                                                │
│  │   Build      │── Uses cache for unchanged files               │
│  │   Engine     │                                                │
│  └──────┬───────┘                                                │
│         │                                                         │
│         │ ReloadEvent                                             │
│         ▼                                                         │
│  ┌──────────────┐                                                │
│  │ SSE Channel  │── Broadcast to all connected browsers          │
│  │              │   event: reload                                │
│  │              │   data: <changed files>                        │
│  └──────────────┘                                                │
│                                                                   │
│  ┌──────────────┐                                                │
│  │ HTTP Server  │── GET /events    → SSE stream                  │
│  │    (viz)     │── GET /*path     → static files from output    │
│  └──────────────┘                                                │
│                                                                   │
└──────────────────────────────────────────────────────────────────┘
```

## Client-Side JavaScript

```javascript
// Include in your HTML for hot reload
const evtSource = new EventSource('/events');

evtSource.addEventListener('reload', (event) => {
    if (event.data === 'full') {
        window.location.reload();
    } else {
        // Partial reload - could reload specific assets
        console.log('Changed:', event.data);
        window.location.reload();
    }
});
```

## Dependencies

```toml
# Add to crates/rpp-cli/Cargo.toml
mime_guess = "2.0"
```

## Verification

```bash
cargo check -p rpp-cli
cargo run -p rpp-cli -- serve --source ./example_pack
# Open browser to http://localhost:8080
# Modify a source file
# Browser should auto-reload
```
