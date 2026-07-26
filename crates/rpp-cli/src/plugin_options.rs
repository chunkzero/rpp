//! Parsing and application of CLI plugin-option overrides.

use std::collections::BTreeSet;

use anyhow::{bail, Context, Result};

/// Parsed `--plugin-opt <plugin-id>.<dotted.key>=<TOML value>` arguments.
#[derive(Debug, Clone, Default)]
pub(crate) struct PluginOptionOverrides {
    entries: Vec<PluginOptionOverride>,
}

#[derive(Debug, Clone)]
struct PluginOptionOverride {
    plugin_id: String,
    key: Vec<String>,
    value: toml::Value,
}

impl PluginOptionOverrides {
    /// Parse CLI arguments, retaining only the last value for an exact duplicate key.
    pub(crate) fn parse(raw: &[String]) -> Result<Self> {
        let mut entries: Vec<PluginOptionOverride> = Vec::new();
        for argument in raw {
            let parsed = PluginOptionOverride::parse(argument)?;
            if let Some(index) = entries
                .iter()
                .position(|entry| entry.plugin_id == parsed.plugin_id && entry.key == parsed.key)
            {
                entries.remove(index);
            }
            entries.push(parsed);
        }
        Ok(Self { entries })
    }

    /// Apply all overrides for `plugin_id` to an options table.
    pub(crate) fn apply(&self, plugin_id: &str, options: &mut toml::Value) {
        for entry in self
            .entries
            .iter()
            .filter(|entry| entry.plugin_id == plugin_id)
        {
            set_dotted_value(options, &entry.key, entry.value.clone());
        }
    }

    /// Reject overrides that do not name an exact resolved plugin id.
    pub(crate) fn validate_plugin_ids<'a>(
        &self,
        plugin_ids: impl IntoIterator<Item = &'a str>,
    ) -> Result<()> {
        let known: BTreeSet<&str> = plugin_ids.into_iter().collect();
        let unknown: BTreeSet<&str> = self
            .entries
            .iter()
            .map(|entry| entry.plugin_id.as_str())
            .filter(|id| !known.contains(id))
            .collect();
        if !unknown.is_empty() {
            bail!(
                "unknown plugin id{} in `--plugin-opt`: {}",
                if unknown.len() == 1 { "" } else { "s" },
                unknown
                    .into_iter()
                    .map(|id| format!("`{id}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        Ok(())
    }

    /// Render the active overrides for command summary output.
    pub(crate) fn summary(&self) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }
        Some(
            self.entries
                .iter()
                .map(PluginOptionOverride::display)
                .collect::<Vec<_>>()
                .join(", "),
        )
    }
}

impl PluginOptionOverride {
    fn parse(argument: &str) -> Result<Self> {
        let (target, value) = argument.split_once('=').ok_or_else(|| {
            anyhow::anyhow!(
                "invalid `--plugin-opt` `{argument}`: expected \
                 `<plugin-id>.<dotted.key>=<TOML value>`"
            )
        })?;
        let (plugin_id, key) = target.split_once('.').ok_or_else(|| {
            anyhow::anyhow!(
                "invalid `--plugin-opt` `{argument}`: expected a plugin id and dotted key"
            )
        })?;

        if !valid_plugin_id(plugin_id) {
            bail!("invalid plugin id `{plugin_id}` in `--plugin-opt`");
        }

        let key: Vec<String> = key.split('.').map(str::to_string).collect();
        if key.iter().any(|segment| !valid_key_segment(segment)) {
            bail!("invalid dotted key `{}` in `--plugin-opt`", key.join("."));
        }

        let value = value
            .parse::<toml::Value>()
            .with_context(|| format!("invalid TOML value for `{target}`"))?;
        Ok(Self {
            plugin_id: plugin_id.to_string(),
            key,
            value,
        })
    }

    fn display(&self) -> String {
        format!("{}.{}={}", self.plugin_id, self.key.join("."), self.value)
    }
}

fn valid_plugin_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    matches!(bytes.next(), Some(byte) if byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
}

fn valid_key_segment(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn set_dotted_value(options: &mut toml::Value, key: &[String], value: toml::Value) {
    if !options.is_table() {
        *options = toml::Value::Table(toml::map::Map::new());
    }
    let mut table = options
        .as_table_mut()
        .expect("plugin options initialized as a table");

    for segment in &key[..key.len() - 1] {
        let nested = table
            .entry(segment.clone())
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
        if !nested.is_table() {
            *nested = toml::Value::Table(toml::map::Map::new());
        }
        table = nested
            .as_table_mut()
            .expect("nested plugin option initialized as a table");
    }
    table.insert(key[key.len() - 1].clone(), value);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &[&str]) -> PluginOptionOverrides {
        PluginOptionOverrides::parse(
            &raw.iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }

    fn applied(raw: &[&str]) -> toml::Value {
        let overrides = parse(raw);
        let mut options = toml::Value::Table(toml::map::Map::new());
        overrides.apply("example", &mut options);
        options
    }

    #[test]
    fn applies_dotted_keys() {
        let overrides = parse(&["example.render.colors.primary=\"#ff00ff\""]);
        let mut options: toml::Value = toml::from_str("[render]\nquality = \"high\"\n").unwrap();
        overrides.apply("example", &mut options);

        assert_eq!(
            options["render"]["colors"]["primary"].as_str(),
            Some("#ff00ff")
        );
        assert_eq!(options["render"]["quality"].as_str(), Some("high"));
    }

    #[test]
    fn parses_toml_value_types() {
        let options = applied(&[
            "example.string=\"text\"",
            "example.integer=42",
            "example.float=1.5",
            "example.boolean=true",
            "example.datetime=1979-05-27T07:32:00Z",
            "example.array=[1, 2, 3]",
            "example.table={ enabled = true }",
        ]);

        assert_eq!(options["string"].as_str(), Some("text"));
        assert_eq!(options["integer"].as_integer(), Some(42));
        assert_eq!(options["float"].as_float(), Some(1.5));
        assert_eq!(options["boolean"].as_bool(), Some(true));
        assert!(options["datetime"].as_datetime().is_some());
        assert_eq!(options["array"].as_array().unwrap().len(), 3);
        assert_eq!(options["table"]["enabled"].as_bool(), Some(true));
        assert!(PluginOptionOverrides::parse(&["example.bad=unquoted".to_string()]).is_err());
    }

    #[test]
    fn duplicate_key_uses_last_value() {
        let overrides = parse(&[
            "example.nested.value=1",
            "example.other=true",
            "example.nested.value=2",
        ]);
        let mut options = toml::Value::Table(toml::map::Map::new());
        overrides.apply("example", &mut options);

        assert_eq!(options["nested"]["value"].as_integer(), Some(2));
        assert_eq!(
            overrides.summary().as_deref(),
            Some("example.other=true, example.nested.value=2")
        );
    }

    #[test]
    fn rejects_unknown_plugin_id() {
        let overrides = parse(&["missing.enabled=true"]);
        let error = overrides.validate_plugin_ids(["example"]).unwrap_err();
        assert!(error.to_string().contains("unknown plugin id"));
        assert!(error.to_string().contains("`missing`"));
    }
}
