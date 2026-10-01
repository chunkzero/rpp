//! Entry discovery: plugin-declared source patterns and the generated `rpp:discovered` module.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::engine::discovery;
use crate::util::glob::{self, Glob};

static NAMESPACE_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9_.-]+$").expect("static namespace regex is valid"));

/// A source file matched by a discovery pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiscoveredEntry {
    pub(crate) name: String,
    /// Source-relative, forward-slash path.
    pub(crate) path: String,
    pub(crate) namespace: Option<String>,
}

struct Pattern {
    name: String,
    glob: Glob,
    /// Index of the path segment matched by the first whole `*` segment, when every
    /// segment before it is literal.
    namespace_segment: Option<usize>,
}

/// Compiled discovery patterns, in name order.
pub(crate) struct Discovery {
    patterns: Vec<Pattern>,
}

impl Discovery {
    pub(crate) fn new(patterns: &BTreeMap<String, String>) -> Result<Self, String> {
        let patterns = patterns
            .iter()
            .map(|(name, pattern)| {
                Ok(Pattern {
                    name: name.clone(),
                    glob: glob::compile(pattern).map_err(|e| format!("discover `{name}`: {e}"))?,
                    namespace_segment: namespace_segment(pattern),
                })
            })
            .collect::<Result<_, String>>()?;
        Ok(Self { patterns })
    }

    /// Whether `rel` matches any pattern.
    pub(crate) fn matches(&self, rel: &str) -> bool {
        self.patterns.iter().any(|p| p.glob.matches(rel))
    }

    /// Every matching file under `source`, ordered by name then path.
    pub(crate) fn discover(&self, source: &Path) -> Result<Vec<DiscoveredEntry>, String> {
        let files = discovery::discover(source).map_err(|e| e.to_string())?;
        let mut entries = Vec::new();
        for pattern in &self.patterns {
            for file in files.iter().filter(|f| pattern.glob.matches(&f.rel)) {
                let namespace = pattern
                    .namespace_segment
                    .and_then(|index| file.rel.split('/').nth(index))
                    .map(str::to_string);
                if let Some(namespace) = &namespace {
                    if !NAMESPACE_REGEX.is_match(namespace) {
                        return Err(format!(
                            "namespace `{namespace}` of `{}` must match ^[a-z0-9_.-]+$",
                            file.rel
                        ));
                    }
                }
                entries.push(DiscoveredEntry {
                    name: pattern.name.clone(),
                    path: file.rel.clone(),
                    namespace,
                });
            }
        }
        Ok(entries)
    }
}

fn namespace_segment(pattern: &str) -> Option<usize> {
    for (index, segment) in pattern.split('/').enumerate() {
        if segment == "*" {
            return Some(index);
        }
        if segment.contains(['*', '?', '[']) {
            return None;
        }
    }
    None
}

/// The source of `rpp:discovered`: every declared name maps to its modules.
/// `source_from_root` is the source directory relative to the bundle root, or empty.
pub(crate) fn discovered_module(
    source_from_root: &str,
    discovery: &Discovery,
    entries: &[DiscoveredEntry],
) -> String {
    let json = |value: &str| Value::String(value.to_string()).to_string();
    let prefix = if source_from_root.is_empty() {
        ".".to_string()
    } else {
        format!("./{source_from_root}")
    };

    let mut code = String::new();
    for (index, entry) in entries.iter().enumerate() {
        code.push_str(&format!(
            "import * as m{index} from {};\n",
            json(&format!("{prefix}/{}", entry.path))
        ));
    }
    code.push_str("export default {\n");
    for pattern in &discovery.patterns {
        code.push_str(&format!("  {}: [\n", json(&pattern.name)));
        for (index, entry) in entries.iter().enumerate() {
            if entry.name != pattern.name {
                continue;
            }
            let namespace = entry
                .namespace
                .as_deref()
                .map(|n| format!(" namespace: {},", json(n)))
                .unwrap_or_default();
            code.push_str(&format!(
                "    {{ path: {},{namespace} module: m{index} }},\n",
                json(&entry.path)
            ));
        }
        code.push_str("  ],\n");
    }
    code.push_str("};\n");
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_is_the_first_whole_star_after_literals() {
        assert_eq!(namespace_segment("*/window/**/window.ts"), Some(0));
        assert_eq!(namespace_segment("ui/*/main.ts"), Some(1));
        assert_eq!(namespace_segment("**/window.ts"), None);
        assert_eq!(namespace_segment("a*/b/*.ts"), None);
        assert_eq!(namespace_segment("a/b.ts"), None);
    }
}
