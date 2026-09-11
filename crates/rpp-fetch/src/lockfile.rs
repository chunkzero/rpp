//! The `rpp.lock` lockfile: a TOML record of resolved GitHub plugin pins.
//!
//! Format (per `docs/SPEC.md` section 6):
//!
//! ```toml
//! version = 2
//! [[plugin]]
//! source = "github:example/rpp-plugins"
//! ref = "v1.2.0"
//! requested_ref = "v1.2.0"
//! commit = "<full sha>"
//! subdir = "plugins/atlas"
//! ```
//!
//! Path sources are never locked. Entries are keyed by their canonical source
//! string, requested ref, and subdir, and serialized in a stable (sorted) order so the file is diff-stable.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::source::PluginSource;

/// The current lockfile schema version this crate writes and accepts.
pub const LOCKFILE_VERSION: u32 = 2;

/// A single locked GitHub plugin pin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockedPlugin {
    /// Canonical source string, e.g. `github:example/rpp-plugins`.
    pub source: String,
    /// The resolved ref (a tag/branch/sha, or the default branch name).
    pub ref_: String,
    /// The requested ref; absent for default-branch requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_ref: Option<String>,
    /// The full commit SHA the ref resolved to.
    pub commit: String,
    /// Optional subdir within the repository.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub subdir: Option<String>,
}

/// The in-memory representation of an `rpp.lock` file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lockfile {
    plugins: Vec<LockedPlugin>,
}

/// Wire format mirroring the on-disk TOML. `ref_` maps to `ref`.
#[derive(Serialize, Deserialize)]
struct RawLockfile {
    version: u32,
    #[serde(default, rename = "plugin")]
    plugins: Vec<RawLockedPlugin>,
}

#[derive(Serialize, Deserialize)]
struct RawLockedPlugin {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    requested_ref: Option<String>,
    source: String,
    #[serde(rename = "ref")]
    ref_: String,
    commit: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    subdir: Option<String>,
}

impl From<RawLockedPlugin> for LockedPlugin {
    fn from(r: RawLockedPlugin) -> Self {
        LockedPlugin {
            source: r.source,
            requested_ref: r.requested_ref,
            ref_: r.ref_,
            commit: r.commit,
            subdir: r.subdir,
        }
    }
}

impl From<&LockedPlugin> for RawLockedPlugin {
    fn from(l: &LockedPlugin) -> Self {
        RawLockedPlugin {
            source: l.source.clone(),
            ref_: l.ref_.clone(),
            requested_ref: l.requested_ref.clone(),
            commit: l.commit.clone(),
            subdir: l.subdir.clone(),
        }
    }
}

impl Lockfile {
    /// Create an empty lockfile.
    pub fn new() -> Self {
        Lockfile::default()
    }

