//! Filesystem watching and change classification for `rpp dev`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::{Context, Result};
use notify_debouncer_full::notify::{RecommendedWatcher, RecursiveMode, Watcher};
use notify_debouncer_full::{new_debouncer, DebouncedEvent, Debouncer, FileIdMap};
use tokio::sync::mpsc;

use crate::project::Project;
use crate::ui;

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

    /// Fold another batch into this one.
    pub fn merge(&mut self, other: ChangeBatch) {
        self.kind_config |= other.kind_config;
        self.kind_plugin |= other.kind_plugin;
        self.paths.extend(other.paths);
    }
}

/// The live filesystem watcher. Plugin directories are re-targeted when
/// `rpp.toml` changes; the source directory and config path are fixed for the
/// life of the session.
pub struct DevWatcher {
    debouncer: Debouncer<RecommendedWatcher, FileIdMap>,
    plugin_dirs: Arc<RwLock<Vec<PathBuf>>>,
}

impl DevWatcher {
    /// Watch `dirs` instead of the current plugin directories.
    pub fn set_plugin_dirs(&mut self, dirs: Vec<PathBuf>) -> Result<()> {
        let current = self
            .plugin_dirs
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let mut added = Vec::new();
        for dir in dirs.iter().filter(|dir| !current.contains(dir)) {
            if let Err(error) = self
                .debouncer
                .watcher()
                .watch(dir, RecursiveMode::Recursive)
            {
                for added_dir in added {
                    let _ = self.debouncer.watcher().unwatch(added_dir);
                }
                return Err(error).with_context(|| format!("watching {}", dir.display()));
            }
            added.push(dir.as_path());
        }
        for dir in current.iter().filter(|dir| !dirs.contains(dir)) {
            let _ = self.debouncer.watcher().unwatch(dir);
        }
        *self.plugin_dirs.write().unwrap_or_else(|e| e.into_inner()) = dirs;
        Ok(())
    }
}

/// Spawn the debounced filesystem watcher. The returned watcher must be kept
/// alive for the duration of the server.
pub fn spawn_watcher(
    root: &Path,
    source_dir: &Path,
    config_path: &Path,
    initial_plugin_dirs: Vec<PathBuf>,
    tx: mpsc::UnboundedSender<ChangeBatch>,
) -> Result<DevWatcher> {
    let source_dir = source_dir.to_path_buf();
    let config_path = config_path.to_path_buf();
    let ignore_dir = root.join(".rpp");
    let plugin_dirs = Arc::new(RwLock::new(Vec::new()));

    let cls_source = source_dir.clone();
    let cls_config = config_path.clone();
    let cls_plugins = Arc::clone(&plugin_dirs);

    let mut debouncer = new_debouncer(
        Duration::from_millis(150),
        None,
        move |result: notify_debouncer_full::DebounceEventResult| {
            let events = match result {
                Ok(events) => events,
                Err(errors) => {
                    for error in errors {
                        ui::warn(format!("file watcher: {error}"));
                    }
                    return;
                }
            };
            let plugins = cls_plugins
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            if let Some(batch) = classify(&events, &cls_source, &cls_config, &plugins, &ignore_dir)
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
    if let Some(parent) = config_path.parent() {
        debouncer
            .watcher()
            .watch(parent, RecursiveMode::NonRecursive)
            .with_context(|| format!("watching {}", parent.display()))?;
    }
    let mut watcher = DevWatcher {
        debouncer,
        plugin_dirs,
    };
    watcher.set_plugin_dirs(initial_plugin_dirs)?;
    Ok(watcher)
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
    collect_local_plugin_dirs(
        &mut dirs,
        &project.user_plugins.plugins,
        &project.user_plugins.root,
    );
    collect_local_plugin_dirs(&mut dirs, &project.config.plugins, &project.root);
    dirs
}

fn collect_local_plugin_dirs(
    dirs: &mut Vec<PathBuf>,
    plugins: &[rpp::config::PluginConfig],
    root: &Path,
) {
    for plugin in plugins {
        if let Some(rest) = plugin
            .source
            .as_deref()
            .and_then(|source| source.strip_prefix("path:"))
        {
            let rest = rest.trim();
            if rest.is_empty() {
                continue;
            }
            let dir = PathBuf::from(rest);
            let abs = if dir.is_absolute() {
                dir
            } else {
                root.join(dir)
            };
            dirs.push(abs);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_retarget_preserves_previous_watches() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("src");
        let original = root.path().join("original");
        let added = root.path().join("added");
        for dir in [&source, &original, &added] {
            std::fs::create_dir(dir).unwrap();
        }
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut watcher = spawn_watcher(
            root.path(),
            &source,
            &root.path().join("rpp.toml"),
            vec![original.clone()],
            tx,
        )
        .unwrap();
        assert!(watcher
            .set_plugin_dirs(vec![added.clone(), root.path().join("missing")])
            .is_err());
        watcher.set_plugin_dirs(vec![original.clone()]).unwrap();
        assert!(watcher.debouncer.watcher().unwatch(&original).is_ok());
        assert!(watcher.debouncer.watcher().unwatch(&added).is_err());
    }
}
