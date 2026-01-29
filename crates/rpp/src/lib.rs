//! RPP - Resource Pack Processor
//!
//! A multi-phase build pipeline with sandboxed Lua plugins.

pub mod build;
pub mod config;
pub mod lua;
pub mod plugin;
pub mod sandbox;
pub mod worker;

// Re-export commonly used types
pub use build::{BuildEngine, BuildEngineBuilder, BuildError, BuildResult};
pub use config::{ConfigError, DevServerConfig, RppConfig};
pub use plugin::{
    GeneratorContext, GeneratorPlugin, LuaProcessor, Plugin, PluginRegistry, ProcessResult,
    ProcessingContext, ProcessorPlugin,
};