    /// Load a lockfile from `path`. A missing file yields an empty lockfile.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidLockfile`] when the file exists but cannot be
    /// parsed, or [`Error::UnsupportedLockVersion`] when its `version` is newer
    /// than [`LOCKFILE_VERSION`].
    pub fn load(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Lockfile::new()),
            Err(e) => return Err(Error::io(format!("reading {}", path.display()), e)),
        };

        let raw: RawLockfile = toml::from_str(&text).map_err(|e| Error::InvalidLockfile {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })?;

        if raw.version > LOCKFILE_VERSION {
            return Err(Error::UnsupportedLockVersion {
                found: raw.version,
                supported: LOCKFILE_VERSION,
            });
        }

        let mut plugins: Vec<LockedPlugin> = raw
            .plugins
            .into_iter()
            .map(|mut pin| {
                if raw.version < 2 {
                    pin.requested_ref = Some(pin.ref_.clone());
                }
                pin.into()
            })
            .collect();
        plugins.sort_by(lock_order);
        Ok(Lockfile { plugins })
    }

    /// Serialize the lockfile to a stable TOML string (sorted entries, trailing
    /// newline).
    pub fn to_toml(&self) -> Result<String> {
        let mut plugins: Vec<&LockedPlugin> = self.plugins.iter().collect();
        plugins.sort_by(|a, b| lock_order(a, b));

        let raw = RawLockfile {
            version: LOCKFILE_VERSION,
            plugins: plugins.iter().map(|p| RawLockedPlugin::from(*p)).collect(),
        };

        let mut out = toml::to_string_pretty(&raw).map_err(|e| Error::InvalidLockfile {
            path: Path::new("rpp.lock").to_path_buf(),
            reason: e.to_string(),
        })?;
        if !out.ends_with('\n') {
            out.push('\n');
        }
        Ok(out)
    }

    /// Write the lockfile to `path` with stable ordering and a trailing newline.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the file cannot be written.
    pub fn save(&self, path: &Path) -> Result<()> {
        let text = self.to_toml()?;
        std::fs::write(path, text).map_err(|e| Error::io(format!("writing {}", path.display()), e))
    }

    /// Drop pins whose source, requested ref, and subdir no longer match any
    /// configured plugin. Returns whether anything was removed.
    pub fn prune(&mut self, configured: &[PluginSource]) -> bool {
        let before = self.plugins.len();
        self.plugins.retain(|pin| {
            configured.iter().any(|source| {
                source.canonical() == pin.source
                    && source.requested_ref() == pin.requested_ref.as_deref()
                    && source.subdir() == pin.subdir.as_deref()
            })
        });
        self.plugins.len() != before
    }

    /// Look up the first locked pin by its canonical source string.
    ///
    /// Prefer [`get_for`](Self::get_for) when a source may have multiple pins
    /// (distinct refs or subdirs). This returns only the first match in stable
    /// sort order.
    pub fn get(&self, source_str: &str) -> Option<&LockedPlugin> {
        self.plugins.iter().find(|p| p.source == source_str)
    }

    /// Look up a pin matching a configured source, requested ref, and subdir.
    pub fn get_for(
        &self,
        source: &str,
        requested_ref: Option<&str>,
        subdir: Option<&str>,
    ) -> Option<&LockedPlugin> {
        self.plugins.iter().find(|plugin| {
            plugin.source == source
                && plugin.subdir.as_deref() == subdir
                && plugin.requested_ref.as_deref() == requested_ref
        })
    }

    /// Insert or replace the pin for a source. Returns the previous pin, if any.
    pub fn upsert(&mut self, plugin: LockedPlugin) -> Option<LockedPlugin> {
        match self.plugins.iter_mut().find(|existing| {
            existing.source == plugin.source
                && existing.requested_ref == plugin.requested_ref
                && existing.subdir == plugin.subdir
        }) {
            Some(existing) => Some(std::mem::replace(existing, plugin)),
            None => {
                self.plugins.push(plugin);
                self.plugins.sort_by(lock_order);
                None
            }
        }
    }

    /// Remove the pin matching `source`, `requested_ref`, and `subdir`.
    ///
    /// `requested_ref` follows the same semantics as [`get_for`](Self::get_for):
    /// `None` matches only default-branch requests.
    pub fn remove_for(
        &mut self,
        source: &str,
        requested_ref: Option<&str>,
        subdir: Option<&str>,
    ) -> Option<LockedPlugin> {
        let idx = self.plugins.iter().position(|plugin| {
            plugin.source == source
                && plugin.subdir.as_deref() == subdir
                && plugin.requested_ref.as_deref() == requested_ref
        })?;
        Some(self.plugins.remove(idx))
    }

    /// All locked pins, in stable (sorted) order.
    pub fn plugins(&self) -> &[LockedPlugin] {
        &self.plugins
    }
}

