//! Light-weight reading of `plugin.toml` manifests.
//!
//! The full manifest type lives in the core `rpp` crate; this crate only needs
//! the `id` and `version` fields for lockfile bookkeeping and `plugin list`.

use std::path::Path;

use serde::Deserialize;

use crate::error::{Error, Result};

/// The file name of a plugin package manifest.
pub const MANIFEST_FILE: &str = "plugin.toml";

/// The subset of `[plugin]` fields this crate cares about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestSummary {
    /// The plugin id (`^[a-z0-9][a-z0-9_-]*$`, unique within a project).
    pub id: String,
    /// The plugin semver version string.
    pub version: String,
}

#[derive(Deserialize)]
struct RawManifest {
    plugin: RawPlugin,
}

#[derive(Deserialize)]
struct RawPlugin {
    id: String,
    version: String,
}

/// Read just `[plugin] id` and `[plugin] version` from the `plugin.toml` inside
/// `dir`.
///
/// # Errors
///
/// Returns [`Error::MissingManifest`] when no `plugin.toml` exists, or
/// [`Error::InvalidManifest`] when it cannot be parsed or is missing required
/// fields.
pub fn parse_manifest_summary(dir: &Path) -> Result<ManifestSummary> {
    let path = dir.join(MANIFEST_FILE);
    if !path.is_file() {
        return Err(Error::MissingManifest(dir.to_path_buf()));
    }

    let text = std::fs::read_to_string(&path)
        .map_err(|e| Error::io(format!("reading {}", path.display()), e))?;

    let raw: RawManifest = toml::from_str(&text).map_err(|e| Error::InvalidManifest {
        path: path.clone(),
        reason: e.to_string(),
    })?;

    if raw.plugin.id.trim().is_empty() {
        return Err(Error::InvalidManifest {
            path: path.clone(),
            reason: "`[plugin] id` must not be empty".to_string(),
        });
    }
    if raw.plugin.version.trim().is_empty() {
        return Err(Error::InvalidManifest {
            path,
            reason: "`[plugin] version` must not be empty".to_string(),
        });
    }

    Ok(ManifestSummary {
        id: raw.plugin.id,
        version: raw.plugin.version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_id_and_version() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(MANIFEST_FILE),
            "[plugin]\nid = \"json-minify\"\nversion = \"1.2.0\"\nruntime = \"lua\"\n",
        )
        .unwrap();
        let summary = parse_manifest_summary(dir.path()).unwrap();
        assert_eq!(summary.id, "json-minify");
        assert_eq!(summary.version, "1.2.0");
    }

    #[test]
    fn missing_manifest_errors() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            parse_manifest_summary(dir.path()),
            Err(Error::MissingManifest(_))
        ));
    }

    #[test]
    fn invalid_manifest_errors() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(MANIFEST_FILE), "not = valid [toml").unwrap();
        assert!(matches!(
            parse_manifest_summary(dir.path()),
            Err(Error::InvalidManifest { .. })
        ));
    }
}
