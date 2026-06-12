//! Lua plugin system v2 (spec §4).
//!
//! Plugins are sandboxed packages (`plugin.toml` + entry script) running on
//! vendored Lua 5.4. The public entry point is [`LuaPluginFactory`], which
//! implements [`crate::model::PluginFactory`].

mod builtins;
mod convert;
mod ctx;
mod factory;
mod file;
mod instance;
mod plugin_builder;
mod sandbox;
mod traceback;

pub use builtins::LogLevel;
pub use factory::LuaPluginFactory;
pub use instance::LuaPluginInstance;
