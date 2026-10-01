//! `rpp.log` builtin and the shared log table used by `ctx.log`.

use mlua::{Lua, MultiValue, Table, Value};

use crate::host::log::{emit, LogLevel};

/// Build a log table (`debug|info|warn|error`) bound to a plugin id.
///
/// Used both for the `rpp.log` module and for `ctx.log`.
pub(crate) fn table(lua: &Lua, plugin_id: &str) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    for (name, level) in [
        ("debug", LogLevel::Debug),
        ("info", LogLevel::Info),
        ("warn", LogLevel::Warn),
        ("error", LogLevel::Error),
    ] {
        let plugin = plugin_id.to_string();
        t.set(
            name,
            lua.create_function(move |_, args: MultiValue| {
                let message = stringify(args);
                emit(&plugin, level, &message);
                Ok(())
            })?,
        )?;
    }
    Ok(t)
}

/// Concatenate Lua values tab-separated, matching Lua's `print`.
pub(crate) fn stringify_values(values: impl IntoIterator<Item = Value>) -> String {
    let parts: Vec<String> = values
        .into_iter()
        .map(|v| match v {
            Value::String(s) => s.to_string_lossy().to_string(),
            Value::Integer(i) => i.to_string(),
            Value::Number(n) => n.to_string(),
            Value::Boolean(b) => b.to_string(),
            Value::Nil => "nil".to_string(),
            other => format!("<{}>", other.type_name()),
        })
        .collect();
    parts.join("\t")
}

fn stringify(args: MultiValue) -> String {
    stringify_values(args)
}
