//! HTTP configuration and the thin GitHub REST client used by the resolver and
//! search. Base URLs are injectable so tests can point at a local mock server.

use std::io::Read as _;

use serde::Deserialize;

use crate::error::{Error, Result};

/// The default GitHub REST API base URL.
pub const DEFAULT_API_BASE: &str = "https://api.github.com";

/// The default codeload (tarball download) base URL.
pub const DEFAULT_CODELOAD_BASE: &str = "https://codeload.github.com";

/// The default registry index base URL.
pub const DEFAULT_REGISTRY_BASE: &str =
    "https://raw.githubusercontent.com/chunkzero/rpp-registry/main";

/// The `User-Agent` header sent with every request.
pub const USER_AGENT: &str = "rpp";
const MAX_TARBALL_BYTES: u64 = 128 * 1024 * 1024;

/// Size cap for registry archives.
pub(crate) const MAX_ARCHIVE_BYTES: u64 = MAX_TARBALL_BYTES;

/// Size cap for registry index files.
pub(crate) const MAX_INDEX_BYTES: u64 = 8 * 1024 * 1024;

/// HTTP endpoint + auth configuration shared by the resolver and search.
///
/// The base URLs are configurable so integration tests can serve canned
/// responses from a local listener without touching the network.
#[derive(Debug, Clone)]
pub struct HttpConfig {
    /// Base URL for the GitHub REST API (no trailing slash).
    pub api_base: String,
    /// Base URL for codeload tarball downloads (no trailing slash).
    pub codeload_base: String,
    /// Base URL of the registry index (no trailing slash); `RPP_REGISTRY`
    /// overrides the default.
    pub registry_base: String,
    /// Optional bearer token; defaults to the `GITHUB_TOKEN` env var.
    pub token: Option<String>,
}

impl Default for HttpConfig {
    fn default() -> Self {
        HttpConfig {
            api_base: DEFAULT_API_BASE.to_string(),
            codeload_base: DEFAULT_CODELOAD_BASE.to_string(),
            registry_base: std::env::var("RPP_REGISTRY")
                .ok()
                .filter(|base| !base.is_empty())
                .map(|base| base.trim_end_matches('/').to_string())
                .unwrap_or_else(|| DEFAULT_REGISTRY_BASE.to_string()),
            token: std::env::var("GITHUB_TOKEN").ok().filter(|t| !t.is_empty()),
        }
    }
}

impl HttpConfig {
    /// A config pointing the API, codeload and registry at the same `base` URL, with no
    /// token. Useful for tests backed by a single mock server.
    pub fn with_base(base: impl Into<String>) -> Self {
        let base = base.into();
        let base = base.trim_end_matches('/').to_string();
        HttpConfig {
            api_base: base.clone(),
            codeload_base: base.clone(),
            registry_base: base,
            token: None,
        }
    }
}

/// A thin blocking GitHub client wrapping a [`ureq::Agent`].
pub(crate) struct GitHubClient {
    agent: ureq::Agent,
    config: HttpConfig,
}

/// Minimal shape of `GET /repos/{owner}/{repo}` (default branch lookup).
#[derive(Deserialize)]
struct RepoInfo {
    default_branch: String,
}

/// Minimal shape of `GET /repos/{owner}/{repo}/commits/{ref}`.
#[derive(Deserialize)]
struct CommitInfo {
    sha: String,
}

impl GitHubClient {
    pub(crate) fn new(config: HttpConfig) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(60))
            .build();
        GitHubClient { agent, config }
    }

    /// Apply shared headers (User-Agent, optional auth) to a request.
    fn prepare(&self, req: ureq::Request) -> ureq::Request {
        let req = req.set("User-Agent", USER_AGENT);
        match &self.config.token {
            Some(token) => req.set("Authorization", &format!("Bearer {token}")),
            None => req,
        }
    }

    /// Resolve `ref_` (or the default branch when `None`) to a full commit SHA.
    pub(crate) fn resolve_commit(
        &self,
        owner: &str,
        repo: &str,
        ref_: Option<&str>,
    ) -> Result<(String, String)> {
        // Determine the ref name we are resolving: explicit ref, else the repo's
        // default branch.
        let ref_name = match ref_ {
            Some(r) => r.to_string(),
            None => self.fetch_default_branch(owner, repo)?,
        };

        let url = format!(
            "{}/repos/{}/{}/commits/{}",
            self.config.api_base, owner, repo, ref_name
        );
        let info: CommitInfo = self.get_json(&url)?;
        Ok((ref_name, info.sha))
    }

    fn fetch_default_branch(&self, owner: &str, repo: &str) -> Result<String> {
        let url = format!("{}/repos/{}/{}", self.config.api_base, owner, repo);
        let info: RepoInfo = self.get_json(&url)?;
        Ok(info.default_branch)
    }

    /// Perform a GET and deserialize the JSON body into `T`.
    fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let req = self.prepare(self.agent.get(url));
        let resp = req.call().map_err(|e| github_err(url, e))?;
        resp.into_json::<T>().map_err(|e| Error::GitHub {
            url: url.to_string(),
            reason: format!("invalid JSON response: {e}"),
        })
    }

    /// Download the gzipped tarball for `sha` from codeload, returning raw bytes.
    pub(crate) fn download_tarball(&self, owner: &str, repo: &str, sha: &str) -> Result<Vec<u8>> {
        let url = format!(
            "{}/{}/{}/tar.gz/{}",
            self.config.codeload_base, owner, repo, sha
        );
        let req = self.prepare(self.agent.get(&url));
        let resp = req.call().map_err(|e| github_err(&url, e))?;

        let mut buf = Vec::new();
        resp.into_reader()
            .take(MAX_TARBALL_BYTES + 1)
            .read_to_end(&mut buf)
            .map_err(|e| Error::GitHub {
                url: url.clone(),
                reason: format!("failed reading tarball body: {e}"),
            })?;
        if buf.len() as u64 > MAX_TARBALL_BYTES {
            return Err(Error::GitHub {
                url,
                reason: format!("tarball exceeds {MAX_TARBALL_BYTES} bytes"),
            });
        }
        Ok(buf)
    }

    /// Search repositories restricted to topic `rpp-plugin`. Returns the raw
    /// JSON value of the search response so the caller can map fields.
    pub(crate) fn search_repositories(&self, query: &str) -> Result<serde_json::Value> {
        let q = format!("topic:rpp-plugin {query}");
        let encoded = urlencode(q.trim());
        let url = format!("{}/search/repositories?q={}", self.config.api_base, encoded);
        self.get_json(&url)
    }
}

