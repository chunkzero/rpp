//! Semantic validation of a deserialized [`Config`].

use std::path::{Component, Path};

use serde_json::Value;

use super::{Config, FormatRange, PackConfig, PluginConfig, SecurityMode};
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
        self.pack.validate().map_err(&fail)?;
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

    /// Reject a source `pack.mcmeta`, which would conflict with the generated one.
    pub(crate) fn validate_source(&self, project_root: &Path) -> Result<()> {
        let source = self.build.source.join("pack.mcmeta");
        if project_root.join(&source).exists() {
            return Err(config_error(
                &project_root.join("rpp.config.ts"),
                format!(
                    "`{}` is not supported; rpp generates `pack.mcmeta` from `pack`, so move its \
                     fields into rpp.config.ts and delete the file",
                    source.display()
                ),
            ));
        }
        Ok(())
    }
}

impl PackConfig {
    fn validate(&self) -> std::result::Result<(), String> {
        if !matches!(
            self.description,
            Value::String(_) | Value::Array(_) | Value::Object(_)
        ) {
            return Err("`pack.description` must be a string, array, or object".into());
        }
        validate_format(self.format, "pack.format")?;
        for (index, overlay) in self.overlays.iter().enumerate() {
            let at = format!("pack.overlays[{index}]");
            let directory = &overlay.directory;
            let valid = !directory.is_empty()
                && directory
                    .bytes()
                    .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.'));
            if !valid || directory == "." || directory == ".." {
                return Err(format!(
                    "`{at}.directory` must be a directory name of `a-z0-9_.-` characters"
                ));
            }
            validate_format(overlay.format, &format!("{at}.format"))?;
        }
        Ok(())
    }
}

fn validate_format(range: FormatRange, at: &str) -> std::result::Result<(), String> {
    if range.min == 0 {
        return Err(format!("`{at}` must be greater than 0"));
    }
    if range.min > range.max {
        return Err(format!("`{at}.min` must not exceed `{at}.max`"));
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_pack_mcmeta_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let config = Config::new("x", 34);
        std::fs::create_dir_all(root.join("src")).unwrap();
        config.validate_source(root).unwrap();
        std::fs::write(root.join("src/pack.mcmeta"), "{}").unwrap();
        let message = config.validate_source(root).unwrap_err().to_string();
        assert!(message.contains("rpp generates `pack.mcmeta`"), "{message}");
    }

    #[test]
    fn pack_metadata_rejected() {
        let config = |edit: fn(&mut PackConfig)| {
            let mut config = Config::new("x", 34);
            edit(&mut config.pack);
            config
                .validate(Path::new("rpp.config.ts"))
                .unwrap_err()
                .to_string()
        };
        assert!(config(|pack| pack.description = 1.into()).contains("pack.description"));
        assert!(config(|pack| pack.format.min = 0).contains("greater than 0"));
        assert!(config(|pack| pack.format.max = 1).contains("must not exceed"));
        let message = config(|pack| {
            pack.overlays.push(super::super::OverlayConfig {
                directory: "Legacy".into(),
                format: FormatRange { min: 34, max: 34 },
            })
        });
        assert!(message.contains("pack.overlays[0].directory"), "{message}");
    }
}
