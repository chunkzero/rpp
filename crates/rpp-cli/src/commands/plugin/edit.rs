//! `toml_edit`-based editing of the `[[plugin]]` array in `rpp.toml`,
//! preserving the user's formatting and comments. Pure string-in/string-out so
//! it is straightforward to unit test.

use std::path::Path;

use anyhow::{bail, Context, Result};
use rpp_fetch::{parse_manifest_summary, PluginSource};
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table};

/// A `[[plugin]]` entry to add.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginEntry {
    /// The `source` string.
    pub source: String,
    /// Optional `ref`.
    pub r#ref: Option<String>,
    /// Optional `subdir`.
    pub subdir: Option<String>,
    /// Optional `origin`: the directory a global install was copied from.
    pub origin: Option<String>,
}

/// Append a `[[plugin]]` entry to `rpp.toml` text, returning the updated text.
///
/// Fails if an entry with the same `source` already exists.
pub fn add_plugin(toml_text: &str, entry: PluginEntry) -> Result<String> {
    let mut doc: DocumentMut = toml_text.parse().context("parsing rpp.toml for editing")?;

    if plugin_sources(&doc).iter().any(|s| s == &entry.source) {
        bail!(
            "a plugin with source `{}` is already configured",
            entry.source
        );
    }

    let array = doc
        .entry("plugin")
        .or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new()));
    let array = array
        .as_array_of_tables_mut()
        .context("`plugin` is not an array of tables")?;

    let mut table = Table::new();
    table["source"] = toml_edit::value(entry.source);
    if let Some(r) = entry.r#ref {
        table["ref"] = toml_edit::value(r);
    }
    if let Some(s) = entry.subdir {
        table["subdir"] = toml_edit::value(s);
    }
    if let Some(origin) = entry.origin {
        table["origin"] = toml_edit::value(origin);
    }
    array.push(table);

    Ok(doc.to_string())
}

/// Remove the `[[plugin]]` entry matching `id_or_source` from `rpp.toml` text.
///
/// Matching is by exact `source` or `id` string first; if that fails, each
/// path/github source is resolved enough to read its plugin id (path sources
/// resolve their `plugin.toml` relative to `project_root`).
///
/// Returns `(updated_text, removed_source)` where `removed_source` is `None`
/// when the removed entry has no source or nothing matched. Unmatched input
/// is returned unchanged.
pub fn remove_plugin(
    toml_text: &str,
    id_or_source: &str,
    project_root: &Path,
) -> Result<(String, Option<String>)> {
    let mut doc: DocumentMut = toml_text.parse().context("parsing rpp.toml for editing")?;

    let Some(array) = doc.get_mut("plugin").and_then(Item::as_array_of_tables_mut) else {
        return Ok((toml_text.to_owned(), None));
    };

    let mut found_index = None;
    let mut removed_source = None;
    for (i, table) in array.iter().enumerate() {
        let source = table.get("source").and_then(|v| v.as_str());
        let id = table.get("id").and_then(|v| v.as_str());
        if source == Some(id_or_source)
            || id == Some(id_or_source)
            || source
                .is_some_and(|source| plugin_id_matches(table, source, project_root, id_or_source))
        {
            found_index = Some(i);
            removed_source = source.map(str::to_string);
            break;
        }
    }

    if let Some(i) = found_index {
        array.remove(i);
    } else {
        return Ok((toml_text.to_owned(), None));
    }
    Ok((doc.to_string(), removed_source))
}

/// Whether the plugin entry's resolved id equals `target`.
fn plugin_id_matches(table: &Table, source: &str, project_root: &Path, target: &str) -> bool {
    let parsed = match PluginSource::parse(
        source,
        table.get("ref").and_then(|v| v.as_str()),
        table.get("subdir").and_then(|v| v.as_str()),
    ) {
        Ok(p) => p,
        Err(_) => return false,
    };
    // Only path sources can be resolved offline without network/cache.
    if let PluginSource::Path { dir } = parsed {
        let abs = if dir.is_absolute() {
            dir
        } else {
            project_root.join(dir)
        };
        if let Ok(summary) = parse_manifest_summary(&abs) {
            return summary.id == target;
        }
    }
    false
}

