//! Parsing and canonicalization of plugin source strings.
//!
//! Two source kinds are supported, matching `docs/SPEC.md` section 1:
//! - `path:<relative-or-absolute-dir>` — a local plugin package directory.
//! - `github:<owner>/<repo>` — a GitHub repository, optionally pinned to a `ref`
//!   and narrowed to a `subdir` within the repo.

use std::path::PathBuf;

use crate::error::{Error, Result};

/// A parsed plugin source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginSource {
    /// A local plugin package directory.
    Path {
        /// The directory the source points at (may be relative).
        dir: PathBuf,
    },
    /// A GitHub-hosted plugin package.
    GitHub {
        /// Repository owner (user or organization).
        owner: String,
        /// Repository name.
        repo: String,
        /// Optional tag/branch/sha to pin. `None` means the default branch.
        ref_: Option<String>,
        /// Optional path within the repository containing the plugin package.
        subdir: Option<String>,
    },
}

impl PluginSource {
    /// Parse a source string together with optional `ref` and `subdir` modifiers.
    ///
    /// The `ref_` and `subdir` arguments correspond to the `ref` and `subdir`
    /// keys in a `[[plugin]]` table; they are only meaningful for `github:`
    /// sources and are rejected for `path:` sources.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidSource`] when the scheme is unknown, the body is
    /// malformed, or modifiers are supplied for a source kind that does not
    /// accept them.
    pub fn parse(s: &str, ref_: Option<&str>, subdir: Option<&str>) -> Result<Self> {
        let trimmed = s.trim();
        let (scheme, body) = trimmed
            .split_once(':')
            .ok_or_else(|| Error::InvalidSource {
                source_str: s.to_string(),
                reason: "missing `path:` or `github:` scheme prefix".to_string(),
            })?;

        match scheme {
            "path" => Self::parse_path(s, body, ref_, subdir),
            "github" => Self::parse_github(s, body, ref_, subdir),
            other => Err(Error::InvalidSource {
                source_str: s.to_string(),
                reason: format!("unknown scheme `{other}` (expected `path` or `github`)"),
            }),
        }
    }

    fn parse_path(s: &str, body: &str, ref_: Option<&str>, subdir: Option<&str>) -> Result<Self> {
        if body.is_empty() {
            return Err(Error::InvalidSource {
                source_str: s.to_string(),
                reason: "path source has an empty directory".to_string(),
            });
        }
        if ref_.is_some() || subdir.is_some() {
            return Err(Error::InvalidSource {
                source_str: s.to_string(),
                reason: "path sources do not accept `ref` or `subdir`".to_string(),
            });
        }
        Ok(PluginSource::Path {
            dir: PathBuf::from(body),
        })
    }

    fn parse_github(s: &str, body: &str, ref_: Option<&str>, subdir: Option<&str>) -> Result<Self> {
        let (owner, repo) = body.split_once('/').ok_or_else(|| Error::InvalidSource {
            source_str: s.to_string(),
            reason: "github source must be `github:<owner>/<repo>`".to_string(),
        })?;

        // Strip a trailing `.git` if present, then validate the segments.
        let repo = repo.strip_suffix(".git").unwrap_or(repo);

        if owner.is_empty() || repo.is_empty() {
            return Err(Error::InvalidSource {
                source_str: s.to_string(),
                reason: "github owner and repo must both be non-empty".to_string(),
            });
        }
        if repo.contains('/') {
            return Err(Error::InvalidSource {
                source_str: s.to_string(),
                reason: "github source must be exactly `github:<owner>/<repo>`".to_string(),
            });
        }
        validate_github_segment(s, "owner", owner)?;
        validate_github_segment(s, "repo", repo)?;

        let ref_ = match ref_ {
            Some(r) if r.trim().is_empty() => {
                return Err(Error::InvalidSource {
                    source_str: s.to_string(),
                    reason: "`ref` must not be empty".to_string(),
                });
            }
            Some(r) => Some(r.trim().to_string()),
            None => None,
        };

        let subdir = match subdir {
            Some(d) if d.trim().is_empty() => None,
            Some(d) => Some(normalize_subdir(s, d)?),
            None => None,
        };

        Ok(PluginSource::GitHub {
            owner: owner.to_string(),
            repo: repo.to_string(),
            ref_,
            subdir,
        })
    }

    /// The canonical source string (the value stored in the lockfile `source`
    /// field). For path sources this is `path:<dir>`; for github sources it is
    /// `github:<owner>/<repo>` (ref/subdir are stored separately).
    pub fn canonical(&self) -> String {
        match self {
            PluginSource::Path { dir } => format!("path:{}", dir.display()),
            PluginSource::GitHub { owner, repo, .. } => format!("github:{owner}/{repo}"),
        }
    }
}

