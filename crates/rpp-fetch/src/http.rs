//! HTTP configuration and the blocking registry client. The base URLs are
//! injectable so tests can point at a local mock server.

use std::io::Read as _;

use crate::error::Error;

/// The default GitHub REST API base URL.
pub const DEFAULT_API_BASE: &str = "https://api.github.com";

/// The default registry index base URL.
pub const DEFAULT_REGISTRY_BASE: &str =
    "https://raw.githubusercontent.com/chunkzero/rpp-registry/main";

/// The `User-Agent` header sent with every request.
pub const USER_AGENT: &str = "rpp";

/// Size cap for registry archives.
pub(crate) const MAX_ARCHIVE_BYTES: u64 = 128 * 1024 * 1024;

/// Size cap for registry index files.
pub(crate) const MAX_INDEX_BYTES: u64 = 8 * 1024 * 1024;

/// HTTP endpoint + auth configuration for registry requests.
///
/// The base URLs are configurable so integration tests can serve canned
/// responses from a local listener without touching the network.
#[derive(Debug, Clone)]
pub struct HttpConfig {
    /// Base URL for the GitHub REST API (no trailing slash); the bearer token is
    /// only sent to this origin.
    pub api_base: String,
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
    /// A config pointing the API and registry at the same `base` URL, with no
    /// token. Useful for tests backed by a single mock server.
    pub fn with_base(base: impl Into<String>) -> Self {
        let base = base.into();
        let base = base.trim_end_matches('/').to_string();
        HttpConfig {
            api_base: base.clone(),
            registry_base: base,
            token: None,
        }
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
/// over HTTPS to the API base's origin, never to archive URLs elsewhere.
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
            url.starts_with("https://")
                && authority(url).is_some_and(|host| Some(host) == authority(&self.config.api_base))
                && self.config.api_base.starts_with("https://")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_base_trims_slash() {
        let cfg = HttpConfig::with_base("http://localhost:1234/");
        assert_eq!(cfg.api_base, "http://localhost:1234");
        assert_eq!(cfg.registry_base, "http://localhost:1234");
        assert!(cfg.token.is_none());
    }
}
