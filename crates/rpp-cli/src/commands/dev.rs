//! `rpp dev`: watch sources + plugins, incrementally rebuild, serve the output
//! statically, and push live-reload events over SSE.
//!
//! Architecture:
//! - A blocking `notify` watcher (debounced) feeds change events into a tokio
//!   channel. Changes are classified into source / plugin / config buckets.
//! - The rebuild loop owns the [`Engine`] (rebuilt when plugins/config change)
//!   and runs `engine.build()` on `spawn_blocking`. No squash in dev.
//! - Each successful rebuild broadcasts a `{type:"reload", changed:[...]}` JSON
//!   payload to all connected `/events` SSE clients via a `tokio::broadcast`.
//! - axum serves the output dir statically and the SSE endpoint.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use axum::Router;
use notify_debouncer_full::notify::{RecursiveMode, Watcher};
use notify_debouncer_full::{new_debouncer, DebouncedEvent};
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt as _;
use tower_http::services::ServeDir;

use crate::project::Project;
use crate::ui;

/// What kind of change a filesystem event represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChangeKind {
    /// A source file under the build source dir.
    Source,
    /// A plugin file (rebuild the engine with reloaded factories).
    Plugin,
    /// `rpp.toml` itself (full project reload).
    Config,
}

/// A classified, deduplicated batch of changed paths.
#[derive(Debug, Default, Clone)]
struct ChangeBatch {
    kind_config: bool,
    kind_plugin: bool,
    paths: Vec<PathBuf>,
}