fn lock_order(a: &LockedPlugin, b: &LockedPlugin) -> std::cmp::Ordering {
    a.source
        .cmp(&b.source)
        .then(a.requested_ref.cmp(&b.requested_ref))
        .then(a.subdir.cmp(&b.subdir))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin(source: &str, ref_: &str, commit: &str, subdir: Option<&str>) -> LockedPlugin {
        LockedPlugin {
            source: source.to_string(),
            ref_: ref_.to_string(),
            requested_ref: Some(ref_.to_string()),
            commit: commit.to_string(),
            subdir: subdir.map(str::to_string),
        }
    }

    #[test]
    fn default_request_is_distinct_from_explicit_branch() {
        let mut lock = Lockfile::new();
        let mut default = pin("github:a/b", "main", "default", None);
        default.requested_ref = None;
        lock.upsert(default.clone());
        lock.upsert(pin("github:a/b", "main", "explicit", None));
        assert_eq!(
            lock.get_for("github:a/b", None, None).unwrap().commit,
            "default"
        );
        assert_eq!(
            lock.get_for("github:a/b", Some("main"), None)
                .unwrap()
                .commit,
            "explicit"
        );
        default.ref_ = "next-default".into();
        default.commit = "new-default".into();
        assert_eq!(lock.upsert(default).unwrap().commit, "default");
        assert_eq!(lock.plugins().len(), 2);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rpp.lock");
        lock.save(&path).unwrap();
        let mut loaded = Lockfile::load(&path).unwrap();
        assert_eq!(loaded, lock);
        loaded.remove_for("github:a/b", None, None).unwrap();
        assert!(loaded.get_for("github:a/b", None, None).is_none());
        assert_eq!(loaded.plugins()[0].commit, "explicit");
    }

    #[test]
    fn legacy_pins_are_explicit_requests() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rpp.lock");
        std::fs::write(
            &path,
            "version = 1\n[[plugin]]\nsource = \"github:a/b\"\nref = \"main\"\ncommit = \"old\"\n",
        )
        .unwrap();
        let lock = Lockfile::load(&path).unwrap();
        assert!(lock.get_for("github:a/b", None, None).is_none());
        assert_eq!(
            lock.get_for("github:a/b", Some("main"), None)
                .unwrap()
                .commit,
            "old"
        );
    }

    #[test]
    fn prune_drops_pins_for_unconfigured_requests() {
        let mut lock = Lockfile::new();
        let mut default = pin("github:a/b", "main", "default", None);
        default.requested_ref = None;
        lock.upsert(default);
        lock.upsert(pin("github:a/b", "main", "legacy", None));
        lock.upsert(pin("github:a/b", "v1", "tagged", Some("sub")));
        let configured = [
            PluginSource::parse("github:a/b", None, None).unwrap(),
            PluginSource::parse("github:a/b", Some("v1"), Some("sub")).unwrap(),
            PluginSource::parse("path:local", None, None).unwrap(),
        ];
        assert!(lock.prune(&configured));
        assert_eq!(
            lock.plugins()
                .iter()
                .map(|pin| pin.commit.as_str())
                .collect::<Vec<_>>(),
            ["default", "tagged"]
        );
        assert!(!lock.prune(&configured));
    }

    #[test]
    fn missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let lock = Lockfile::load(&dir.path().join("rpp.lock")).unwrap();
        assert!(lock.plugins().is_empty());
    }

    #[test]
    fn round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rpp.lock");

        let mut lock = Lockfile::new();
        lock.upsert(pin(
            "github:example/rpp-plugins",
            "v1.2.0",
            "abc123",
            Some("plugins/atlas"),
        ));
        lock.upsert(pin("github:a/b", "main", "deadbeef", None));
        lock.save(&path).unwrap();

        let loaded = Lockfile::load(&path).unwrap();
        assert_eq!(loaded, lock);
        // Stable sorted order: `github:a/b` precedes `github:example/...`.
        assert_eq!(loaded.plugins()[0].source, "github:a/b");
    }

    #[test]
    fn save_has_trailing_newline_and_version() {
        let mut lock = Lockfile::new();
        lock.upsert(pin("github:a/b", "main", "sha", None));
        let text = lock.to_toml().unwrap();
        assert!(text.ends_with('\n'));
        assert!(text.contains("version = 2"));
        assert!(text.contains("ref = \"main\""));
    }

    #[test]
    fn upsert_replaces() {
        let mut lock = Lockfile::new();
        lock.upsert(pin("github:a/b", "main", "old", None));
        let prev = lock.upsert(pin("github:a/b", "main", "new", None));
        assert_eq!(prev.unwrap().commit, "old");
        assert_eq!(lock.get("github:a/b").unwrap().commit, "new");
        assert_eq!(lock.plugins().len(), 1);
    }

    #[test]
    fn distinct_refs_and_subdirs_do_not_overwrite() {
        let mut lock = Lockfile::new();
        lock.upsert(pin("github:a/b", "v1", "one", Some("plugins/one")));
        lock.upsert(pin("github:a/b", "v2", "two", Some("plugins/two")));
        assert_eq!(lock.plugins().len(), 2);
        assert_eq!(
            lock.get_for("github:a/b", Some("v2"), Some("plugins/two"))
                .unwrap()
                .commit,
            "two"
        );
        assert!(lock
            .get_for("github:a/b", Some("v1"), Some("plugins/two"))
            .is_none());
    }

    #[test]
    fn remove_for_works() {
        let mut lock = Lockfile::new();
        lock.upsert(pin("github:a/b", "main", "sha", None));
        assert!(lock.remove_for("github:a/b", Some("main"), None).is_some());
        assert!(lock.get("github:a/b").is_none());
        assert!(lock.remove_for("github:a/b", Some("main"), None).is_none());
    }

    #[test]
    fn remove_for_leaves_other_subdirs() {
        let mut lock = Lockfile::new();
        lock.upsert(pin(
            "github:example/rpp-plugins",
            "v1",
            "one",
            Some("plugins/one"),
        ));
        lock.upsert(pin(
            "github:example/rpp-plugins",
            "v1",
            "two",
            Some("plugins/two"),
        ));
        assert!(lock
            .remove_for(
                "github:example/rpp-plugins",
                Some("v1"),
                Some("plugins/one"),
            )
            .is_some());
        assert_eq!(lock.plugins().len(), 1);
        assert_eq!(
            lock.get_for(
                "github:example/rpp-plugins",
                Some("v1"),
                Some("plugins/two"),
            )
            .unwrap()
            .commit,
            "two"
        );
    }

    #[test]
    fn rejects_newer_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rpp.lock");
        std::fs::write(&path, "version = 999\n").unwrap();
        assert!(matches!(
            Lockfile::load(&path),
            Err(Error::UnsupportedLockVersion { found: 999, .. })
        ));
    }

    #[test]
    fn invalid_toml_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rpp.lock");
        std::fs::write(&path, "this is not toml = [").unwrap();
        assert!(matches!(
            Lockfile::load(&path),
            Err(Error::InvalidLockfile { .. })
        ));
    }
}
