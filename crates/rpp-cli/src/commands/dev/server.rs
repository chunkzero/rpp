//! HTTP static server and SSE live-reload endpoint for `rpp dev`.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use axum::Router;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt as _;
use tower_http::services::ServeDir;

use crate::ui;

/// The SSE endpoint: streams reload events to the connected client.
pub async fn sse_handler(
    State(tx): State<broadcast::Sender<String>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let rx = tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|msg| match msg {
        Ok(data) => Some(Ok(Event::default().data(data))),
        Err(_) => None,
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// Bind and serve the dev HTTP server until Ctrl-C.
pub async fn serve_http(
    output_dir: PathBuf,
    reload_tx: broadcast::Sender<String>,
    host: &str,
    port: u16,
    open_browser: bool,
) -> Result<()> {
    let app = Router::new()
        .route("/events", get(sse_handler))
        .fallback_service(ServeDir::new(output_dir))
        .with_state(reload_tx);

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
