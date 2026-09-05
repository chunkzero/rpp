//! Parsing of the `source` grammar for `[[plugin]]` entries (spec §1).

use std::path::PathBuf;

use thiserror::Error;

/// A structured plugin source descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginSourceSpec {
    /// A local plugin package directory (`path:<dir>`).
    Path {
        /// The directory containing `plugin.toml`.
        dir: PathBuf,
    },
    /// A GitHub repository (`github:<owner>/<repo>`).
    GitHub {
        /// Repository owner.
        owner: String,
        /// Repository name.
        repo: String,
        /// Optional git ref (tag/branch/sha).
        r#ref: Option<String>,
        /// Optional subdirectory within the repo.
        subdir: Option<String>,
    },
}

/// Error returned when a plugin `source` string cannot be parsed.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SourceParseError {
    /// The `source` had no recognized scheme prefix.
    #[error("missing scheme; expected `path:` or `github:`")]
    MissingScheme,
    /// A `path:` source had an empty directory.
    #[error("empty path")]
    EmptyPath,
    /// A `github:` source was not in `owner/repo` form.
    #[error("expected `github:owner/repo`")]
    BadGitHub,
}

impl PluginSourceSpec {
    /// Parse a source string with optional `ref`/`subdir` overrides.
    pub fn parse(
        source: &str,
        r#ref: Option<&str>,
        subdir: Option<&str>,
    ) -> Result<Self, SourceParseError> {
        if let Some(rest) = source.strip_prefix("path:") {
            let rest = rest.trim();
            if rest.is_empty() {
                return Err(SourceParseError::EmptyPath);
            }
            return Ok(PluginSourceSpec::Path {
                dir: PathBuf::from(rest),
            });
        }
        if let Some(rest) = source.strip_prefix("github:") {
            let rest = rest.trim();
            let (owner, repo) = rest.split_once('/').ok_or(SourceParseError::BadGitHub)?;
            if owner.is_empty() || repo.is_empty() || repo.contains('/') {
                return Err(SourceParseError::BadGitHub);
            }
            return Ok(PluginSourceSpec::GitHub {
                owner: owner.to_string(),
                repo: repo.to_string(),
                r#ref: r#ref.map(str::to_string),
                subdir: subdir.map(str::to_string),
            });
        }
        Err(SourceParseError::MissingScheme)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_path() {
        let s = PluginSourceSpec::parse("path:plugins/foo", None, None).unwrap();
        assert_eq!(
            s,
            PluginSourceSpec::Path {
                dir: PathBuf::from("plugins/foo")
            }
        );
    }

    #[test]
    fn parses_github() {
        let s = PluginSourceSpec::parse("github:a/b", Some("v1"), Some("sub")).unwrap();
        assert_eq!(
            s,
            PluginSourceSpec::GitHub {
                owner: "a".into(),
                repo: "b".into(),
                r#ref: Some("v1".into()),
                subdir: Some("sub".into()),
            }
        );
    }

    #[test]
    fn rejects_unknown_scheme() {
        assert_eq!(
            PluginSourceSpec::parse("git:a/b", None, None),
            Err(SourceParseError::MissingScheme)
        );
    }

    #[test]
    fn rejects_bad_github() {
        assert_eq!(
            PluginSourceSpec::parse("github:justowner", None, None),
            Err(SourceParseError::BadGitHub)
        );
    }
}
