use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio_stream::{Stream, StreamExt};
use tokio_stream::wrappers::BroadcastStream;
use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
};

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
            ReloadEvent::FullReload => "full".to_string(),
            ReloadEvent::FileChanged { paths } => {
                let paths_str: Vec<_> = paths
                    .iter()
                    .map(|p| p.to_string_lossy().to_string())
                    .collect();
                paths_str.join(",")
            }
        };
        let _ = self.tx.send(msg);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }
}

pub async fn sse_handler(
    State(broadcaster): State<Arc<SseBroadcaster>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = broadcaster.subscribe();
    let stream = BroadcastStream::new(rx)
        .filter_map(|msg| match msg {
            Ok(text) => Some(Ok(Event::default().event("reload").data(text))),
            Err(_) => None,
        });

    Sse::new(stream).keep_alive(KeepAlive::default())
}
