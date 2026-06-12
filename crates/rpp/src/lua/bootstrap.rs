//! Shared entry-script evaluation used at validation load and instantiation.

use std::path::Path;
use std::time::Duration;

use mlua::{Lua, Value};

use crate::error::{Error, Result};
use crate::lua::sandbox::{install_limits, run_limited, Deadline, Sandbox};
use crate::lua::traceback;

/// A completed entry-script evaluation with its owning Lua state.
pub(crate) struct EntryEval {
    pub lua: Lua,
    pub sandbox: Sandbox,
    pub deadline: Deadline,
    pub value: Value,
}

/// Evaluate a plugin entry script in a fresh Lua state.
pub(crate) fn eval_entry(
    plugin_id: &str,
    root: &Path,
    entry_name: &str,
    entry_source: &str,
    memory_limit: usize,
    execution_limit: Duration,
) -> Result<EntryEval> {
    let lua = Lua::new();
    let deadline = install_limits(&lua, memory_limit).map_err(|e| Error::PluginLoad {
        plugin: plugin_id.to_string(),
        message: traceback::render(&e),
    })?;
    let sandbox = Sandbox::new(&lua, plugin_id, root).map_err(|e| Error::PluginLoad {
        plugin: plugin_id.to_string(),
        message: traceback::render(&e),
    })?;
    let value = eval_entry_in_lua(
        &lua,
        &sandbox,
        &deadline,
        execution_limit,
        entry_name,
        entry_source,
    )
    .map_err(|e| Error::PluginLoad {
        plugin: plugin_id.to_string(),
        message: traceback::render(&e),
    })?;
    Ok(EntryEval {
        lua,
        sandbox,
        deadline,
        value,
    })
}

/// Evaluate the entry script in an already-prepared sandbox.
pub(crate) fn eval_entry_in_lua(
    lua: &Lua,
    sandbox: &Sandbox,
    deadline: &Deadline,
    execution_limit: Duration,
    entry_name: &str,
    entry_source: &str,
) -> mlua::Result<Value> {
    run_limited(deadline, execution_limit, || {
        sandbox.exec(lua, &format!("@{entry_name}"), entry_source)
    })
}
