//! `toml_edit`-based editing of the `[[plugin]]` array in `rpp.toml`,
//! preserving the user's formatting and comments. Pure string-in/string-out so
//! it is straightforward to unit test.

use anyhow::{bail, Context, Result};
use rpp_fetch::PluginSource;
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
/// Fails if an entry with the same normalized source, ref, and subdir exists.
pub fn add_plugin(toml_text: &str, entry: PluginEntry) -> Result<String> {
    let mut doc: DocumentMut = toml_text.parse().context("parsing rpp.toml for editing")?;

    let identity = PluginSource::parse(
        &entry.source,
        entry.r#ref.as_deref(),
        entry.subdir.as_deref(),
    )?;
    if plugin_sources(&doc)?.contains(&identity) {
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
    table["source"] = toml_edit::value(identity.canonical());
    if let Some(r) = identity.requested_ref() {
        table["ref"] = toml_edit::value(r);
    }
    if let Some(s) = identity.subdir() {
        table["subdir"] = toml_edit::value(s);
    }
    if let Some(origin) = entry.origin {
        table["origin"] = toml_edit::value(origin);
    }
    array.push(table);

    Ok(doc.to_string())
}

/// Remove the exact entry selected by the command's resolver.
pub(super) fn remove_plugin_at(toml_text: &str, index: usize) -> Result<String> {
    let mut doc: DocumentMut = toml_text
        .parse()
        .context("parsing plugin manifest for editing")?;
    let array = doc
        .get_mut("plugin")
        .and_then(Item::as_array_of_tables_mut)
        .context("plugin entries disappeared before editing")?;
    anyhow::ensure!(
        index < array.len(),
        "selected plugin entry disappeared before editing"
    );
    array.remove(index);
    Ok(doc.to_string())
}

/// Collect normalized source identities of all configured plugins.
fn plugin_sources(doc: &DocumentMut) -> Result<Vec<PluginSource>> {
    let Some(array) = doc.get("plugin").and_then(Item::as_array_of_tables) else {
        return Ok(Vec::new());
    };
    array
        .iter()
        .filter_map(|t| {
            t.get("source").and_then(|v| v.as_str()).map(|source| {
                PluginSource::parse(
                    source,
                    t.get("ref").and_then(Item::as_str),
                    t.get("subdir").and_then(Item::as_str),
                )
                .map_err(Into::into)
            })
        })
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
}
