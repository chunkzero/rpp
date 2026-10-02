//! Filesystem watching and change classification for `rpp dev`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::{Context, Result};
use notify_debouncer_full::notify::{RecommendedWatcher, RecursiveMode, Watcher};
use notify_debouncer_full::{
    new_debouncer, DebounceEventResult, DebouncedEvent, Debouncer, FileIdMap,
};
use tokio::sync::mpsc;

use crate::project::Project;

/// What kind of change a filesystem event represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChangeKind {
    /// A source file under the build source dir.
    Source,
    /// A plugin file (rebuild the engine with reloaded factories).
    Plugin,
    /// A project config file (full project reload).
    Config,
}

/// A classified, deduplicated batch of changed paths.
#[derive(Debug, Default, Clone)]
pub struct ChangeBatch {
    /// Whether a project config file changed.
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

/// The live filesystem watcher. Plugin directories and config files are re-targeted
/// when the project config changes; the source directory is fixed for the life of
/// the session.
pub struct DevWatcher {
    debouncer: Debouncer<RecommendedWatcher, FileIdMap>,
    plugin_dirs: Arc<RwLock<Vec<PathBuf>>>,
    config_files: Arc<RwLock<Vec<PathBuf>>>,
    source_dir: PathBuf,
    watched_dirs: Vec<PathBuf>,
}

impl DevWatcher {
    /// Treat `files` as project config files, watching their directories.
    pub fn set_config_files(&mut self, files: Vec<PathBuf>) -> Result<()> {
        let plugin_dirs = snapshot(&self.plugin_dirs);
        for dir in files.iter().filter_map(|file| file.parent()) {
            if self.watched_dirs.iter().any(|d| d == dir)
                || dir.starts_with(&self.source_dir)
                || plugin_dirs.iter().any(|plugin| dir.starts_with(plugin))
            {
                continue;
            }
            self.debouncer
                .watcher()
                .watch(dir, RecursiveMode::NonRecursive)
                .with_context(|| format!("watching {}", dir.display()))?;
            self.watched_dirs.push(dir.to_path_buf());
        }
        *self.config_files.write().unwrap_or_else(|e| e.into_inner()) = files;
        Ok(())
    }

    /// Watch `dirs` instead of the current plugin directories.
    pub fn set_plugin_dirs(&mut self, dirs: Vec<PathBuf>) -> Result<()> {
        let current = snapshot(&self.plugin_dirs);
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
    let config_files = Arc::new(RwLock::new(vec![config_path.to_path_buf()]));
    let plugin_dirs = Arc::new(RwLock::new(Vec::new()));
    let handler = {
        let source_dir = source_dir.clone();
        let config_files = Arc::clone(&config_files);
        let plugin_dirs = Arc::clone(&plugin_dirs);
        let ignore_dir = root.join(".rpp");
        move |result: DebounceEventResult| {
            let events = match result {
                Ok(events) => events,
                Err(errors) => {
                    for error in errors {
                        tracing::warn!(%error, "File watcher error");
                    }
                    return;
                }
            };
            let configs = snapshot(&config_files);
            let plugins = snapshot(&plugin_dirs);
            if let Some(batch) = classify(&events, &source_dir, &configs, &plugins, &ignore_dir) {
                let _ = tx.send(batch);
            }
        }
    };
    let mut debouncer = new_debouncer(Duration::from_millis(150), None, handler)
        .context("creating filesystem watcher")?;

    debouncer
        .watcher()
        .watch(&source_dir, RecursiveMode::Recursive)
        .with_context(|| format!("watching {}", source_dir.display()))?;
    let mut watched_dirs = Vec::new();
    if let Some(parent) = config_path.parent() {
        debouncer
            .watcher()
            .watch(parent, RecursiveMode::NonRecursive)
            .with_context(|| format!("watching {}", parent.display()))?;
        watched_dirs.push(parent.to_path_buf());
    }
    let mut watcher = DevWatcher {
        debouncer,
        plugin_dirs,
        config_files,
        source_dir,
        watched_dirs,
    };
    watcher.set_plugin_dirs(initial_plugin_dirs)?;
    Ok(watcher)
}

/// A copy of the paths in `lock`, ignoring poisoning.
fn snapshot(lock: &RwLock<Vec<PathBuf>>) -> Vec<PathBuf> {
    lock.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Classify a debounced event batch into a [`ChangeBatch`], ignoring writes
/// under `.rpp` (our own cache/output bookkeeping).
fn classify(
    events: &[DebouncedEvent],
    source_dir: &Path,
    config_files: &[PathBuf],
    plugin_dirs: &[PathBuf],
    ignore_dir: &Path,
) -> Option<ChangeBatch> {
    let mut batch = ChangeBatch::default();
    for event in events {
        for path in &event.paths {
            if path.starts_with(ignore_dir) {
                continue;
            }
            let kind = classify_path(path, source_dir, config_files, plugin_dirs);
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
    config_files: &[PathBuf],
    plugin_dirs: &[PathBuf],
) -> Option<ChangeKind> {
    if config_files.iter().any(|file| file == path) {
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

/// The package directories to watch.
pub fn local_plugin_dirs(project: &Project) -> Vec<PathBuf> {
    project
        .config
        .plugins
        .iter()
        .filter_map(|plugin| project.ts.packages.get(&plugin.package))
        .map(|package| package.dir.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_kinds() {
        let src = PathBuf::from("/proj/src");
        let cfg = vec![PathBuf::from("/proj/rpp.config.ts")];
        let plugins = vec![PathBuf::from("/proj/plugins/hello")];
        let kind = |path: &str| classify_path(Path::new(path), &src, &cfg, &plugins);

        assert_eq!(kind("/proj/rpp.config.ts"), Some(ChangeKind::Config));
        assert_eq!(
            kind("/proj/plugins/hello/src/plugin.ts"),
            Some(ChangeKind::Plugin)
        );
        assert_eq!(kind("/proj/src/assets/x.json"), Some(ChangeKind::Source));
        assert_eq!(kind("/proj/other/x"), None);
    }

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
            &root.path().join("rpp.config.ts"),
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
