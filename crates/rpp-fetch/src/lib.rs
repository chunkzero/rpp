//! Plugin registry resolution, caching, and lockfile for rpp.
//!
//! This is a standalone, **blocking** crate (no async / tokio). It implements
//! section 6 of `docs/SPEC.md`:
//!
//! - [`registry`] — resolve `rpp.json` dependencies from the plugin registry or
//!   `path:` directories, pinned in an `rpp.lock` version 3.

mod error;
mod http;
pub mod registry;

pub use error::{Error, Incompatibility, Result};
pub use http::{HttpConfig, DEFAULT_REGISTRY_BASE, USER_AGENT};
