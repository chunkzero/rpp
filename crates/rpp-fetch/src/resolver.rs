//! Resolution of [`PluginSource`]s to local directories containing a
//! `plugin.toml`, with GitHub fetch + caching.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::extract::extract_tarball;
use crate::http::{GitHubClient, HttpConfig};
use crate::lockfile::{LockedPlugin, Lockfile};
use crate::manifest::MANIFEST_FILE;
use crate::source::PluginSource;

/// The default cache subdirectory under the user cache dir (`~/.cache/rpp`).
const CACHE_SUBDIR: &str = "plugins";

/// A pin describing what a GitHub source resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    /// The ref that was requested (explicit, or the resolved default branch).
    pub ref_: String,
    /// The full commit SHA the ref resolved to.
    pub commit: String,
}

/// The result of resolving a plugin source.
#[derive(Debug, Clone)]
pub struct ResolvedPlugin {
    /// The local directory containing the plugin package (its `plugin.toml`).
    pub root: PathBuf,
    /// For GitHub sources, the pin that should be recorded in the lockfile.
    /// `None` for path sources (which are never locked).
    pub pinned: Option<Pin>,
}

/// Resolves plugin sources to local directories, fetching and caching GitHub
/// repositories as needed.
pub struct Resolver {
    cache_root: PathBuf,
    project_root: PathBuf,
    http: HttpConfig,
}

impl Resolver {
    /// Create a resolver.
    ///
    /// `project_root` is the directory relative `path:` sources are resolved
    /// against (typically the directory containing `rpp.toml`). The cache root
    /// is taken from `RPP_CACHE_DIR` if set, else `~/.cache/rpp/plugins`.
    ///
    /// # Errors
    ///
    /// Returns an error if no cache directory can be determined.
    pub fn new(project_root: impl Into<PathBuf>) -> Result<Self> {
        Ok(Resolver {
            cache_root: default_cache_root()?,
            project_root: project_root.into(),
            http: HttpConfig::default(),
        })
    }

    /// Override the cache root directory (e.g. for tests).
    pub fn with_cache_root(mut self, cache_root: impl Into<PathBuf>) -> Self {
        self.cache_root = cache_root.into();
        self
    }

    /// Override the HTTP configuration (base URLs + token).
    pub fn with_http_config(mut self, http: HttpConfig) -> Self {
        self.http = http;
        self
    }

    /// The cache root this resolver writes GitHub packages into.
    pub fn cache_root(&self) -> &Path {
        &self.cache_root
    }

    /// Resolve `source` to a local directory containing a `plugin.toml`.
    ///
    /// For GitHub sources, if `locked` is supplied and the pinned commit is
    /// already present in the cache, the directory is returned immediately with
    /// **no network calls**.
    ///
    /// # Errors
    ///
    /// Path sources error if the directory is missing or lacks a `plugin.toml`.
    /// GitHub sources error on network/HTTP failures, unsafe tar entries, or a
    /// missing `plugin.toml` after extraction.
    pub fn resolve(
        &self,
        source: &PluginSource,
        locked: Option<&LockedPlugin>,
    ) -> Result<ResolvedPlugin> {
        match source {
            PluginSource::Path { dir } => self.resolve_path(dir),
            PluginSource::GitHub {
                owner,
                repo,
                ref_,
                subdir,
            } => self.resolve_github(owner, repo, ref_.as_deref(), subdir.as_deref(), locked),
        }
    }

    fn resolve_path(&self, dir: &Path) -> Result<ResolvedPlugin> {
        let resolved = if dir.is_absolute() {
            dir.to_path_buf()
        } else {
            self.project_root.join(dir)
        };

        if !resolved.is_dir() {
            return Err(Error::PathNotFound(resolved));
        }
        if !resolved.join(MANIFEST_FILE).is_file() {
            return Err(Error::MissingManifest(resolved));
        }

        Ok(ResolvedPlugin {
            root: resolved,
            pinned: None,
        })
    }

