//! `rpp` core library: project config, the runtime-agnostic plugin model,
//! shared host services, the Lua plugin system, and the incremental build engine.
//!
//! The pipeline is defined entirely in terms of [`model`] traits. The Lua
//! runtime ([`lua::LuaPluginFactory`]) is one implementation; the build engine
//! ([`engine::Engine`]) drives them.

pub mod config;
pub mod engine;
pub mod host;
pub mod manifest;
pub mod model;

#[cfg(feature = "lua")]
pub mod lua;

mod cache;
mod error;
mod util;

pub use error::{Error, Result};
