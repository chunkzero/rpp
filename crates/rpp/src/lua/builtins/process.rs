//! Trusted process execution builtin.

use std::path::PathBuf;
use std::time::Duration;

use mlua::{Lua, Table, Value};

use crate::lua::runtime::{Phase, RuntimeAccess};

pub(crate) fn module(lua: &Lua, access: RuntimeAccess) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set(
        "run",
        lua.create_function(move |lua, request: Table| -> mlua::Result<Table> {
            #[cfg(not(feature = "wasm"))]
            let _ = lua;
            if !access.allows_process() {
                return Err(mlua::Error::external(
                    "process execution requires trusted process permissions",
                ));
            }
            if !access.is_native() && !matches!(access.phase.get(), Phase::Generator | Phase::Hook)
            {
                return Err(mlua::Error::external(
                    "process execution is only available in generators and hooks",
                ));
            }
            let program: String = request.get("program").or_else(|_| request.get("tool"))?;
            let args = match request.get::<Value>("args").unwrap_or(Value::Nil) {
                Value::Nil => Vec::new(),
                Value::Table(table) => table
                    .sequence_values::<String>()
                    .collect::<Result<_, _>>()?,
                _ => return Err(mlua::Error::external("process args must be a table")),
            };
            let environment = match request.get::<Value>("env").unwrap_or(Value::Nil) {
                Value::Nil => Vec::new(),
                Value::Table(table) => {
                    let mut env = Vec::new();
                    for pair in table.pairs::<String, String>() {
                        env.push(pair?);
                    }
                    env
                }
                _ => return Err(mlua::Error::external("process env must be a table")),
            };
            let stdin = match request.get::<Value>("stdin").unwrap_or(Value::Nil) {
                Value::Nil => Vec::new(),
                Value::String(s) => s.as_bytes().to_vec(),
                _ => return Err(mlua::Error::external("process stdin must be a string")),
            };
            let timeout = match request.get::<Value>("timeout").unwrap_or(Value::Nil) {
                Value::Nil => None,
                Value::Integer(seconds) => Some(Duration::from_secs(seconds as u64)),
                Value::Number(seconds) => Some(Duration::from_secs_f64(seconds)),
                _ => return Err(mlua::Error::external("process timeout must be a number")),
            };
            let cwd = match request.get::<Value>("cwd").unwrap_or(Value::Nil) {
                Value::Nil => None,
                Value::String(s) => Some(PathBuf::from(s.to_str()?.as_ref())),
                _ => return Err(mlua::Error::external("process cwd must be a string")),
            };
            #[cfg(not(feature = "wasm"))]
            {
                let _ = (program, args, environment, stdin, timeout, cwd);
                return Err(mlua::Error::external(
                    "process execution requires rpp built with the `wasm` feature",
                ));
            }
            #[cfg(feature = "wasm")]
            {
                let mut permissions = access.wasm_permissions();
                permissions.working_directory = Some(access.project_root.clone());
                let output = rpp_wasm::run_process(
                    &permissions,
                    rpp_wasm::ProcessRequest {
                        program,
                        args,
                        cwd,
                        environment,
                        stdin,
                        timeout,
                    },
                )
                .map_err(mlua::Error::external)?;
                let out = lua.create_table()?;
                out.set("status", output.status)?;
                out.set("stdout", lua.create_string(output.stdout)?)?;
                out.set("stderr", lua.create_string(output.stderr)?)?;
                Ok(out)
            }
        })?,
    )?;
    Ok(table)
}
