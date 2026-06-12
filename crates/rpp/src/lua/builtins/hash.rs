//! `rpp.hash` builtin: common hash functions returning hex strings.

use mlua::{Lua, Table};
use sha2::{Digest, Sha256};

use crate::util::hash::{to_hex, u64_hex, xxh3};

/// Build the `rpp.hash` module table.
pub(crate) fn module(lua: &Lua) -> mlua::Result<Table> {
    let t = lua.create_table()?;

    t.set(
        "xxh3",
        lua.create_function(|_, s: mlua::String| Ok(u64_hex(xxh3(&s.as_bytes()))))?,
    )?;

    t.set(
        "sha256",
        lua.create_function(|_, s: mlua::String| {
            let mut hasher = Sha256::new();
            hasher.update(s.as_bytes());
            Ok(to_hex(&hasher.finalize()))
        })?,
    )?;

    t.set(
        "md5",
        lua.create_function(|_, s: mlua::String| {
            let digest = md5::compute(s.as_bytes());
            Ok(to_hex(&digest.0))
        })?,
    )?;

    t.set(
        "crc32",
        lua.create_function(|_, s: mlua::String| {
            let mut hasher = crc32fast::Hasher::new();
            hasher.update(&s.as_bytes());
            Ok(hasher.finalize())
        })?,
    )?;

    Ok(t)
}