    fn resolve_github(
        &self,
        owner: &str,
        repo: &str,
        ref_: Option<&str>,
        subdir: Option<&str>,
        locked: Option<&LockedPlugin>,
    ) -> Result<ResolvedPlugin> {
        let client = GitHubClient::new(self.http.clone());

        // Pin short-circuit: if a lock pins a commit already cached, return with
        // zero network calls when possible.
        if let Some(lock) = locked.filter(|lock| {
            lock.subdir.as_deref() == subdir && ref_.is_none_or(|requested| lock.ref_ == requested)
        }) {
            let root = self.ensure_cached_commit(&client, owner, repo, &lock.commit, subdir)?;
            return Ok(ResolvedPlugin {
                root,
                pinned: Some(Pin {
                    ref_: lock.ref_.clone(),
                    commit: lock.commit.clone(),
                }),
            });
        }

        // Resolve ref -> commit SHA, then ensure the commit is cached.
        let (ref_name, sha) = client.resolve_commit(owner, repo, ref_)?;
        let root = self.ensure_cached_commit(&client, owner, repo, &sha, subdir)?;

        Ok(ResolvedPlugin {
            root,
            pinned: Some(Pin {
                ref_: ref_name,
                commit: sha,
            }),
        })
    }

    /// Ensure `owner/repo@sha` is present in the cache and contains a valid
    /// manifest at `subdir`. Downloads and extracts when missing or damaged.
    fn ensure_cached_commit(
        &self,
        client: &GitHubClient,
        owner: &str,
        repo: &str,
        sha: &str,
        subdir: Option<&str>,
    ) -> Result<PathBuf> {
        // Extraction publishes the commit directory atomically, so a present
        // directory is complete. It is shared by every project and subdir that
        // pins this commit; a missing manifest is a bad `subdir`, not damage.
        let dir = self.commit_dir(owner, repo, sha);
        if !dir.is_dir() {
            let bytes = client.download_tarball(owner, repo, sha)?;
            extract_tarball(&bytes, &dir)?;
        }
        let root = self.join_subdir(&dir, subdir);
        self.verify_manifest(&root)?;
        Ok(root)
    }

    /// `<cache_root>/github/<owner>/<repo>/<sha>`
    fn commit_dir(&self, owner: &str, repo: &str, sha: &str) -> PathBuf {
        self.cache_root
            .join("github")
            .join(owner)
            .join(repo)
            .join(sha)
    }

    fn join_subdir(&self, dir: &Path, subdir: Option<&str>) -> PathBuf {
        match subdir {
            Some(s) if !s.is_empty() => dir.join(s),
            _ => dir.to_path_buf(),
        }
    }

    fn verify_manifest(&self, root: &Path) -> Result<()> {
        if !root.is_dir() {
            return Err(Error::PathNotFound(root.to_path_buf()));
        }
        if !root.join(MANIFEST_FILE).is_file() {
            return Err(Error::MissingManifest(root.to_path_buf()));
        }
        Ok(())
    }
}

/// Compute the default cache root: `RPP_CACHE_DIR` if set, else
/// `<user-cache-dir>/rpp/plugins`.
fn default_cache_root() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("RPP_CACHE_DIR") {
        if !dir.is_empty() {
            return Ok(PathBuf::from(dir));
        }
    }
    let base = dirs::cache_dir().ok_or_else(|| {
        Error::io(
            "determining user cache directory",
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no cache directory available; set RPP_CACHE_DIR",
            ),
        )
    })?;
    Ok(base.join("rpp").join(CACHE_SUBDIR))
}

impl Lockfile {
    /// Record a resolved GitHub plugin pin. Path sources are ignored.
    ///
    /// Returns the previous pin for the same source/ref/subdir key, if any.
    pub fn record_resolved(
        &mut self,
        source: &PluginSource,
        resolved: &ResolvedPlugin,
    ) -> Option<LockedPlugin> {
        let PluginSource::GitHub { subdir, .. } = source else {
            return None;
        };
        let pin = resolved.pinned.as_ref()?;
        self.upsert(LockedPlugin {
            source: source.canonical(),
            ref_: pin.ref_.clone(),
            commit: pin.commit.clone(),
            subdir: subdir.clone(),
        })
    }
}