/// Run the dev server (blocks until Ctrl-C).
pub fn run(dir: &Path) -> Result<()> {
    let project = Project::discover(dir)?;
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

    // Initial build so the output dir exists before serving.
    let project = Arc::new(tokio::sync::Mutex::new(project));
    let (reload_tx, _) = broadcast::channel::<String>(64);

    {
        let project = Arc::clone(&project);
        let built = tokio::task::spawn_blocking(move || {
            let guard = project.blocking_lock();
            let engine = guard.build_engine()?;
            engine.build().context("initial build")
        })
        .await
        .context("join initial build")??;
        ui::success(format!(
            "initial build: {} processed, {} cached, {} generated",
            built.processed, built.cached, built.generated
        ));
    }

    // Filesystem watcher -> tokio channel.
    let (fs_tx, fs_rx) = mpsc::unbounded_channel::<ChangeBatch>();
    let _watcher = spawn_watcher(&root, &source_dir, &config_path, &plugin_dirs, fs_tx)?;

    // Rebuild loop.
    {
        let project = Arc::clone(&project);
        let reload_tx = reload_tx.clone();
        tokio::spawn(rebuild_loop(project, fs_rx, reload_tx));
    }

    // HTTP server.
    let app = Router::new()
        .route("/events", get(sse_handler))
        .fallback_service(ServeDir::new(output_dir.clone()))
        .with_state(reload_tx.clone());

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

/// The SSE endpoint: streams reload events to the connected client.
async fn sse_handler(
    State(tx): State<broadcast::Sender<String>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let rx = tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|msg| match msg {
        Ok(data) => Some(Ok(Event::default().data(data))),
        Err(_) => None,
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// The rebuild loop: drains change batches, rebuilds, and broadcasts reloads.
async fn rebuild_loop(
    project: Arc<tokio::sync::Mutex<Project>>,
    mut fs_rx: mpsc::UnboundedReceiver<ChangeBatch>,
    reload_tx: broadcast::Sender<String>,
) {
    while let Some(batch) = fs_rx.recv().await {
        let project = Arc::clone(&project);
        let result = tokio::task::spawn_blocking(move || rebuild_once(&project, &batch)).await;

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

/// Perform one rebuild for a change batch. Returns the SSE payload (or `None`
/// when nothing changed on disk). Runs on a blocking thread.
fn rebuild_once(
    project: &Arc<tokio::sync::Mutex<Project>>,
    batch: &ChangeBatch,
) -> Result<Option<String>> {
    let started = std::time::Instant::now();
    let mut guard = project.blocking_lock();

    // Config change -> reload the whole project from disk.
    if batch.kind_config {
        let reloaded = Project::discover(&guard.root).context("reloading rpp.toml")?;
        let topology_changed = guard.source_dir() != reloaded.source_dir()
            || guard.output_dir() != reloaded.output_dir()
            || guard.config.dev.host != reloaded.config.dev.host
            || guard.config.dev.port != reloaded.config.dev.port
            || local_plugin_dirs(&guard) != local_plugin_dirs(&reloaded);
        if topology_changed {
            anyhow::bail!(
                "source, output, dev address, or local plugin paths changed; restart `rpp dev`"
            );
        }
        *guard = reloaded;
    }

    // Build a fresh engine (this reloads plugin factories, so it also covers
    // plugin-file changes). For pure source changes this is still correct and
    // the engine's cache keeps it cheap.
    let engine = guard.build_engine()?;
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

    Ok(Some(reload_payload(&changed)))
}

/// Build the JSON SSE payload `{"type":"reload","changed":[...]}`.
fn reload_payload(changed: &[String]) -> String {
    serde_json::json!({ "type": "reload", "changed": changed }).to_string()
}

/// Spawn the debounced filesystem watcher. The returned debouncer must be kept
/// alive for the duration of the server.
fn spawn_watcher(
    root: &Path,
    source_dir: &Path,
    config_path: &Path,
    plugin_dirs: &[PathBuf],
    tx: mpsc::UnboundedSender<ChangeBatch>,
) -> Result<
    notify_debouncer_full::Debouncer<
        notify_debouncer_full::notify::RecommendedWatcher,
        notify_debouncer_full::FileIdMap,
    >,
> {
    let source_dir = source_dir.to_path_buf();
    let config_path = config_path.to_path_buf();
    let plugin_dirs = plugin_dirs.to_vec();
    let ignore_dir = root.join(".rpp"); // ignore our own cache writes

    // Clones moved into the classifier closure (the originals drive the watches).
    let cls_source = source_dir.clone();
    let cls_config = config_path.clone();
    let cls_plugins = plugin_dirs.clone();
    let cls_ignore = ignore_dir.clone();

    let mut debouncer = new_debouncer(
        Duration::from_millis(150),
        None,
        move |result: notify_debouncer_full::DebounceEventResult| {
            let events = match result {
                Ok(events) => events,
                Err(_) => return,
            };
            if let Some(batch) =
                classify(&events, &cls_source, &cls_config, &cls_plugins, &cls_ignore)
            {
                let _ = tx.send(batch);
            }
        },
    )
    .context("creating filesystem watcher")?;

    // Watch the source dir, plugin dirs, and the config file's directory.
    debouncer
        .watcher()
        .watch(&source_dir, RecursiveMode::Recursive)
        .with_context(|| format!("watching {}", source_dir.display()))?;
    for dir in &plugin_dirs {
        debouncer
            .watcher()
            .watch(dir, RecursiveMode::Recursive)
            .with_context(|| format!("watching {}", dir.display()))?;
    }
    if let Some(parent) = config_path.parent() {
        debouncer
            .watcher()
            .watch(parent, RecursiveMode::NonRecursive)
            .with_context(|| format!("watching {}", parent.display()))?;
    }
    Ok(debouncer)
}

/// Classify a debounced event batch into a [`ChangeBatch`], ignoring writes
/// under `.rpp` (our own cache/output bookkeeping).
fn classify(
    events: &[DebouncedEvent],
    source_dir: &Path,
    config_path: &Path,
    plugin_dirs: &[PathBuf],
    ignore_dir: &Path,
) -> Option<ChangeBatch> {
    let mut batch = ChangeBatch::default();
    for event in events {
        for path in &event.paths {
            if path.starts_with(ignore_dir) {
                continue;
            }
            let kind = classify_path(path, source_dir, config_path, plugin_dirs);
            match kind {
                Some(ChangeKind::Config) => batch.kind_config = true,
                Some(ChangeKind::Plugin) => batch.kind_plugin = true,
                Some(ChangeKind::Source) => {}
                None => continue,
            }
            batch.paths.push(path.clone());
        }
    }
    if batch.paths.is_empty() {
        None
    } else {
        Some(batch)
    }
}

fn classify_path(
    path: &Path,
    source_dir: &Path,
    config_path: &Path,
    plugin_dirs: &[PathBuf],
) -> Option<ChangeKind> {
    if path == config_path {
        return Some(ChangeKind::Config);
    }
    if plugin_dirs.iter().any(|d| path.starts_with(d)) {
        return Some(ChangeKind::Plugin);
    }
    if path.starts_with(source_dir) {
        return Some(ChangeKind::Source);
    }
    None
}

/// The set of local (`path:`) plugin directories to watch.
fn local_plugin_dirs(project: &Project) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for plugin in &project.config.plugins {
        if let Some(rest) = plugin.source.strip_prefix("path:") {
            let rest = rest.trim();
            if rest.is_empty() {
                continue;
            }
            let dir = PathBuf::from(rest);
            let abs = if dir.is_absolute() {
                dir
            } else {
                project.root.join(dir)
            };
            dirs.push(abs);
        }
    }
    dirs
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
