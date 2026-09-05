//! GitHub repository discovery, restricted to the `rpp-plugin` topic.

use crate::error::{Error, Result};
use crate::http::{GitHubClient, HttpConfig};

/// A single repository hit from a plugin search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoHit {
    /// Short repository name (without owner).
    pub name: String,
    /// `owner/repo`.
    pub full_name: String,
    /// The repository description, if any.
    pub description: Option<String>,
    /// Star count.
    pub stars: u64,
    /// The repository's HTML URL.
    pub url: String,
}

/// Search GitHub for plugin repositories matching `query`, restricted to the
/// `rpp-plugin` topic, using the default HTTP configuration.
///
/// # Errors
///
/// Returns [`Error::GitHub`] on network/HTTP failure or a malformed response.
pub fn search(query: &str) -> Result<Vec<RepoHit>> {
    search_with_config(query, HttpConfig::default())
}

/// Like [`search`] but with an explicit HTTP configuration (so tests can point
/// at a local mock server).
///
/// # Errors
///
/// Returns [`Error::GitHub`] on network/HTTP failure or a malformed response.
pub fn search_with_config(query: &str, config: HttpConfig) -> Result<Vec<RepoHit>> {
    let client = GitHubClient::new(config);
    let value = client.search_repositories(query)?;
    parse_search_response(&value)
}

/// Map the GitHub search JSON into [`RepoHit`]s.
fn parse_search_response(value: &serde_json::Value) -> Result<Vec<RepoHit>> {
    let items = value
        .get("items")
        .and_then(|i| i.as_array())
        .ok_or_else(|| Error::GitHub {
            url: "search/repositories".to_string(),
            reason: "response missing `items` array".to_string(),
        })?;

    let mut hits = Vec::with_capacity(items.len());
    for item in items {
        let Some(full_name) = item.get("full_name").and_then(|v| v.as_str()) else {
            continue;
        };
        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| {
                full_name
                    .rsplit('/')
                    .next()
                    .unwrap_or(full_name)
                    .to_string()
            });
        let description = item
            .get("description")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let stars = item
            .get("stargazers_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let url = item
            .get("html_url")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("https://github.com/{full_name}"));

        hits.push(RepoHit {
            name,
            full_name: full_name.to_string(),
            description,
            stars,
            url,
        });
    }
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_items() {
        let v = serde_json::json!({
            "items": [
                {
                    "name": "rpp-plugins",
                    "full_name": "example/rpp-plugins",
                    "description": "Some plugins",
                    "stargazers_count": 42,
                    "html_url": "https://github.com/example/rpp-plugins"
                },
                {
                    "full_name": "other/thing",
                    "description": null,
                    "stargazers_count": 0,
                    "html_url": "https://github.com/other/thing"
                }
            ]
        });
        let hits = parse_search_response(&v).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].name, "rpp-plugins");
        assert_eq!(hits[0].full_name, "example/rpp-plugins");
        assert_eq!(hits[0].stars, 42);
        assert_eq!(hits[0].description.as_deref(), Some("Some plugins"));
        assert_eq!(hits[1].description, None);
    }

    #[test]
    fn missing_items_errors() {
        let v = serde_json::json!({ "message": "rate limited" });
        assert!(matches!(
            parse_search_response(&v),
            Err(Error::GitHub { .. })
        ));
    }
}
