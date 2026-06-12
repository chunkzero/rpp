//! Builtin Lua modules preloaded for `require` (spec §4).

pub(crate) mod hash;
pub(crate) mod json;
pub(crate) mod log;
pub(crate) mod path;
pub(crate) mod str;
pub(crate) mod toml_mod;

pub use log::LogLevel;