/// Collect the `source` strings of all configured plugins.
fn plugin_sources(doc: &DocumentMut) -> Vec<String> {
    let Some(array) = doc.get("plugin").and_then(Item::as_array_of_tables) else {
        return Vec::new();
    };
    array
        .iter()
        .filter_map(|t| t.get("source").and_then(|v| v.as_str()).map(str::to_string))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r#"# my pack
[pack]
name = "demo"  # keep this comment

[[plugin]]
source = "path:plugins/json-minify"
[plugin.options]
pretty = false
"#;

    #[test]
    fn add_preserves_comments_and_appends() {
        let out = add_plugin(
            BASE,
            PluginEntry {
                source: "github:example/atlas".into(),
                r#ref: Some("v1.0.0".into()),
                subdir: Some("plugins/atlas".into()),
                origin: None,
            },
        )
        .unwrap();
        // Comments preserved.
        assert!(out.contains("# my pack"));
        assert!(out.contains("# keep this comment"));
        // New entry appended with ref + subdir.
        assert!(out.contains("source = \"github:example/atlas\""));
        assert!(out.contains("ref = \"v1.0.0\""));
        assert!(out.contains("subdir = \"plugins/atlas\""));
        // Original entry still present.
        assert!(out.contains("source = \"path:plugins/json-minify\""));
    }

    #[test]
    fn add_rejects_duplicate_source() {
        let err = add_plugin(
            BASE,
            PluginEntry {
                source: "path:plugins/json-minify".into(),
                r#ref: None,
                subdir: None,
                origin: None,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("already configured"));
    }

    #[test]
    fn add_to_config_without_plugins() {
        let text = "[pack]\nname = \"x\"\n";
        let out = add_plugin(
            text,
            PluginEntry {
                source: "github:a/b".into(),
                r#ref: None,
                subdir: None,
                origin: None,
            },
        )
        .unwrap();
        assert!(out.contains("[[plugin]]"));
        assert!(out.contains("source = \"github:a/b\""));
    }

    #[test]
    fn remove_by_source() {
        let dir = tempfile::tempdir().unwrap();
        let (out, removed) = remove_plugin(BASE, "path:plugins/json-minify", dir.path()).unwrap();
        assert_eq!(removed.as_deref(), Some("path:plugins/json-minify"));
        assert!(!out.contains("json-minify"));
        // Pack section preserved.
        assert!(out.contains("name = \"demo\""));
    }

    #[test]
    fn remove_nonexistent_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let (out, removed) = remove_plugin(BASE, "github:no/such", dir.path()).unwrap();
        assert_eq!(removed, None);
        assert_eq!(out, BASE);
    }

    #[test]
    fn remove_by_plugin_id() {
        // Build a real plugin dir so id resolution works.
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("plugins/json-minify");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.toml"),
            "[plugin]\nid = \"json-minify\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();

        let (out, removed) = remove_plugin(BASE, "json-minify", dir.path()).unwrap();
        assert_eq!(removed.as_deref(), Some("path:plugins/json-minify"));
        assert!(!out.contains("[[plugin]]"));
    }

    #[test]
    fn remove_by_global_id_entry() {
        let text = r#"[pack]
name = "demo"

[[plugin]]
id = "window"
[plugin.options]
namespace = "window"
"#;
        let dir = tempfile::tempdir().unwrap();
        let (out, removed) = remove_plugin(text, "window", dir.path()).unwrap();
        assert_eq!(removed, None);
        assert!(!out.contains("[[plugin]]"));
        assert!(out.contains("name = \"demo\""));
    }
}
