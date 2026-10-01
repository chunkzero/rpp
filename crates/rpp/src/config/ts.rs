//! Conversion of the `rpp.config.ts` JSON value into [`Config`].

use std::path::PathBuf;

use serde_json::{Map, Value};

use super::Config;
use crate::error::{Error, Result};
use crate::util::json_toml::json_to_toml;

const REJECTED_PLUGIN_KEYS: [&str; 4] = ["id", "source", "ref", "subdir"];

pub(super) fn from_json(value: &Value, path: PathBuf) -> Result<Config> {
    let fail = |message: String| Error::Config {
        path: path.clone(),
        message,
    };

    reject_nulls(value, "config").map_err(&fail)?;
    let converted = convert_root(value).map_err(&fail)?;
    let table = json_to_toml(&converted).map_err(&fail)?;
    let config: Config = table
        .try_into()
        .map_err(|e| fail(to_camel_names(&e.to_string())))?;
    config.validate(&path).map_err(|e| match e {
        Error::Config { path, message } if message.starts_with("`build.lua.") => Error::Config {
            path,
            message: to_camel_names(&message.replace("build.lua.", "build.limits.")),
        },
        other => other,
    })?;
    Ok(config)
}

fn reject_nulls(value: &Value, at: &str) -> std::result::Result<(), String> {
    match value {
        Value::Null => Err(format!("`{at}` must not be null")),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .try_for_each(|(i, item)| reject_nulls(item, &format!("{at}[{i}]"))),
        Value::Object(map) => map
            .iter()
            .try_for_each(|(key, item)| reject_nulls(item, &format!("{at}.{key}"))),
        _ => Ok(()),
    }
}

fn convert_root(value: &Value) -> std::result::Result<Value, String> {
    let map = value.as_object().ok_or("the config must be an object")?;
    let mut out = Map::new();
    for (key, item) in map {
        match key.as_str() {
            "plugins" => {
                let items = item.as_array().ok_or("`plugins` must be an array")?;
                let plugins = items
                    .iter()
                    .enumerate()
                    .map(|(i, plugin)| convert_plugin(plugin, i))
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                out.insert("plugin".into(), Value::Array(plugins));
            }
            "build" => {
                out.insert(key.clone(), convert_build(item)?);
            }
            _ => {
                out.insert(snake_key(key)?, convert_keys(item)?);
            }
        }
    }
    Ok(Value::Object(out))
}

fn convert_build(value: &Value) -> std::result::Result<Value, String> {
    let Value::Object(map) = convert_keys(value)? else {
        return Err("`build` must be an object".into());
    };
    let mut out = Map::new();
    for (key, item) in map {
        match key.as_str() {
            "lua" => return Err("`build.lua` is not supported; use `build.limits`".into()),
            "limits" => out.insert("lua".into(), item),
            _ => out.insert(key, item),
        };
    }
    Ok(Value::Object(out))
}

fn convert_plugin(value: &Value, index: usize) -> std::result::Result<Value, String> {
    let at = format!("plugins[{index}]");
    let map = value
        .as_object()
        .ok_or_else(|| format!("`{at}` must be an object"))?;
    let mut out = Map::new();
    for (key, item) in map {
        match key.as_str() {
            "plugin" => {
                out.insert("package".into(), item.clone());
            }
            "options" | "outputs" => {
                out.insert(key.clone(), item.clone());
            }
            "security" if item.as_str() == Some("native") => {
                return Err(format!("`{at}.security` must not be \"native\""));
            }
            "permissions" => {
                if item.get("lua").is_some() {
                    return Err(format!("`{at}.permissions.lua` is not supported"));
                }
                out.insert(key.clone(), convert_keys(item)?);
            }
            key if REJECTED_PLUGIN_KEYS.contains(&key) => {
                return Err(format!("`{at}.{key}` is not supported; use `plugin`"));
            }
            _ => {
                out.insert(snake_key(key)?, convert_keys(item)?);
            }
        }
    }
    if !out.contains_key("package") {
        return Err(format!("`{at}` must set `plugin`"));
    }
    Ok(Value::Object(out))
}

