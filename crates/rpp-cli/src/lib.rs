//! Project-level APIs shared by the `rpp` binary and integration-test hosts.

pub mod commands;
pub mod project;
pub mod ui;

mod atomic;
mod codegen;
mod ordered_json;
