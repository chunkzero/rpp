//! `rpp.json` builtin: serde-backed JSON decode/encode.

use mlua::{Lua, LuaSerdeExt, Table, Value};

use crate::lua::convert::lua_to_json;

/// Build the `rpp.json` module table.
pub(crate) fn module(lua: &Lua) -> mlua::Result<Table> {
    let t = lua.create_table()?;

    t.set(
        "decode",
        lua.create_function(|lua, text: mlua::String| {
            let text = text.to_str()?;
            let value: serde_json::Value = serde_json::from_str(&text)
                .map_err(|e| mlua::Error::external(format!("json decode error: {e}")))?;
            lua.to_value(&value)
        })?,
    )?;

    t.set(
        "encode",
        lua.create_function(|_, (value, opts): (Value, Option<Table>)| {
            let pretty = match opts {
                Some(t) => t.get::<Option<bool>>("pretty")?.unwrap_or(false),
                None => false,
            };
            let json = lua_to_json(value)
                .map_err(|e| mlua::Error::external(format!("json encode error: {e}")))?;
            let out = if pretty {
                serde_json::to_string_pretty(&json)
            } else {
                serde_json::to_string(&json)
            }
            .map_err(|e| mlua::Error::external(format!("json encode error: {e}")))?;
            Ok(out)
        })?,
    )?;

    Ok(t)
}
