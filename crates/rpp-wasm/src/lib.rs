//! Dynamic WASIp2 component host used by TypeScript plugins.

#![deny(missing_docs)]

mod engine;
mod error;
mod instance;
mod store;
mod types;

pub use engine::{CompiledComponent, WasmEngine};
pub use error::{Error, Result};
pub use instance::WasmInstance;
pub use types::{
    Function, Limits, Permissions, Preopen, Schema, Value, ValueType, DEFAULT_DEADLINE,
    DEFAULT_MEMORY_LIMIT,
};
