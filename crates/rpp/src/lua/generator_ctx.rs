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
    env: Table,
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

        let host_source_files = &host;
        ctx.set(
            "source_files",
            scope.create_function_mut(
                move |lua, (_this, glob): (Value, Option<mlua::String>)| {
                    let glob = match glob {
                        Some(s) => Some(s.to_str()?.to_string()),
                        None => None,
                    };
                    let files = host_source_files
                        .borrow_mut()
                        .list_source_files(glob.as_deref());
                    let table = lua.create_table()?;
                    for (index, file) in files.into_iter().enumerate() {
                        table.raw_set(index + 1, file)?;
                    }
                    Ok(table)
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

        let host_emit_output = &host;
        ctx.set(
            "emit_output",
            scope.create_function_mut(
                move |_,
                      (_this, root, path, contents): (
                    Value,
                    mlua::String,
                    mlua::String,
                    mlua::String,
                )| {
                    let root = root.to_str()?.to_string();
                    let path = path.to_str()?.to_string();
                    validate_relative(&path).map_err(mlua::Error::external)?;
                    host_emit_output.borrow_mut().emit_output(
                        &root,
                        &path,
                        contents.as_bytes().to_vec(),
                    );
                    Ok(())
                },
            )?,
        )?;

        let host_load = &host;
        ctx.set(
            "load_source",
            scope.create_function_mut(
                move |lua, (_this, path): (Value, mlua::String)| -> mlua::Result<Value> {
                    let path = path.to_str()?.to_string();
                    validate_relative(&path).map_err(mlua::Error::external)?;
                    let bytes = host_load.borrow_mut().read_source(&path).ok_or_else(|| {
                        mlua::Error::external(format!("source `{path}` not found"))
                    })?;
                    let source = std::str::from_utf8(&bytes).map_err(|error| {
                        mlua::Error::external(format!("source `{path}` is not UTF-8: {error}"))
                    })?;
                    lua.load(source)
                        .set_name(format!("@{path}"))
                        .set_environment(env.clone())
                        .eval()
                },
            )?,
        )?;

        handler.call::<()>(ctx)
    })
}
