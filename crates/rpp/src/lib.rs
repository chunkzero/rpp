//! `rpp` core library: project config, the runtime-agnostic plugin model,
//! the TypeScript plugin runtime and its host services, and the incremental build engine.
//!
//! The pipeline is defined entirely in terms of [`model`] traits. The V8 runtime
//! ([`js::JsPluginFactory`]) is one implementation; the build engine
//! ([`engine::Engine`]) drives them.

pub mod config;
pub mod engine;
pub mod manifest;
pub mod model;

#[cfg(feature = "js")]
pub mod js;

mod cache;
mod error;
mod util;

pub use error::{Error, Result};

/// Where the legacy-configuration and Lua-plugin migration steps are documented.
pub const MIGRATION_GUIDE: &str = "https://github.com/chunkzero/rpp/blob/main/docs/MIGRATING.md";
