use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
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
                            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
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