/// A failed registry request: the HTTP status when the server answered.
#[derive(Debug)]
pub(crate) struct Failure {
    pub(crate) status: Option<u16>,
    reason: String,
}

impl Failure {
    pub(crate) fn into_error(self, url: &str) -> Error {
        Error::Registry {
            url: url.to_string(),
            reason: self.reason,
        }
    }
}

/// Blocking client for registry files and archives. The bearer token is only sent
/// to hosts equal to the API base host, never to archive URLs elsewhere.
pub(crate) struct RegistryClient {
    agent: ureq::Agent,
    pub(crate) config: HttpConfig,
}

impl RegistryClient {
    pub(crate) fn new(config: HttpConfig) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(60))
            .build();
        RegistryClient { agent, config }
    }

    /// GET `url`, reading at most `max_bytes` of body.
    pub(crate) fn get_bytes(
        &self,
        url: &str,
        max_bytes: u64,
    ) -> std::result::Result<Vec<u8>, Failure> {
        let mut req = self.agent.get(url).set("User-Agent", USER_AGENT);
        if let Some(token) = self.config.token.as_ref().filter(|_| {
            authority(url).is_some_and(|host| Some(host) == authority(&self.config.api_base))
        }) {
            req = req.set("Authorization", &format!("Bearer {token}"));
        }
        let resp = req.call().map_err(|e| match e {
            ureq::Error::Status(code, resp) => Failure {
                status: Some(code),
                reason: format!("HTTP {code} {}", resp.status_text()),
            },
            ureq::Error::Transport(t) => Failure {
                status: None,
                reason: format!("transport error: {t}"),
            },
        })?;
        let mut buf = Vec::new();
        resp.into_reader()
            .take(max_bytes + 1)
            .read_to_end(&mut buf)
            .map_err(|e| Failure {
                status: None,
                reason: format!("failed reading response body: {e}"),
            })?;
        if buf.len() as u64 > max_bytes {
            return Err(Failure {
                status: None,
                reason: format!("response exceeds {max_bytes} bytes"),
            });
        }
        Ok(buf)
    }

    /// GET `url` and deserialize an index-sized JSON body.
    pub(crate) fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
    ) -> std::result::Result<T, Failure> {
        let bytes = self.get_bytes(url, MAX_INDEX_BYTES)?;
        serde_json::from_slice(&bytes).map_err(|e| Failure {
            status: None,
            reason: format!("invalid JSON response: {e}"),
        })
    }
}

/// The `host[:port]` of an absolute URL.
fn authority(url: &str) -> Option<&str> {
    let rest = url.split_once("://")?.1;
    rest.split(['/', '?', '#']).next()
}

/// Convert a `ureq::Error` into our [`Error::GitHub`], surfacing the status code
/// for HTTP-level failures.
fn github_err(url: &str, err: ureq::Error) -> Error {
    let reason = match err {
        ureq::Error::Status(code, resp) => {
            let status = resp.status_text().to_string();
            format!("HTTP {code} {status}")
        }
        ureq::Error::Transport(t) => format!("transport error: {t}"),
    };
    Error::GitHub {
        url: url.to_string(),
        reason,
    }
}

/// Minimal percent-encoding for query strings (spaces, `+`, `:` and friends).
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_base_trims_slash() {
        let cfg = HttpConfig::with_base("http://localhost:1234/");
        assert_eq!(cfg.api_base, "http://localhost:1234");
        assert_eq!(cfg.codeload_base, "http://localhost:1234");
        assert_eq!(cfg.registry_base, "http://localhost:1234");
        assert!(cfg.token.is_none());
    }

    #[test]
    fn urlencodes_spaces_and_colons() {
        assert_eq!(
            urlencode("topic:rpp-plugin atlas"),
            "topic%3Arpp-plugin%20atlas"
        );
    }
}
