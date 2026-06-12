//! `rpp.path` builtin: forward-slash path helpers.

use mlua::{Lua, MultiValue, Table};

use crate::util::glob;

/// Build the `rpp.path` module table.
pub(crate) fn module(lua: &Lua) -> mlua::Result<Table> {
    let t = lua.create_table()?;

    t.set(
        "join",
        lua.create_function(|lua, parts: MultiValue| {
            let mut segments: Vec<String> = Vec::new();
            for part in parts {
                let s: mlua::String = lua
                    .unpack(part)
                    .map_err(|_| mlua::Error::external("path.join expects string arguments"))?;
                let s = s.to_str()?;
                for seg in s.split('/') {
                    if !seg.is_empty() {
                        segments.push(seg.to_string());
                    }
                }
            }
            Ok(segments.join("/"))
        })?,
    )?;

    t.set(
        "dirname",
        lua.create_function(|_, p: mlua::String| {
            let p = p.to_str()?;
            Ok(match p.rsplit_once('/') {
                Some((dir, _)) => dir.to_string(),
                None => String::new(),
            })
        })?,
    )?;

    t.set(
        "basename",
        lua.create_function(|_, p: mlua::String| {
            let p = p.to_str()?;
            Ok(match p.rsplit_once('/') {
                Some((_, base)) => base.to_string(),
                None => p.to_string(),
            })
        })?,
    )?;

    t.set(
        "ext",
        lua.create_function(|_, p: mlua::String| {
            let p = p.to_str()?;
            let base = p.rsplit_once('/').map(|(_, b)| b).unwrap_or(&p);
            Ok(match base.rsplit_once('.') {
                Some((name, ext)) if !name.is_empty() => ext.to_string(),
                _ => String::new(),
            })
        })?,
    )?;

    t.set(
        "with_ext",
        lua.create_function(|_, (p, ext): (mlua::String, mlua::String)| {
            let p = p.to_str()?;
            let ext = ext.to_str()?;
            let ext = ext.trim_start_matches('.');
            let (dir, base) = match p.rsplit_once('/') {
                Some((d, b)) => (Some(d), b),
                None => (None, &p[..]),
            };
            let stem = match base.rsplit_once('.') {
                Some((name, _)) if !name.is_empty() => name,
                _ => base,
            };
            let new_base = if ext.is_empty() {
                stem.to_string()
            } else {
                format!("{stem}.{ext}")
            };
            Ok(match dir {
                Some(d) => format!("{d}/{new_base}"),
                None => new_base,
            })
        })?,
    )?;

    t.set(
        "match",
        lua.create_function(|_, (pattern, p): (mlua::String, mlua::String)| {
            Ok(glob::matches(&pattern.to_str()?, &p.to_str()?))
        })?,
    )?;

    Ok(t)
}
