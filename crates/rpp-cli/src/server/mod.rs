mod sse;

use anyhow::Result;
use futures_util::StreamExt;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use viz::types::Event;
use viz::{
    get,
    header::ACCEPT,
    serve,
    types::{Sse, State},
    HandlerExt, IntoResponse, Request, RequestExt, Response, ResponseExt, Router, StatusCode,
};

#[derive(Debug, Clone)]
struct AppState {
    file_change_tx: broadcast::Sender<String>,
}

pub fn serve_cli() -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_time()
        .enable_io()
        .build()?;

    rt.block_on(serve_async())
}

async fn serve_async() -> Result<()> {
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    let listener = TcpListener::bind(addr).await?;
    tracing::info!("Serving viz server");

    let (file_change_tx, _) = broadcast::channel::<String>(16);

    let app = Router::new().route("/", get(index)).route(
        "/sse",
        get(sse.with(State::new(AppState {
            file_change_tx: file_change_tx.clone(),
        }))),
    );

    tokio::select! {
        _ = serve(listener, app) => {}
        _ = file_change_tx.closed() => {}
    }

    Ok(())
}

async fn index(_: Request) -> Result<Response, viz::Error> {
    Ok(Response::html::<&'static str>("idk"))
}

async fn sse(req: Request) -> Result<impl IntoResponse, viz::Error> {
    // check request `Accept` header
    if !matches!(req.header::<_, String>(ACCEPT), Some(ts) if ts == mime::TEXT_EVENT_STREAM.as_ref())
    {
        Err(StatusCode::BAD_REQUEST.into_error())?;
    }

    let state = req
        .state::<AppState>()
        .ok_or_else(|| StatusCode::INTERNAL_SERVER_ERROR.into_error())?;

    let file_change_rx = state.file_change_tx.subscribe();

    let file_changes = BroadcastStream::new(file_change_rx);

    let stream = async_stream::stream! {
        yield Event::default().data("handshake");

        // Forward file change events as SSE
        let mut stream = file_changes;
        while let Some(Ok(file_path)) = stream.next().await {
            yield Event::default().data(
                format!("event: fileChange\ndata: {}\n\n", file_path)
            );
        }
    };

    Ok(Sse::new(stream))
}
