//! Conversion of the `rpp.config.ts` JSON value into [`Config`].

use std::path::PathBuf;

use serde_json::Value;

use super::validate::reject_nulls;
use super::Config;
use crate::error::{Error, Result};

const REJECTED_PLUGIN_KEYS: [&str; 4] = ["id", "source", "ref", "subdir"];

pub(super) fn from_json(value: &Value, path: PathBuf) -> Result<Config> {
    let fail = |message: String| Error::Config {
        path: path.clone(),
        message,
    };
    reject_nulls(value, "config").map_err(&fail)?;
    reject_wrong_shapes(value).map_err(&fail)?;
    reject_legacy_keys(value).map_err(&fail)?;
    let config: Config =
        serde_path_to_error::deserialize(value).map_err(|e| fail(e.to_string()))?;
    config.validate(&path)?;
    Ok(config)
}

/// Appends the migration guide to a rejection of a key from the removed TOML schema.
fn guided(message: &str) -> String {
    format!("{message}; see {}", crate::MIGRATION_GUIDE)
}

/// Serde derives also read arrays as positional structs, so objects are required explicitly.
fn reject_wrong_shapes(value: &Value) -> std::result::Result<(), String> {
    let root = value.as_object().ok_or("the config must be an object")?;
    if root.get("build").is_some_and(|build| !build.is_object()) {
        return Err("`build` must be an object".into());
    }
    let plugins = match root.get("plugins") {
        None => return Ok(()),
        Some(Value::Array(plugins)) => plugins,
        Some(_) => return Err("`plugins` must be an array".into()),
    };
    match plugins.iter().position(|plugin| !plugin.is_object()) {
        Some(index) => Err(format!("`plugins[{index}]` must be an object")),
        None => Ok(()),
    }
}

fn reject_legacy_keys(value: &Value) -> std::result::Result<(), String> {
    if value.pointer("/build/lua").is_some() {
        return Err(guided("`build.lua` is not supported; use `build.limits`"));
    }
    let plugins = value.get("plugins").and_then(Value::as_array);
    for (index, plugin) in plugins.into_iter().flatten().enumerate() {
        let at = format!("plugins[{index}]");
        if plugin.get("security").and_then(Value::as_str) == Some("native") {
            return Err(guided(&format!("`{at}.security` must not be \"native\"")));
        }
        if plugin.pointer("/permissions/lua").is_some() {
            return Err(guided(&format!("`{at}.permissions.lua` is not supported")));
        }
        if let Some(key) = REJECTED_PLUGIN_KEYS
            .iter()
            .find(|key| plugin.get(**key).is_some())
        {
            return Err(guided(&format!(
                "`{at}.{key}` is not supported; use `plugin`"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::config::{PngSetting, SecurityMode, SquashEngine};

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
                "squash": { "engine": "packsquash", "png": "max", "packsquashBinary": "ps" }
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
        assert_eq!(config.build.limits.memory_limit_mb, 64);
        assert_eq!(config.build.limits.execution_deadline_seconds, 5);
        assert_eq!(config.build.wasm.memory_limit_mb, 128);
        assert_eq!(config.build.squash.engine, SquashEngine::Packsquash);
        assert_eq!(config.build.squash.png, PngSetting::Max);
        assert_eq!(config.build.squash.packsquash_binary, "ps");
        assert_eq!(config.dev.port, 9000);
        assert_eq!(config.plugins[0].package, "window");
        assert_eq!(
            config.plugins[0].options["someKey"]["innerKey"].as_i64(),
            Some(1)
        );
        assert_eq!(config.plugins[1].options, json!({}));
        assert_eq!(config.plugins[1].security, SecurityMode::Trusted);
        assert_eq!(config.plugins[1].permissions.process, ["git"]);
        assert!(config.plugins[1].permissions.network);
        assert!(config.plugins[1].outputs.contains_key("docs"));
    }

    #[test]
    fn rejects_build_lua_with_guide() {
        let message = message(json!({ "pack": { "name": "demo" }, "build": { "lua": {} } }));
        assert!(message.contains("`build.lua`"), "{message}");
        assert!(message.ends_with(crate::MIGRATION_GUIDE), "{message}");
    }

    #[test]
    fn rejects_native_security_with_guide() {
        let message = message(json!({
            "pack": { "name": "demo" },
            "plugins": [{ "plugin": "a", "security": "native" }]
        }));
        assert!(message.contains("native"), "{message}");
        assert!(message.ends_with(crate::MIGRATION_GUIDE), "{message}");
    }

    #[test]
    fn rejects_positional_arrays() {
        let pack = json!({ "name": "demo" });
        let cases = [
            (json!([{ "name": "demo" }]), "the config must be an object"),
            (
                json!({ "pack": pack, "build": [] }),
                "`build` must be an object",
            ),
            (
                json!({ "pack": pack, "plugins": {} }),
                "`plugins` must be an array",
            ),
            (
                json!({ "pack": pack, "plugins": [{ "plugin": "a" }, ["a"]] }),
                "`plugins[1]` must be an object",
            ),
        ];
        for (value, expected) in cases {
            assert_eq!(
                message(value),
                format!("invalid config rpp.config.ts: {expected}")
            );
        }
    }

    #[test]
    fn reports_full_paths() {
        let pack = json!({ "name": "demo" });
        let cases = [
            (
                json!({ "pack": pack, "build": { "limits": { "memoryLimitMb": "big" } } }),
                "build.limits.memoryLimitMb: invalid type: string \"big\", expected u32",
            ),
            (
                json!({ "pack": pack, "plugins": [{ "plugin": "a" }, { "options": {} }] }),
                "plugins[1]: missing field `plugin`",
            ),
            (
                json!({ "pack": pack, "plugins": [{ "plugin": "a", "security": "root" }] }),
                "plugins[0].security: unknown variant `root`",
            ),
            (
                json!({ "pack": pack, "build": { "squash": { "engine": "zip" } } }),
                "build.squash.engine: unknown variant `zip`, expected `builtin` or `packsquash`",
            ),
        ];
        for (value, expected) in cases {
            let message = message(value);
            assert!(message.contains(expected), "{message}");
        }
    }

    #[test]
    fn rejects_unsupported_values() {
        let pack = json!({ "name": "demo" });
        assert!(message(
            json!({ "pack": pack, "plugins": [{ "plugin": "a", "options": { "k": null } }] })
        )
        .contains("must not be null"));
        assert!(
            message(json!({ "pack": pack, "plugins": [{ "options": {} }] }))
                .contains("missing field `plugin`")
        );
        assert!(message(
            json!({ "pack": pack, "plugins": [{ "plugin": "a", "source": "path:x" }] })
        )
        .contains("`plugins[0].source`"));
        assert!(
            message(json!({ "pack": pack, "plugin": [{ "package": "a" }] }))
                .contains("unknown field `plugin`")
        );
        assert!(
            message(json!({ "pack": { "name": "demo", "packFromat": 1 } }))
                .contains("`packFromat`")
        );
        assert!(
            message(json!({ "pack": { "name": "demo", "pack_format": 1 } }))
                .contains("unknown field `pack_format`")
        );
        assert!(
            message(json!({ "pack": pack, "build": { "limits": { "memoryLimitMb": 0 } } }))
                .contains("build.limits.memoryLimitMb")
        );
    }
}
