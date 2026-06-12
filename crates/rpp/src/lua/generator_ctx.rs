//! Generator-phase `ctx` host bridge (spec §4).

use std::cell::RefCell;

use mlua::{Function, Table, Value};

use crate::model::GeneratorHost;
use crate::util::path::validate_relative;

/// Call a generator handler with the host bridge installed for one scope.
pub(crate) fn call_generator(
    lua: &mlua::Lua,
    handler: &Function,
    ctx: Table,
    host: &mut dyn GeneratorHost,
) -> mlua::Result<()> {
    let host = RefCell::new(host);
    lua.scope(|scope| {
        let host_files = &host;
        ctx.set(
            "files",
            scope.create_function_mut(
                move |lua, (_this, glob): (Value, Option<mlua::String>)| {
                    let glob = match glob {
                        Some(s) => Some(s.to_str()?.to_string()),
                        None => None,
                    };
                    let files = host_files.borrow_mut().list_files(glob.as_deref());
                    let t = lua.create_table()?;
                    for (i, f) in files.into_iter().enumerate() {
                        t.raw_set(i + 1, f)?;
                    }
                    Ok(t)
                },
            )?,
        )?;

        let host_read = &host;
        ctx.set(
            "read",
            scope.create_function_mut(move |lua, (_this, path): (Value, mlua::String)| {
                let path = path.to_str()?.to_string();
                validate_relative(&path).map_err(mlua::Error::external)?;
                match host_read.borrow_mut().read_file(&path) {
                    Some(bytes) => Ok(Value::String(lua.create_string(&bytes)?)),
                    None => Ok(Value::Nil),
                }
            })?,
        )?;

        let host_read_src = &host;
        ctx.set(
            "read_source",
            scope.create_function_mut(move |lua, (_this, path): (Value, mlua::String)| {
                let path = path.to_str()?.to_string();
                validate_relative(&path).map_err(mlua::Error::external)?;
                match host_read_src.borrow_mut().read_source(&path) {
                    Some(bytes) => Ok(Value::String(lua.create_string(&bytes)?)),
                    None => Ok(Value::Nil),
                }
            })?,
        )?;

        let host_emit = &host;
        ctx.set(
            "emit",
            scope.create_function_mut(
                move |_, (_this, path, contents): (Value, mlua::String, mlua::String)| {
                    let path = path.to_str()?.to_string();
                    validate_relative(&path).map_err(mlua::Error::external)?;
                    host_emit
                        .borrow_mut()
                        .emit(&path, contents.as_bytes().to_vec());
                    Ok(())
                },
            )?,
        )?;

        let host_remove = &host;
        ctx.set(
            "remove",
            scope.create_function_mut(move |_, (_this, path): (Value, mlua::String)| {
                let path = path.to_str()?.to_string();
                validate_relative(&path).map_err(mlua::Error::external)?;
                host_remove.borrow_mut().remove(&path);
                Ok(())
            })?,
        )?;

        handler.call::<()>(ctx)
    })
}
