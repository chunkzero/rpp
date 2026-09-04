//! Trusted process execution builtin.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use mlua::{Lua, Table, Value};

use crate::lua::runtime::{Phase, RuntimeAccess};
use crate::lua::sandbox::Deadline;

pub(crate) fn module(lua: &Lua, access: RuntimeAccess, deadline: Deadline) -> mlua::Result<Table> {
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
            let timeout = parse_timeout(request.get::<Value>("timeout").unwrap_or(Value::Nil))?;
            let remaining = deadline
                .lock()
                .and_then(|deadline| deadline.checked_duration_since(Instant::now()))
                .ok_or_else(|| mlua::Error::runtime("Lua execution deadline exceeded"))?;
            let timeout = Some(timeout.map_or(remaining, |requested| requested.min(remaining)));
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

fn parse_timeout(value: Value) -> mlua::Result<Option<Duration>> {
    let seconds = match value {
        Value::Nil => return Ok(None),
        Value::Integer(seconds) if seconds >= 0 => seconds as f64,
        Value::Number(seconds) if seconds.is_finite() && seconds >= 0.0 => seconds,
        Value::Integer(_) | Value::Number(_) => {
            return Err(mlua::Error::external(
                "process timeout must be finite and non-negative",
            ))
        }
        _ => return Err(mlua::Error::external("process timeout must be a number")),
    };
    Duration::try_from_secs_f64(seconds)
        .map(Some)
        .map_err(|_| mlua::Error::external("process timeout is too large"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_timeouts() {
        assert!(parse_timeout(Value::Integer(-1)).is_err());
        assert!(parse_timeout(Value::Number(f64::NAN)).is_err());
        assert!(parse_timeout(Value::Number(f64::INFINITY)).is_err());
    }

    #[cfg(all(feature = "wasm", unix))]
    #[test]
    fn process_is_bounded_by_lua_deadline() {
        use std::collections::BTreeMap;

        use crate::config::{PluginPermissions, SecurityMode};

        let lua = Lua::new();
        let deadline = std::sync::Arc::new(parking_lot::Mutex::new(Some(
            Instant::now() + Duration::from_millis(100),
        )));
        let access = RuntimeAccess::new(
            SecurityMode::Trusted,
            PluginPermissions {
                process: vec!["/bin/sh".into()],
                ..Default::default()
            },
            std::env::current_dir().unwrap(),
            BTreeMap::new(),
            BTreeMap::new(),
        );
        access.phase.set(Phase::Generator);
        let process = module(&lua, access, deadline).unwrap();
        let request = lua.create_table().unwrap();
        request.set("program", "/bin/sh").unwrap();
        request.set("args", vec!["-c", "sleep 2"]).unwrap();

        let started = Instant::now();
        let error = process
            .get::<mlua::Function>("run")
            .unwrap()
            .call::<Table>(request)
            .unwrap_err();
        assert!(error.to_string().contains("timed out"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