/// Recursively converts object keys from camelCase to snake_case.
fn convert_keys(value: &Value) -> std::result::Result<Value, String> {
    Ok(match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| Ok((snake_key(key)?, convert_keys(item)?)))
                .collect::<std::result::Result<_, String>>()?,
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(convert_keys)
                .collect::<std::result::Result<_, _>>()?,
        ),
        other => other.clone(),
    })
}

fn snake_key(key: &str) -> std::result::Result<String, String> {
    if key.contains('_') {
        return Err(format!("unknown field `{key}`; keys are camelCase"));
    }
    let mut out = String::with_capacity(key.len() + 2);
    for c in key.chars() {
        if c.is_ascii_uppercase() {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    Ok(out)
}

/// Rewrites backtick-quoted snake_case names in `message` to camelCase.
fn to_camel_names(message: &str) -> String {
    message
        .split('`')
        .enumerate()
        .map(|(i, part)| {
            if i % 2 == 1 {
                camel_name(part)
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("`")
}

fn camel_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut upper = false;
    for c in name.chars() {
        if c == '_' && !out.is_empty() {
            upper = true;
        } else if upper {
            out.push(c.to_ascii_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::config::{PngSetting, SecurityMode};

    fn parse(value: Value) -> Result<Config> {
        Config::from_ts_json(&value, "rpp.config.ts")
    }

    fn message(value: Value) -> String {
        parse(value).unwrap_err().to_string()
    }

    #[test]
    fn maps_camel_case_schema() {
        let config = parse(json!({
            "pack": { "name": "demo", "description": "d", "packFormat": 34 },
            "build": {
                "source": "src",
                "workers": 2,
                "limits": { "memoryLimitMb": 64, "executionDeadlineSeconds": 5 },
                "wasm": { "memoryLimitMb": 128 },
                "squash": { "png": "max", "packsquashBinary": "ps" }
            },
            "dev": { "port": 9000, "open": true },
            "plugins": [
                { "plugin": "window", "options": { "someKey": { "innerKey": 1 } } },
                {
                    "plugin": "tools",
                    "security": "trusted",
                    "permissions": { "process": ["git"], "network": true },
                    "outputs": { "docs": "out/docs" }
                }
            ]
        }))
        .unwrap();
        assert_eq!(config.pack.pack_format, Some(34));
        assert_eq!(config.build.workers, 2);
        assert_eq!(config.build.lua.memory_limit_mb, 64);
        assert_eq!(config.build.lua.execution_deadline_seconds, 5);
        assert_eq!(config.build.wasm.memory_limit_mb, 128);
        assert_eq!(config.build.squash.png, PngSetting::Max);
        assert_eq!(config.build.squash.packsquash_binary, "ps");
        assert_eq!(config.dev.port, 9000);
        assert_eq!(config.plugins[0].package.as_deref(), Some("window"));
        assert_eq!(config.plugins[0].label(), "window");
        assert_eq!(
            config.plugins[0].options["someKey"]["innerKey"].as_integer(),
            Some(1)
        );
        assert_eq!(config.plugins[1].security, SecurityMode::Trusted);
        assert_eq!(config.plugins[1].permissions.process, ["git"]);
        assert!(config.plugins[1].permissions.network);
        assert!(config.plugins[1].outputs.contains_key("docs"));
    }

    #[test]
    fn rejects_unsupported_values() {
        let pack = json!({ "name": "demo" });
        assert!(message(json!({ "pack": pack, "build": { "lua": {} } })).contains("`build.lua`"));
        assert!(message(
            json!({ "pack": pack, "plugins": [{ "plugin": "a", "security": "native" }] })
        )
        .contains("native"));
        assert!(message(
            json!({ "pack": pack, "plugins": [{ "plugin": "a", "options": { "k": null } }] })
        )
        .contains("must not be null"));
        assert!(
            message(json!({ "pack": pack, "plugins": [{ "options": {} }] }))
                .contains("must set `plugin`")
        );
        assert!(
            message(json!({ "pack": { "name": "demo", "packFromat": 1 } }))
                .contains("`packFromat`")
        );
        assert!(
            message(json!({ "pack": pack, "build": { "limits": { "memoryLimitMb": 0 } } }))
                .contains("build.limits.memoryLimitMb")
        );
    }
}
