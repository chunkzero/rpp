//! `rpp.hash` builtin: common hash functions returning hex strings.

use mlua::{Lua, Table};

use crate::host::hash;

/// Build the `rpp.hash` module table.
pub(crate) fn module(lua: &Lua) -> mlua::Result<Table> {
    let t = lua.create_table()?;

    t.set(
        "xxh3",
        lua.create_function(|_, s: mlua::String| Ok(hash::xxh3_hex(&s.as_bytes())))?,
    )?;
    t.set(
        "sha256",
        lua.create_function(|_, s: mlua::String| Ok(hash::sha256_hex(&s.as_bytes())))?,
    )?;
    t.set(
        "md5",
        lua.create_function(|_, s: mlua::String| Ok(hash::md5_hex(&s.as_bytes())))?,
    )?;
    t.set(
        "crc32",
        lua.create_function(|_, s: mlua::String| Ok(hash::crc32(&s.as_bytes())))?,
    )?;

    Ok(t)
}
