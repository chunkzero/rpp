//! Semantic validation of a deserialized [`Config`].

use std::path::{Component, Path};

use serde_json::Value;

use super::{Config, PluginConfig, SecurityMode};
use crate::error::{Error, Result};
use crate::manifest::{is_valid_id, ID_GRAMMAR};

fn config_error(path: &Path, message: String) -> Error {
    Error::Config {
        path: path.to_path_buf(),
        message,
    }
}

impl Config {
    /// Validate everything except the pack source, attributing errors to `path`.
    pub(super) fn validate(&self, path: &Path) -> Result<()> {
        let fail = |message: String| config_error(path, message);
        let name = &self.pack.name;
        if name.trim().is_empty() {
            return Err(fail("`pack.name` must not be empty".into()));
        }
        if name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
            return Err(fail(
                "`pack.name` must be a file-name-safe value without path separators".into(),
            ));
        }
        for plugin in &self.plugins {
            plugin.validate(path)?;
        }
        let limits = &self.build.limits;
        let wasm = &self.build.wasm;
        for (key, value) in [
            (
                "build.limits.memoryLimitMb",
                u64::from(limits.memory_limit_mb),
            ),
            (
                "build.limits.executionDeadlineSeconds",
                limits.execution_deadline_seconds,
            ),
            ("build.wasm.memoryLimitMb", u64::from(wasm.memory_limit_mb)),
            (
                "build.wasm.executionDeadlineSeconds",
                wasm.execution_deadline_seconds,
            ),
        ] {
            if value == 0 {
                return Err(fail(format!("`{key}` must be greater than 0")));
            }
        }
        Ok(())
    }

    /// Check `pack.packFormat` against the source `pack.mcmeta`, when both are present.
    pub(crate) fn validate_source(&self, project_root: &Path) -> Result<()> {
        let path = project_root.join("rpp.config.ts");
        if let Some(expected) = self.pack.pack_format {
            let mcmeta_path = project_root.join(&self.build.source).join("pack.mcmeta");
            if mcmeta_path.is_file() {
                validate_pack_format_mcmeta(&mcmeta_path, expected, &path)?;
            }
        }
        Ok(())
    }
}

impl PluginConfig {
    /// Validate the package name, options, capability policy and output roots, attributing
    /// errors to `path`.
    pub(crate) fn validate(&self, path: &Path) -> Result<()> {
        let package = &self.package;
        let fail = |message: String| config_error(path, message);
        if !is_valid_id(package) {
            return Err(fail(format!(
                "plugin package `{package}` must match {ID_GRAMMAR}"
            )));
        }
        reject_nulls(&self.options, "options").map_err(&fail)?;
        if self.security == SecurityMode::Sandboxed && !self.permissions.is_empty() {
            return Err(fail(format!(
                "plugin `{package}` grants permissions but uses `security: \"sandboxed\"`"
            )));
        }
        for (key, values) in [
            ("permissions.read", &self.permissions.read),
            ("permissions.write", &self.permissions.write),
        ] {
            for value in values {
                validate_project_relative(value)
                    .map_err(|message| fail(format!("plugin `{package}` {key}: {message}")))?;
            }
        }
        for (name, output) in &self.outputs {
            let valid_name = !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
            if !valid_name {
                return Err(fail(format!(
                    "plugin `{package}` has invalid output name `{name}`"
                )));
            }
            validate_project_relative(output).map_err(|message| {
                fail(format!("plugin `{package}` output `{name}`: {message}"))
            })?;
        }
        Ok(())
    }
}

/// Rejects `null` anywhere in `value`, naming its path from `at`.
pub(crate) fn reject_nulls(value: &Value, at: &str) -> std::result::Result<(), String> {
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

fn validate_project_relative(path: &Path) -> std::result::Result<(), &'static str> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err("path must be project-relative");
    }
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_) | Component::ParentDir))
    {
        return Err("path must not contain `.` or prefix components");
    }
    Ok(())
}

fn validate_pack_format_mcmeta(
    mcmeta_path: &Path,
    expected: u32,
    config_path: &Path,
) -> Result<()> {
    let fail = |message: String| config_error(config_path, message);
    let text = std::fs::read_to_string(mcmeta_path).map_err(|e| Error::io(mcmeta_path, e))?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
        fail(format!(
            "`{}` is not valid JSON: {e}",
            mcmeta_path.display()
        ))
    })?;
    let actual = value
        .get("pack")
        .and_then(|pack| pack.get("pack_format"))
        .and_then(|format| format.as_u64());
    match actual {
        Some(actual) if actual == u64::from(expected) => Ok(()),
        Some(actual) => Err(fail(format!(
            "`pack.packFormat` ({expected}) does not match `{}` pack_format ({actual})",
            mcmeta_path.display()
        ))),
        None => Err(fail(format!(
            "`{}` is missing `pack.pack_format` but `pack.packFormat` is set in config",
            mcmeta_path.display()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_format_mismatch_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/pack.mcmeta"),
            r#"{"pack":{"pack_format":9}}"#,
        )
        .unwrap();
        let mut config = Config::new("x");
        config.pack.pack_format = Some(34);
        let msg = config.validate_source(root).unwrap_err().to_string();
        assert!(msg.contains("does not match"), "{msg}");

        std::fs::write(
            root.join("src/pack.mcmeta"),
            r#"{"pack":{"pack_format":34}}"#,
        )
        .unwrap();
        config.validate_source(root).unwrap();
    }
}
