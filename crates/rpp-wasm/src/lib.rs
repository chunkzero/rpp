//! Dynamic WASIp2 component host used by TypeScript plugins.

#![deny(missing_docs)]

mod component;
mod engine;
mod error;
mod instance;
mod store;
mod types;
mod value;

pub use component::CompiledComponent;
pub use engine::WasmEngine;
pub use error::{Error, Result};
pub use instance::WasmInstance;
pub use types::{Function, Limits, Permissions, Preopen, Schema, Value, ValueType};
