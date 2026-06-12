//! Conversions between guest bindings and public host types.

use crate::bindings::guest;
use crate::error::{Error, Result};
use crate::types::{PluginInfo, ProcessResult, ProcessorDef};

pub(crate) fn convert_info(info: guest::PluginInfo) -> PluginInfo {
    PluginInfo {
        id: info.id,
        version: info.version,
        processors: info
            .processors
            .into_iter()
            .map(|p| ProcessorDef {
                name: p.name,
                patterns: p.patterns,
                priority: p.priority,
            })
            .collect(),
        has_generator: info.has_generator,
    }
}

pub(crate) fn convert_process_result(r: guest::ProcessResult) -> ProcessResult {
    match r {
        guest::ProcessResult::Unchanged => ProcessResult::Unchanged,
        guest::ProcessResult::Modified(f) => ProcessResult::Modified {
            path: f.path,
            contents: f.contents,
        },
        guest::ProcessResult::Dropped => ProcessResult::Dropped,
    }
}

/// Validate a plugin id (`^[a-z0-9][a-z0-9_-]*$`) and a semver version.
pub(crate) fn validate_plugin_info(info: &PluginInfo) -> Result<()> {
    if !is_valid_id(&info.id) {
        return Err(Error::InvalidInfo(format!(
            "plugin id {:?} does not match ^[a-z0-9][a-z0-9_-]*$",
            info.id
        )));
    }
    if semver::Version::parse(&info.version).is_err() {
        return Err(Error::InvalidInfo(format!(
            "plugin version {:?} is not valid semver",
            info.version
        )));
    }
    Ok(())
}

fn is_valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}
