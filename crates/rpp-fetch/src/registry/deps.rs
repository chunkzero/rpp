//! Parsing of the `dependencies` object in a project's `rpp.json`.

use std::fmt;
use std::path::PathBuf;

use semver::{Version, VersionReq};
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer};

use crate::error::{Error, Result};

const MAX_NAME_LEN: usize = 64;

fn invalid(name: &str, spec: &str, reason: impl Into<String>) -> Error {
    Error::InvalidDependency {
        name: name.to_string(),
        spec: spec.to_string(),
        reason: reason.into(),
    }
}

/// Where a dependency comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencySpec {
    /// A registry version range. A bare version (`0.1.4`) means exactly that version;
    /// operators (`^0.1`, `~0.1.2`, `>=0.1, <0.3`) follow semver.
    Registry(VersionReq),
    /// `path:<dir>`, relative to the project root unless absolute. Never locked.
    Path(PathBuf),
}

impl DependencySpec {
    /// Parse one dependency value.
    ///
    /// # Errors
    ///
    /// [`crate::Error::InvalidDependency`] (with `name`) for an empty `path:`, an
    /// invalid range, or `*`/empty ranges (a range must be explicit).
    pub fn parse(name: &str, spec: &str) -> Result<Self> {
        if let Some(dir) = spec.strip_prefix("path:") {
            if dir.is_empty() {
                return Err(invalid(name, spec, "`path:` needs a directory"));
            }
            return Ok(Self::Path(PathBuf::from(dir)));
        }
        let trimmed = spec.trim();
        if trimmed.is_empty() || trimmed == "*" {
            return Err(invalid(name, spec, "a version range must be explicit"));
        }
        if let Ok(version) = Version::parse(trimmed) {
            let exact = VersionReq::parse(&format!("={version}"))
                .map_err(|e| invalid(name, spec, e.to_string()))?;
            return Ok(Self::Registry(exact));
        }
        VersionReq::parse(trimmed)
            .map(Self::Registry)
            .map_err(|e| invalid(name, spec, e.to_string()))
    }
}

/// One `"name": "spec"` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    /// Plugin name; must equal the name in the package's `rpp.json`.
    pub name: String,
    /// The parsed spec.
    pub spec: DependencySpec,
    /// The spec exactly as written, recorded in the lock as `requested`.
    pub raw: String,
}

/// Parse the `dependencies` object of an `rpp.json` document, in document order.
/// A missing `dependencies` key means no dependencies.
///
/// # Errors
///
/// [`crate::Error::InvalidManifest`] (with `path` `rpp.json`) when the text is not a
/// JSON object or `dependencies` is not an object of strings;
/// [`crate::Error::InvalidDependency`] for invalid names or specs.
pub fn parse_dependencies(manifest_json: &str) -> Result<Vec<Dependency>> {
    let manifest_error = |reason: String| Error::InvalidManifest {
        path: "rpp.json".into(),
        reason,
    };
    let value: serde_json::Value =
        serde_json::from_str(manifest_json).map_err(|e| manifest_error(e.to_string()))?;
    if !value.is_object() {
        return Err(manifest_error("expected a JSON object".to_string()));
    }
    let manifest: Manifest =
        serde_json::from_str(manifest_json).map_err(|e| manifest_error(e.to_string()))?;

    manifest
        .dependencies
        .unwrap_or_default()
        .0
        .into_iter()
        .map(|(name, raw)| {
            validate_name(&name)?;
            let spec = DependencySpec::parse(&name, &raw)?;
            Ok(Dependency { name, spec, raw })
        })
        .collect()
}

#[derive(Deserialize)]
struct Manifest {
    dependencies: Option<OrderedStrings>,
}

/// A JSON object of string values, in document order.
#[derive(Default)]
struct OrderedStrings(Vec<(String, String)>);

impl<'de> Deserialize<'de> for OrderedStrings {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct PairsVisitor;

        impl<'de> Visitor<'de> for PairsVisitor {
            type Value = OrderedStrings;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("`dependencies` as an object of strings")
            }

            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut pairs = Vec::new();
                while let Some(pair) = map.next_entry::<String, String>()? {
                    pairs.push(pair);
                }
                Ok(OrderedStrings(pairs))
            }
        }

        deserializer.deserialize_map(PairsVisitor)
    }
}

/// Validate a plugin name: `^[a-z0-9][a-z0-9_-]*$`, at most 64 characters.
///
/// # Errors
///
/// [`crate::Error::InvalidDependency`] with an empty `spec`.
pub fn validate_name(name: &str) -> Result<()> {
    let mut chars = name.chars();
    let valid_start = chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    let valid_rest =
        chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if !valid_start || !valid_rest {
        return Err(invalid(name, "", "names must match [a-z0-9][a-z0-9_-]*"));
    }
    if name.len() > MAX_NAME_LEN {
        return Err(invalid(
            name,
            "",
            format!("names are at most {MAX_NAME_LEN} characters"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(text: &str) -> DependencySpec {
        DependencySpec::Registry(VersionReq::parse(text).unwrap())
    }

    #[test]
    fn bare_version_is_exact() {
        assert_eq!(DependencySpec::parse("a", "0.1.4").unwrap(), req("=0.1.4"));
    }

    #[test]
    fn caret_range_parses() {
        assert_eq!(DependencySpec::parse("a", "^0.1").unwrap(), req("^0.1"));
        assert_eq!(
            DependencySpec::parse("a", ">=0.1, <0.3").unwrap(),
            req(">=0.1, <0.3")
        );
    }

    #[test]
    fn path_spec() {
        assert_eq!(
            DependencySpec::parse("a", "path:../local").unwrap(),
            DependencySpec::Path(PathBuf::from("../local"))
        );
        assert!(DependencySpec::parse("a", "path:").is_err());
    }

    #[test]
    fn rejects_wildcard_and_bad_names() {
        assert!(DependencySpec::parse("a", "*").is_err());
        assert!(DependencySpec::parse("a", "").is_err());
        assert!(validate_name("window-2_x").is_ok());
        for bad in ["", "-a", "A", "a b", "a/b", &"a".repeat(65)] {
            assert!(validate_name(bad).is_err(), "{bad}");
        }
        assert!(parse_dependencies(r#"{"dependencies": {"Bad": "1.0.0"}}"#).is_err());
    }

    #[test]
    fn keeps_document_order() {
        let deps = parse_dependencies(
            r#"{"name": "x", "dependencies": {"zeta": "1.0.0", "alpha": "^1", "mid": "path:m"}}"#,
        )
        .unwrap();
        let names: Vec<_> = deps.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["zeta", "alpha", "mid"]);
        assert_eq!(deps[1].raw, "^1");
    }

    #[test]
    fn missing_dependencies_is_empty() {
        assert!(parse_dependencies("{}").unwrap().is_empty());
        assert!(matches!(
            parse_dependencies("[]"),
            Err(Error::InvalidManifest { .. })
        ));
        assert!(parse_dependencies(r#"{"dependencies": {"a": 1}}"#).is_err());
    }
}
