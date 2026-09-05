//! HTTP static server and SSE live-reload endpoint for `rpp dev`.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt as _;
use tower_http::services::ServeDir;

use crate::ui;

use super::pack::PackStore;

#[derive(Clone)]
pub(super) struct ServerState {
    reloads: broadcast::Sender<String>,
    packs: PackStore,
}

async fn download(State(state): State<ServerState>, Path(file): Path<String>) -> Response {
    match state.packs.current() {
        Some(pack) if file == format!("{}.zip", pack.sha1) => (
            [
                (header::CONTENT_TYPE, "application/zip"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            pack.bytes,
        )
            .into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The SSE endpoint: streams reload events to the connected client.
pub async fn sse_handler(
    State(state): State<ServerState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let rx = state.reloads.subscribe();
    let initial = tokio_stream::once(Ok(Event::default().data(
        serde_json::json!({"type": "reload", "changed": [], "pack": state.packs.metadata()})
            .to_string(),
    )));
    let stream = BroadcastStream::new(rx).filter_map(move |msg| match msg {
        Ok(data) => {
            let mut payload: serde_json::Value = serde_json::from_str(&data).unwrap();
            if payload["type"] == "reload" {
                // Queued notifications may predate the connection snapshot.
                payload["pack"] = state.packs.metadata();
            }
            Some(Ok(Event::default().data(payload.to_string())))
        }
        Err(_) => Some(Ok(Event::default().data(
            serde_json::json!({"type": "reload", "changed": [], "pack": state.packs.metadata()})
                .to_string(),
        ))),
    });
    Sse::new(initial.chain(stream)).keep_alive(KeepAlive::default())
}

/// Bind and serve the dev HTTP server until Ctrl-C.
pub async fn serve_http(
    output_dir: PathBuf,
    reload_tx: broadcast::Sender<String>,
    packs: PackStore,
    host: &str,
    port: u16,
    open_browser: bool,
) -> Result<()> {
    let app = Router::new()
        .route("/events", get(sse_handler))
        .route("/packs/:file", get(download))
        .fallback_service(ServeDir::new(output_dir))
        .with_state(ServerState {
            reloads: reload_tx,
            packs,
        });

    let addr: SocketAddr = format!("{host}:{port}")
        .parse()
        .with_context(|| format!("invalid dev address {host}:{port}"))?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;

    let url = format!("http://{addr}/");
    ui::success(format!("dev server on {url}"));
    ui::detail("watching for changes (Ctrl-C to stop)");
    if open_browser {
        let _ = open::that(&url);
    }

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .context("serving")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use sha1::{Digest, Sha1};

    #[tokio::test]
    async fn download_matches_metadata_and_old_urls_never_serve_new_bytes() {
        let source = tempfile::tempdir().unwrap();
        let file = source.path().join("pack.mcmeta");
        std::fs::write(&file, "{}").unwrap();
        let packs = PackStore::default();
        packs.publish(source.path()).unwrap();
        let original = packs.current().unwrap();
        let (reloads, _) = broadcast::channel(2);
        let state = ServerState { reloads, packs };
        let response = download(State(state.clone()), Path(format!("{}.zip", original.sha1))).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/zip");
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(format!("{:x}", Sha1::digest(&bytes)), original.sha1);
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        assert!(zip.by_name("pack.mcmeta").is_ok());

        std::fs::write(file, "changed").unwrap();
        state.packs.publish(source.path()).unwrap();
        let response = download(State(state), Path(format!("{}.zip", original.sha1))).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn sse_starts_with_current_pack_without_waiting_for_an_edit() {
        let source = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("pack.mcmeta"), "{}").unwrap();
        let packs = PackStore::default();
        packs.publish(source.path()).unwrap();
        let metadata = packs.metadata();
        let (reloads, _) = broadcast::channel(2);
        let response = sse_handler(State(ServerState {
            reloads: reloads.clone(),
            packs: packs.clone(),
        }))
        .await
        .into_response();
        let mut stream = response.into_body().into_data_stream();
        let bytes = stream.next().await.unwrap().unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        let event: serde_json::Value =
            serde_json::from_str(text.trim().strip_prefix("data: ").unwrap()).unwrap();
        assert_eq!(event["pack"], metadata);

        std::fs::write(source.path().join("pack.mcmeta"), "new pack").unwrap();
        packs.publish(source.path()).unwrap();
        for _ in 0..3 {
            reloads
                .send(
                    serde_json::json!({
                        "type": "reload", "changed": ["pack.mcmeta"], "pack": metadata
                    })
                    .to_string(),
                )
                .unwrap();
        }
        // Capacity is two: recover from lag, then consume older queued notifications.
        for _ in 0..3 {
            let bytes = stream.next().await.unwrap().unwrap();
            let text = std::str::from_utf8(&bytes).unwrap();
            let event: serde_json::Value =
                serde_json::from_str(text.trim().strip_prefix("data: ").unwrap()).unwrap();
            assert_eq!(event["pack"], packs.metadata());
        }
    }
}
