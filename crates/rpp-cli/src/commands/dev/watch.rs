//! Filesystem watching and change classification for `rpp dev`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use notify_debouncer_full::notify::{RecursiveMode, Watcher};
use notify_debouncer_full::{new_debouncer, DebouncedEvent};
use tokio::sync::mpsc;

use crate::project::Project;

/// What kind of change a filesystem event represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// A source file under the build source dir.
    Source,
    /// A plugin file (rebuild the engine with reloaded factories).
    Plugin,
    /// `rpp.toml` itself (full project reload).
    Config,
}

/// A classified, deduplicated batch of changed paths.
#[derive(Debug, Default, Clone)]
pub struct ChangeBatch {
    /// Whether `rpp.toml` changed.
    pub kind_config: bool,
    /// Whether a plugin directory changed.
    pub kind_plugin: bool,
    /// All changed paths in the batch.
    pub paths: Vec<PathBuf>,
}

impl ChangeBatch {
    /// Whether the engine topology (plugins/config) changed and must be rebuilt.
    pub fn needs_engine_rebuild(&self) -> bool {
        self.kind_config || self.kind_plugin
    }
}

/// Spawn the debounced filesystem watcher. The returned debouncer must be kept
/// alive for the duration of the server.
pub fn spawn_watcher(
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
    let ignore_dir = root.join(".rpp");

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
pub fn classify(
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

pub fn classify_path(
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
pub fn local_plugin_dirs(project: &Project) -> Vec<PathBuf> {
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
