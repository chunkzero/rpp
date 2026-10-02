//! HTTP configuration and the blocking registry client. The base URL is
//! injectable so tests can point at a local mock server.

use std::io::Read as _;

use crate::error::Error;

/// The default registry index base URL.
pub const DEFAULT_REGISTRY_BASE: &str =
    "https://raw.githubusercontent.com/chunkzero/rpp-registry/main";

/// The `User-Agent` header sent with every request.
pub const USER_AGENT: &str = "rpp";

/// Size cap for registry archives.
pub(crate) const MAX_ARCHIVE_BYTES: u64 = 128 * 1024 * 1024;

/// Size cap for registry index files.
pub(crate) const MAX_INDEX_BYTES: u64 = 8 * 1024 * 1024;

/// Where registry requests go.
///
/// The base URL is configurable so integration tests can serve canned responses from a
/// local listener without touching the network.
#[derive(Debug, Clone)]
pub struct HttpConfig {
    /// Base URL of the registry index (no trailing slash); `RPP_REGISTRY`
    /// overrides the default.
    pub registry_base: String,
}

impl Default for HttpConfig {
    fn default() -> Self {
        let base = std::env::var("RPP_REGISTRY")
            .ok()
            .filter(|base| !base.is_empty())
            .unwrap_or_else(|| DEFAULT_REGISTRY_BASE.to_string());
        Self::with_base(base)
    }
}

impl HttpConfig {
    /// A config pointing the registry at `base`. Useful for tests backed by a mock
    /// server.
    pub fn with_base(base: impl Into<String>) -> Self {
        HttpConfig {
            registry_base: base.into().trim_end_matches('/').to_string(),
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

/// Blocking client for registry files and archives.
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
        let resp = self
            .agent
            .get(url)
            .set("User-Agent", USER_AGENT)
            .call()
            .map_err(|e| match e {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_base_trims_slash() {
        let cfg = HttpConfig::with_base("http://localhost:1234/");
        assert_eq!(cfg.registry_base, "http://localhost:1234");
    }
}
