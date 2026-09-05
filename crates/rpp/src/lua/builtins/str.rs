//! `rpp.str` builtin: small string helpers.

use mlua::{Lua, Table};

/// Build the `rpp.str` module table.
pub(crate) fn module(lua: &Lua) -> mlua::Result<Table> {
    let t = lua.create_table()?;

    t.set(
        "starts_with",
        lua.create_function(|_, (s, prefix): (mlua::String, mlua::String)| {
            Ok(s.to_str()?.starts_with(&*prefix.to_str()?))
        })?,
    )?;

    t.set(
        "ends_with",
        lua.create_function(|_, (s, suffix): (mlua::String, mlua::String)| {
            Ok(s.to_str()?.ends_with(&*suffix.to_str()?))
        })?,
    )?;

    t.set(
        "split",
        lua.create_function(|lua, (s, sep): (mlua::String, mlua::String)| {
            let s = s.to_str()?;
            let sep = sep.to_str()?;
            let out = lua.create_table()?;
            if sep.is_empty() {
                out.push(s.to_string())?;
            } else {
                for (i, part) in s.split(&*sep).enumerate() {
                    out.raw_set(i + 1, part.to_string())?;
                }
            }
            Ok(out)
        })?,
    )?;

    t.set(
        "trim",
        lua.create_function(|_, s: mlua::String| Ok(s.to_str()?.trim().to_string()))?,
    )?;

    Ok(t)
}
