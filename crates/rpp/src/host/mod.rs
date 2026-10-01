//! Runtime-independent host services shared by plugin runtimes: access policy,
//! pack metadata, logging, hashing, and process execution.

pub(crate) mod access;
pub mod hash;
pub mod log;
pub(crate) mod process;

pub use access::RuntimeAccess;
pub(crate) use access::{Phase, PhaseCell};

/// Pack metadata exposed to plugin code as `ctx.pack`.
#[derive(Debug, Clone, Default)]
pub struct PackInfo {
    /// Pack name (`ctx.pack.name`).
    pub name: String,
    /// Pack description (`ctx.pack.description`).
    pub description: Option<String>,
    /// Pack format (`ctx.pack.format`).
    pub format: Option<u32>,
}
