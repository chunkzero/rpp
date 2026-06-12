//! Plugin source resolution, caching, lockfile, and GitHub discovery for rpp.
//!
//! This is a standalone, **blocking** crate (no async / tokio). It implements
//! section 6 of `docs/SPEC.md`:
//!
//! - [`PluginSource`] — parse `path:` and `github:` source strings.
//! - [`Resolver`] — resolve a source to a local directory containing a
//!   `plugin.toml`, fetching and caching GitHub repositories on demand.
//! - [`Lockfile`] — read/write the `rpp.lock` pin file.
//! - [`search`] — discover plugin repositories via the GitHub `rpp-plugin`
//!   topic.
//! - [`parse_manifest_summary`] — read `[plugin] id`/`version` from a
//!   `plugin.toml`.
//!
//! # Example
//!
//! ```no_run
//! use rpp_fetch::{Lockfile, PluginSource, Resolver};
//!
//! # fn main() -> Result<(), rpp_fetch::Error> {
//! let source = PluginSource::parse("github:example/rpp-plugins", Some("v1.0.0"), None)?;
//! let lock = Lockfile::load(std::path::Path::new("rpp.lock"))?;
//! let resolver = Resolver::new(".")?;
//! let resolved = resolver.resolve(&source, lock.get(&source.canonical()))?;
//! println!("plugin at {}", resolved.root.display());
//! # Ok(())
//! # }
//! ```

mod error;
mod extract;
mod http;
mod lockfile;
mod manifest;
mod resolver;
mod search;
mod source;

pub use error::{Error, Result};
pub use http::{HttpConfig, DEFAULT_API_BASE, DEFAULT_CODELOAD_BASE, USER_AGENT};
pub use lockfile::{LockedPlugin, Lockfile, LOCKFILE_VERSION};
pub use manifest::{parse_manifest_summary, ManifestSummary, MANIFEST_FILE};
pub use resolver::{Pin, ResolvedPlugin, Resolver};
pub use search::{search, search_with_config, RepoHit};
pub use source::PluginSource;
