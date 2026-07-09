//! Dynamic WASIp2 component host used by Lua plugins.

#![deny(missing_docs)]

mod engine;
mod error;
mod instance;
mod process;
mod store;
mod types;

pub use engine::{CompiledComponent, WasmEngine};
pub use error::{Error, Result};
pub use instance::WasmInstance;
pub use types::{
    Function, Limits, Permissions, Preopen, ProcessOutput, ProcessRequest, Schema, Value,
    ValueType, DEFAULT_DEADLINE, DEFAULT_MEMORY_LIMIT,
};

/// Run a structured external process using the same policy used by components.
pub fn run_process(
    permissions: &Permissions,
    request: ProcessRequest,
) -> std::result::Result<ProcessOutput, String> {
    process::run(permissions, request)
}
