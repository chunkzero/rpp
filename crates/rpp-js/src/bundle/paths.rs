use std::path::{Component, Path, PathBuf};

use rolldown_sourcemap::{JSONSourceMap, SourceMap};

use super::{Package, NODE_MODULES, VIRTUAL_PREFIX};

/// Turns a source path relative to `map_dir` (as Rolldown reports it) into the form listed
/// in the source map: relative to `root`, under `node_modules/`, or under a package specifier.
pub(super) fn display_source(
    root: &Path,
    map_dir: &Path,
    packages: &[Package],
    allow_node_modules: bool,
    source: &str,
) -> String {
    if let Some(at) = source.find(VIRTUAL_PREFIX) {
        return source[at + VIRTUAL_PREFIX.len()..].to_string();
    }
    let absolute = normalize(&map_dir.join(source));
    if allow_node_modules {
        let components: Vec<_> = absolute.components().collect();
        if let Some(at) = components
            .iter()
            .rposition(|c| c.as_os_str() == NODE_MODULES)
        {
            let tail: PathBuf = components[at..].iter().collect();
            return tail.to_string_lossy().replace('\\', "/");
        }
    }
    for package in packages.iter().filter(|package| package.dir != root) {
        if let Ok(relative) = absolute.strip_prefix(&package.dir) {
            let relative = relative.to_string_lossy().replace('\\', "/");
            return format!("{}/{relative}", package.specifier);
        }
    }
    match absolute.strip_prefix(root) {
        Ok(relative) => relative.to_string_lossy().replace('\\', "/"),
        Err(_) => source.replace('\\', "/"),
    }
}

/// Resolves `.` and `..` components without touching the filesystem.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Parses a source map, making each source an absolute normalized path.
pub(super) fn absolute_sources(json: &str, dir: &Path) -> Option<SourceMap> {
    let mut map: JSONSourceMap = serde_json::from_str(json).ok()?;
    let base = match map.source_root.take() {
        Some(source_root) => dir.join(source_root),
        None => dir.to_path_buf(),
    };
    for source in &mut map.sources {
        *source = normalize(&base.join(&*source))
            .to_string_lossy()
            .into_owned();
    }
    SourceMap::from_json(map).ok()
}

pub(super) fn relative_to(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