/// GitHub owner/repo segments allow alphanumerics plus `-`, `_`, and `.`.
fn validate_github_segment(s: &str, kind: &str, segment: &str) -> Result<()> {
    let valid = segment
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if !valid {
        return Err(Error::InvalidSource {
            source_str: s.to_string(),
            reason: format!("github {kind} `{segment}` contains invalid characters"),
        });
    }
    Ok(())
}

/// Normalize a subdir: forward slashes, no leading/trailing slash, and reject
/// any `..` component so it can never escape the extracted repo root.
fn normalize_subdir(s: &str, subdir: &str) -> Result<String> {
    let cleaned = subdir.replace('\\', "/");
    let parts: Vec<&str> = cleaned
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();

    if parts.contains(&"..") {
        return Err(Error::InvalidSource {
            source_str: s.to_string(),
            reason: "`subdir` must not contain `..` components".to_string(),
        });
    }
    Ok(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_path_source() {
        let src = PluginSource::parse("path:plugins/json-minify", None, None).unwrap();
        assert_eq!(
            src,
            PluginSource::Path {
                dir: PathBuf::from("plugins/json-minify")
            }
        );
        assert_eq!(src.canonical(), "path:plugins/json-minify");
    }

    #[test]
    fn parses_absolute_path_source() {
        let src = PluginSource::parse("path:/abs/dir", None, None).unwrap();
        assert_eq!(
            src,
            PluginSource::Path {
                dir: PathBuf::from("/abs/dir")
            }
        );
    }

    #[test]
    fn parses_github_source_bare() {
        let src = PluginSource::parse("github:example/rpp-plugins", None, None).unwrap();
        assert_eq!(
            src,
            PluginSource::GitHub {
                owner: "example".into(),
                repo: "rpp-plugins".into(),
                ref_: None,
                subdir: None,
            }
        );
        assert_eq!(src.canonical(), "github:example/rpp-plugins");
    }

    #[test]
    fn parses_github_with_ref_and_subdir() {
        let src = PluginSource::parse(
            "github:example/rpp-plugins",
            Some("v1.2.0"),
            Some("plugins/atlas/"),
        )
        .unwrap();
        assert_eq!(
            src,
            PluginSource::GitHub {
                owner: "example".into(),
                repo: "rpp-plugins".into(),
                ref_: Some("v1.2.0".into()),
                subdir: Some("plugins/atlas".into()),
            }
        );
    }

    #[test]
    fn strips_dot_git_suffix() {
        let src = PluginSource::parse("github:example/rpp-plugins.git", None, None).unwrap();
        match src {
            PluginSource::GitHub { repo, .. } => assert_eq!(repo, "rpp-plugins"),
            _ => panic!("expected github"),
        }
    }

    #[test]
    fn rejects_missing_scheme() {
        assert!(PluginSource::parse("plugins/foo", None, None).is_err());
    }

    #[test]
    fn rejects_unknown_scheme() {
        assert!(PluginSource::parse("gitlab:foo/bar", None, None).is_err());
    }

    #[test]
    fn rejects_empty_path() {
        assert!(PluginSource::parse("path:", None, None).is_err());
    }

    #[test]
    fn rejects_path_with_ref() {
        assert!(PluginSource::parse("path:dir", Some("main"), None).is_err());
    }

    #[test]
    fn rejects_github_without_repo() {
        assert!(PluginSource::parse("github:example", None, None).is_err());
        assert!(PluginSource::parse("github:example/", None, None).is_err());
        assert!(PluginSource::parse("github:/repo", None, None).is_err());
    }

    #[test]
    fn rejects_github_with_extra_path_segment() {
        assert!(PluginSource::parse("github:a/b/c", None, None).is_err());
    }

    #[test]
    fn rejects_github_bad_chars() {
        assert!(PluginSource::parse("github:ex ample/repo", None, None).is_err());
    }

    #[test]
    fn rejects_subdir_traversal() {
        assert!(PluginSource::parse("github:example/repo", None, Some("../escape")).is_err());
    }

    #[test]
    fn empty_subdir_becomes_none() {
        let src = PluginSource::parse("github:a/b", None, Some("  ")).unwrap();
        match src {
            PluginSource::GitHub { subdir, .. } => assert_eq!(subdir, None),
            _ => panic!("expected github"),
        }
    }
}
