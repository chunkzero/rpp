//! `rpp.toml` builtin: serde-backed TOML decode/encode.

use mlua::{Lua, Table, Value};

use crate::lua::convert::{constructors, lua_to_toml, toml_to_lua};

/// Build the `rpp.toml` module table.
pub(crate) fn module(lua: &Lua) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    constructors(lua, &t)?;

    t.set(
        "decode",
        lua.create_function(|lua, text: mlua::String| {
            let text = text.to_str()?;
            let value: toml::Value = toml::from_str(&text)
                .map_err(|e| mlua::Error::external(format!("toml decode error: {e}")))?;
            toml_to_lua(lua, &value)
                .map_err(|e| mlua::Error::external(format!("toml decode error: {e}")))
        })?,
    )?;

    t.set(
        "encode",
        lua.create_function(|_, value: Value| {
            let toml_value = lua_to_toml(value)
                .map_err(|e| mlua::Error::external(format!("toml encode error: {e}")))?;
            toml::to_string(&toml_value)
                .map_err(|e| mlua::Error::external(format!("toml encode error: {e}")))
        })?,
    )?;

    Ok(t)
}
