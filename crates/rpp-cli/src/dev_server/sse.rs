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
                let paths_str: Vec<_> = paths
                    .iter()
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
    use tokio_stream::wrappers::BroadcastStream;

    let rx = broadcaster.subscribe();
    let stream = BroadcastStream::new(rx);

    Ok(Response::builder()
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .body(viz::Body::from_stream(stream))?)
}
