//! Plugin system traits and types.

mod generator;
mod processor;
mod registry;

#[cfg(feature = "lua")]
mod lua_processor;

pub use generator::{GeneratorContext, GeneratorPlugin};
pub use processor::{ProcessResult, ProcessingContext, ProcessorPlugin};
pub use registry::PluginRegistry;

#[cfg(feature = "lua")]
pub use lua_processor::LuaProcessor;

/// Base trait for all plugins.
pub trait Plugin: Send + Sync {
    /// Unique identifier for this plugin.
    fn name(&self) -> &str;

    /// Semantic version string (e.g., "1.2.3").
    fn version(&self) -> &str;

    /// Downcast support for identifying plugin types.
    fn as_any(&self) -> &dyn std::any::Any;
}
