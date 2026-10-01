//! Lua bridge for calling named WASM components.

#[cfg(feature = "wasm")]
use std::collections::BTreeMap;
#[cfg(feature = "wasm")]
use std::sync::Mutex;

use mlua::{Lua, Table};

#[cfg(feature = "wasm")]
use mlua::{MultiValue, UserData, UserDataMethods, Value, Variadic};

#[cfg(feature = "wasm")]
use crate::host::{Phase, RuntimeAccess};

#[cfg(feature = "wasm")]
use rpp_wasm::{CompiledComponent, Value as WasmValue, ValueType, WasmInstance};

#[cfg(feature = "wasm")]
pub(crate) fn module(lua: &Lua, access: RuntimeAccess) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set(
        "load",
        lua.create_function(move |lua, name: String| {
            let component = access
                .components
                .get(&name)
                .ok_or_else(|| mlua::Error::external(format!("unknown component `{name}`")))?
                .clone();
            lua.create_userdata(ComponentHandle {
                name,
                component,
                access: access.clone(),
                instance: Mutex::new(None),
            })
        })?,
    )?;
    Ok(table)
}

#[cfg(not(feature = "wasm"))]
pub(crate) fn module(lua: &Lua, _access: crate::host::RuntimeAccess) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set(
        "load",
        lua.create_function(|_, name: String| {
            Err::<(), _>(mlua::Error::external(format!(
                "component `{name}` requires rpp built with the `wasm` feature"
            )))
        })?,
    )?;
    Ok(table)
}

#[cfg(feature = "wasm")]
struct ComponentHandle {
    name: String,
    component: CompiledComponent,
    access: RuntimeAccess,
    instance: Mutex<Option<WasmInstance>>,
}

#[cfg(feature = "wasm")]
impl ComponentHandle {
    fn call(&self, lua: &Lua, path: &str, args: Variadic<Value>) -> mlua::Result<MultiValue> {
        let schema = self
            .component
            .schema()
            .functions
            .iter()
            .find(|function| function.path == path)
            .ok_or_else(|| {
                mlua::Error::external(format!("component `{}` has no export `{path}`", self.name))
            })?;
        if args.len() != schema.params.len() {
            return Err(mlua::Error::external(format!(
                "component `{}` export `{path}` expects {} argument(s), got {}",
                self.name,
                schema.params.len(),
                args.len()
            )));
        }
        let params = args
            .into_iter()
            .zip(&schema.params)
            .map(|(value, (name, ty))| {
                lua_to_wasm(value, ty).map_err(|message| {
                    component_error(
                        &self.name,
                        path,
                        format!("invalid parameter `{name}`: {message}"),
                    )
                })
            })
            .collect::<mlua::Result<Vec<_>>>()?;

        let phase = self.access.phase.get();
        let results = if phase == Phase::Processor {
            // Processor calls are functions of one file. A fresh store prevents
            // guest globals and deterministic WASI streams from coupling output
            // to worker scheduling or the order in which files reach a worker.
            let mut instance = self.instantiate(path)?;
            instance.call(path, &params).map_err(|error| {
                component_error(&self.name, path, format!("call failed: {error}"))
            })?
        } else {
            let mut instance = self.instance.lock().map_err(|_| {
                mlua::Error::external(format!("component `{}` instance lock poisoned", self.name))
            })?;
            if instance.is_none() {
                *instance = Some(self.instantiate(path)?);
            }
            instance
                .as_mut()
                .expect("component instance exists")
                .call(path, &params)
                .map_err(|error| {
                    component_error(&self.name, path, format!("call failed: {error}"))
                })?
        };
        if results.len() != schema.results.len() {
            return Err(component_error(
                &self.name,
                path,
                format!(
                    "returned {} value(s), but its schema declares {}",
                    results.len(),
                    schema.results.len()
                ),
            ));
        }
        let mut out = MultiValue::new();
        for (index, (value, ty)) in results.into_iter().zip(&schema.results).enumerate() {
            out.push_back(wasm_to_lua(lua, value, ty).map_err(|message| {
                component_error(
                    &self.name,
                    path,
                    format!("invalid result {}: {message}", index + 1),
                )
            })?);
        }
        Ok(out)
    }

    fn instantiate(&self, path: &str) -> mlua::Result<WasmInstance> {
        self.component
            .instantiate(self.access.wasm_permissions())
            .map_err(|error| {
                component_error(&self.name, path, format!("failed to instantiate: {error}"))
            })
    }
}

#[cfg(feature = "wasm")]
fn component_error(name: &str, path: &str, message: impl std::fmt::Display) -> mlua::Error {
    mlua::Error::external(format!("component `{name}` export `{path}`: {message}"))
}

#[cfg(feature = "wasm")]
impl UserData for ComponentHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method(
            "call",
            |lua, this, (path, args): (String, Variadic<Value>)| this.call(lua, &path, args),
        );
        methods.add_method("schema", |lua, this, ()| {
            let table = lua.create_table()?;
            for (i, function) in this.component.schema().functions.iter().enumerate() {
                table.raw_set(i + 1, function.path.clone())?;
            }
            Ok(table)
        });
    }
}

#[cfg(feature = "wasm")]
fn lua_to_wasm(value: Value, ty: &ValueType) -> Result<WasmValue, String> {
    Ok(match ty {
        ValueType::Bool => match value {
            Value::Boolean(value) => WasmValue::Bool(value),
            other => return Err(expected("boolean", &other)),
        },
        ValueType::S8 => WasmValue::S8(integer_in_range::<i8>(value, "s8")?),
        ValueType::U8 => WasmValue::U8(integer_in_range::<u8>(value, "u8")?),
        ValueType::S16 => WasmValue::S16(integer_in_range::<i16>(value, "s16")?),
        ValueType::U16 => WasmValue::U16(integer_in_range::<u16>(value, "u16")?),
        ValueType::S32 => WasmValue::S32(integer_in_range::<i32>(value, "s32")?),
        ValueType::U32 => WasmValue::U32(integer_in_range::<u32>(value, "u32")?),
        ValueType::S64 => WasmValue::S64(as_integer(value)?),
        ValueType::U64 => WasmValue::U64(integer_in_range::<u64>(value, "u64")?),
        ValueType::Float32 => {
            let value = as_number(value)?;
            let narrowed = value as f32;
            if value.is_finite() && !narrowed.is_finite() {
                return Err(format!("number {value} is outside the range of float32"));
            }
            WasmValue::Float32(narrowed)
        }
        ValueType::Float64 => WasmValue::Float64(as_number(value)?),
        ValueType::Char => {
            let s = as_string(value)?;
            let mut chars = s.chars();
            let ch = chars.next().ok_or_else(|| {
                "char expects one Unicode scalar value, got an empty string".to_string()
            })?;
            if chars.next().is_some() {
                return Err("char expects one Unicode scalar value".into());
            }
            WasmValue::Char(ch)
        }
        ValueType::String => WasmValue::String(as_string(value)?),
        ValueType::List(inner) if matches!(inner.as_ref(), ValueType::U8) => match value {
            Value::String(s) => {
                WasmValue::List(s.as_bytes().iter().copied().map(WasmValue::U8).collect())
            }
            other => list_to_wasm(other, inner)?,
        },
        ValueType::List(inner) => list_to_wasm(value, inner)?,
        ValueType::Record(fields) => {
            let Value::Table(table) = value else {
                return Err(expected("record table", &value));
            };
            reject_unknown_record_fields(&table, fields)?;
            let mut out = Vec::with_capacity(fields.len());
            for (name, ty) in fields {
                let value = table
                    .get::<Value>(name.as_str())
                    .map_err(|error| format!("field `{name}` could not be read: {error}"))?;
                out.push((
                    name.clone(),
                    lua_to_wasm(value, ty)
                        .map_err(|message| format!("field `{name}`: {message}"))?,
                ));
            }
            WasmValue::Record(out)
        }
        ValueType::Tuple(types) => {
            let Value::Table(table) = value else {
                return Err(expected("tuple table", &value));
            };
            let length = sequence_length(&table)?;
            if length != types.len() {
                return Err(format!(
                    "tuple expects {} element(s), got {}",
                    types.len(),
                    length
                ));
            }
            let mut out = Vec::with_capacity(types.len());
            for (index, ty) in types.iter().enumerate() {
                let value = table.raw_get::<Value>(index + 1).map_err(|error| {
                    format!("tuple element {} could not be read: {error}", index + 1)
                })?;
                out.push(
                    lua_to_wasm(value, ty)
                        .map_err(|message| format!("tuple element {}: {message}", index + 1))?,
                );
            }
            WasmValue::Tuple(out)
        }
        ValueType::Variant(cases) => {
            let Value::Table(table) = value else {
                return Err(expected("variant table", &value));
            };
            let tag = table
                .get::<String>("tag")
                .map_err(|_| "variant requires a string `tag` field".to_string())?;
            let payload_type = cases
                .iter()
                .find(|(name, _)| name == &tag)
                .map(|(_, ty)| ty)
                .ok_or_else(|| invalid_case("variant", &tag, cases.iter().map(|(name, _)| name)))?;
            let value = table
                .get::<Value>("value")
                .map_err(|error| format!("variant payload could not be read: {error}"))?;
            let payload = match payload_type {
                Some(ty) => {
                    Some(Box::new(lua_to_wasm(value, ty).map_err(|message| {
                        format!("variant case `{tag}` payload: {message}")
                    })?))
                }
                None if value.is_nil() => None,
                None => return Err(format!("variant case `{tag}` does not accept a payload")),
            };
            WasmValue::Variant(tag, payload)
        }
        ValueType::Enum(cases) => {
            let case = as_string(value)?;
            if !cases.contains(&case) {
                return Err(invalid_case("enum", &case, cases.iter()));
            }
            WasmValue::Enum(case)
        }
        ValueType::Option(inner) => {
            let Value::Table(table) = value else {
                return Err(expected("tagged option table", &value));
            };
            let tag = table
                .raw_get::<String>("tag")
                .map_err(|_| "option requires a string `tag` field".to_string())?;
            let payload = table
                .raw_get::<Value>("value")
                .map_err(|error| error.to_string())?;
            match tag.as_str() {
                "none" if payload.is_nil() => WasmValue::Option(None),
                "none" => return Err("option `none` does not accept a payload".into()),
                "some" => WasmValue::Option(Some(Box::new(
                    lua_to_wasm(payload, inner)
                        .map_err(|message| format!("option `some` payload: {message}"))?,
                ))),
                _ => return Err("option tag must be `none` or `some`".into()),
            }
        }
        ValueType::Result { ok, err } => {
            let Value::Table(table) = value else {
                return Err(expected("result table", &value));
            };
            let ok_value = table
                .get::<Value>("ok")
                .map_err(|error| format!("result `ok` branch could not be read: {error}"))?;
            let err_value = table
                .get::<Value>("err")
                .map_err(|error| format!("result `err` branch could not be read: {error}"))?;
            match (ok_value.is_nil(), err_value.is_nil()) {
                (false, true) => WasmValue::Result(Ok(convert_result_payload(ok_value, ok, "ok")?)),
                (true, false) => {
                    WasmValue::Result(Err(convert_result_payload(err_value, err, "err")?))
                }
                (false, false) => {
                    return Err("result must contain exactly one of `ok` or `err`".into())
                }
                (true, true) => return Err("result must contain an `ok` or `err` field".into()),
            }
        }
        ValueType::Flags(allowed) => {
            let Value::Table(table) = value else {
                return Err(expected("flags list", &value));
            };
            let mut flags = Vec::new();
            for value in table.sequence_values::<String>() {
                let flag = value.map_err(|error| format!("flag must be a string: {error}"))?;
                if !allowed.contains(&flag) {
                    return Err(invalid_case("flag", &flag, allowed.iter()));
                }
                if flags.contains(&flag) {
                    return Err(format!("flag `{flag}` was specified more than once"));
                }
                flags.push(flag);
            }
            WasmValue::Flags(flags)
        }
        ValueType::Unsupported(kind) => {
            return Err(format!("unsupported component value type `{kind}`"))
        }
    })
}

#[cfg(feature = "wasm")]
fn sequence_length(table: &Table) -> Result<usize, String> {
    let length = table.raw_len();
    let mut count = 0;
    for pair in table.pairs::<Value, Value>() {
        let (key, _) = pair.map_err(|error| error.to_string())?;
        if !matches!(key, Value::Integer(index) if index > 0 && (index as u64) <= length as u64) {
            return Err("sequence must have exactly the integer keys 1..n (no holes)".into());
        }
        count += 1;
    }
    if count != length {
        return Err("sequence must have exactly the integer keys 1..n (no holes)".into());
    }
    Ok(length)
}

#[cfg(feature = "wasm")]
fn list_to_wasm(value: Value, inner: &ValueType) -> Result<WasmValue, String> {
    let Value::Table(table) = value else {
        return Err(expected("list table or byte string", &value));
    };
    let length = sequence_length(&table)?;
    let mut out = Vec::with_capacity(length);
    for index in 0..length {
        let value = table
            .raw_get::<Value>(index + 1)
            .map_err(|error| format!("list element {} could not be read: {error}", index + 1))?;
        out.push(
            lua_to_wasm(value, inner)
                .map_err(|message| format!("list element {}: {message}", index + 1))?,
        );
    }
    Ok(WasmValue::List(out))
}

#[cfg(feature = "wasm")]
fn wasm_to_lua(lua: &Lua, value: WasmValue, ty: &ValueType) -> Result<Value, String> {
    Ok(match (value, ty) {
        (WasmValue::Bool(value), ValueType::Bool) => Value::Boolean(value),
        (WasmValue::S8(value), ValueType::S8) => Value::Integer(value.into()),
        (WasmValue::U8(value), ValueType::U8) => Value::Integer(value.into()),
        (WasmValue::S16(value), ValueType::S16) => Value::Integer(value.into()),
        (WasmValue::U16(value), ValueType::U16) => Value::Integer(value.into()),
        (WasmValue::S32(value), ValueType::S32) => Value::Integer(value.into()),
        (WasmValue::U32(value), ValueType::U32) => Value::Integer(value.into()),
        (WasmValue::S64(value), ValueType::S64) => Value::Integer(value),
        (WasmValue::U64(value), ValueType::U64) => {
            Value::Integer(i64::try_from(value).map_err(|_| {
                format!("u64 value {value} cannot be represented exactly as a Lua integer")
            })?)
        }
        (WasmValue::Float32(value), ValueType::Float32) => Value::Number(value.into()),
        (WasmValue::Float64(value), ValueType::Float64) => Value::Number(value),
        (WasmValue::Char(value), ValueType::Char) => Value::String(
            lua.create_string(value.to_string())
                .map_err(|error| error.to_string())?,
        ),
        (WasmValue::String(value), ValueType::String) => Value::String(
            lua.create_string(value)
                .map_err(|error| error.to_string())?,
        ),
        (WasmValue::List(values), ValueType::List(inner))
            if matches!(inner.as_ref(), ValueType::U8) =>
        {
            let bytes = values
                .into_iter()
                .map(|value| match value {
                    WasmValue::U8(value) => Ok(value),
                    _ => Err("byte list contained a non-u8 value".to_string()),
                })
                .collect::<Result<Vec<_>, _>>()?;
            Value::String(
                lua.create_string(bytes)
                    .map_err(|error| error.to_string())?,
            )
        }
        (WasmValue::List(values), ValueType::List(inner)) => {
            let table = lua.create_table().map_err(|error| error.to_string())?;
            for (i, value) in values.into_iter().enumerate() {
                table
                    .raw_set(
                        i + 1,
                        wasm_to_lua(lua, value, inner)
                            .map_err(|message| format!("list element {}: {message}", i + 1))?,
                    )
                    .map_err(|error| error.to_string())?;
            }
            Value::Table(table)
        }
        (WasmValue::Record(fields), ValueType::Record(types)) => {
            let mut fields_by_name = BTreeMap::new();
            for (name, value) in fields {
                if fields_by_name.insert(name.clone(), value).is_some() {
                    return Err(format!(
                        "component returned duplicate record field `{name}`"
                    ));
                }
            }
            if fields_by_name.len() != types.len() {
                return Err(format!(
                    "component returned {} record field(s), expected {}",
                    fields_by_name.len(),
                    types.len()
                ));
            }
            let table = lua.create_table().map_err(|error| error.to_string())?;
            for (name, ty) in types {
                let value = fields_by_name
                    .remove(name)
                    .ok_or_else(|| format!("component omitted record field `{name}`"))?;
                table
                    .set(
                        name.clone(),
                        wasm_to_lua(lua, value, ty)
                            .map_err(|message| format!("field `{name}`: {message}"))?,
                    )
                    .map_err(|error| error.to_string())?;
            }
            Value::Table(table)
        }
        (WasmValue::Tuple(values), ValueType::Tuple(types)) => {
            if values.len() != types.len() {
                return Err(format!(
                    "component returned a tuple with {} element(s), expected {}",
                    values.len(),
                    types.len()
                ));
            }
            let table = lua.create_table().map_err(|error| error.to_string())?;
            for (i, (value, ty)) in values.into_iter().zip(types).enumerate() {
                table
                    .raw_set(
                        i + 1,
                        wasm_to_lua(lua, value, ty)
                            .map_err(|message| format!("tuple element {}: {message}", i + 1))?,
                    )
                    .map_err(|error| error.to_string())?;
            }
            Value::Table(table)
        }
        (WasmValue::Variant(tag, payload), ValueType::Variant(cases)) => {
            let payload_type = cases
                .iter()
                .find(|(name, _)| name == &tag)
                .map(|(_, ty)| ty)
                .ok_or_else(|| invalid_case("variant", &tag, cases.iter().map(|(name, _)| name)))?;
            let table = lua.create_table().map_err(|error| error.to_string())?;
            table
                .set("tag", tag.clone())
                .map_err(|error| error.to_string())?;
            match (payload, payload_type) {
                (Some(payload), Some(ty)) => table
                    .set(
                        "value",
                        wasm_to_lua(lua, *payload, ty).map_err(|message| {
                            format!("variant case `{tag}` payload: {message}")
                        })?,
                    )
                    .map_err(|error| error.to_string())?,
                (None, None) => {}
                (Some(_), None) => {
                    return Err(format!(
                        "variant case `{tag}` unexpectedly returned a payload"
                    ))
                }
                (None, Some(_)) => {
                    return Err(format!("variant case `{tag}` did not return its payload"))
                }
            }
            Value::Table(table)
        }
        (WasmValue::Enum(tag), ValueType::Enum(cases)) => {
            if !cases.contains(&tag) {
                return Err(invalid_case("enum", &tag, cases.iter()));
            }
            Value::String(lua.create_string(tag).map_err(|error| error.to_string())?)
        }
        (WasmValue::Option(value), ValueType::Option(inner)) => {
            let table = lua.create_table().map_err(|error| error.to_string())?;
            table
                .raw_set("tag", if value.is_some() { "some" } else { "none" })
                .map_err(|error| error.to_string())?;
            if let Some(value) = value {
                table
                    .raw_set("value", wasm_to_lua(lua, *value, inner)?)
                    .map_err(|error| error.to_string())?;
            }
            Value::Table(table)
        }
        (WasmValue::Result(result), ValueType::Result { ok, err }) => {
            let table = lua.create_table().map_err(|error| error.to_string())?;
            match result {
                Ok(value) => match (value, ok) {
                    (Some(value), Some(ty)) => table
                        .set(
                            "ok",
                            wasm_to_lua(lua, *value, ty)
                                .map_err(|message| format!("result `ok` payload: {message}"))?,
                        )
                        .map_err(|error| error.to_string())?,
                    (None, None) => table.set("ok", true).map_err(|error| error.to_string())?,
                    (Some(_), None) => return Err("void `ok` result returned a payload".into()),
                    (None, Some(_)) => return Err("`ok` result omitted its payload".into()),
                },
                Err(value) => match (value, err) {
                    (Some(value), Some(ty)) => table
                        .set(
                            "err",
                            wasm_to_lua(lua, *value, ty)
                                .map_err(|message| format!("result `err` payload: {message}"))?,
                        )
                        .map_err(|error| error.to_string())?,
                    (None, None) => table.set("err", true).map_err(|error| error.to_string())?,
                    (Some(_), None) => return Err("void `err` result returned a payload".into()),
                    (None, Some(_)) => return Err("`err` result omitted its payload".into()),
                },
            }
            Value::Table(table)
        }
        (WasmValue::Flags(flags), ValueType::Flags(allowed)) => {
            let table = lua.create_table().map_err(|error| error.to_string())?;
            for (i, flag) in flags.into_iter().enumerate() {
                if !allowed.contains(&flag) {
                    return Err(invalid_case("flag", &flag, allowed.iter()));
                }
                table
                    .raw_set(i + 1, flag)
                    .map_err(|error| error.to_string())?;
            }
            Value::Table(table)
        }
        (value, ty) => {
            return Err(format!(
                "component returned {}, expected {}",
                wasm_value_name(&value),
                value_type_name(ty)
            ))
        }
    })
}

#[cfg(feature = "wasm")]
fn as_integer(value: Value) -> Result<i64, String> {
    match value {
        Value::Integer(value) => Ok(value),
        other => Err(expected("integer", &other)),
    }
}

#[cfg(feature = "wasm")]
fn as_number(value: Value) -> Result<f64, String> {
    match value {
        Value::Integer(value) => Ok(value as f64),
        Value::Number(value) => Ok(value),
        other => Err(expected("number", &other)),
    }
}

#[cfg(feature = "wasm")]
fn as_string(value: Value) -> Result<String, String> {
    match value {
        Value::String(value) => value
            .to_str()
            .map(|value| value.to_string())
            .map_err(|_| "expected a UTF-8 string".into()),
        other => Err(expected("string", &other)),
    }
}

#[cfg(feature = "wasm")]
fn integer_in_range<T>(value: Value, wit_name: &str) -> Result<T, String>
where
    T: TryFrom<i64>,
{
    let value = as_integer(value)?;
    T::try_from(value).map_err(|_| format!("integer {value} is outside the range of {wit_name}"))
}

#[cfg(feature = "wasm")]
fn reject_unknown_record_fields(
    table: &Table,
    fields: &[(String, ValueType)],
) -> Result<(), String> {
    for pair in table.clone().pairs::<Value, Value>() {
        let (key, _) = pair.map_err(|error| format!("record field could not be read: {error}"))?;
        let Value::String(key) = key else {
            return Err(format!(
                "record field names must be strings, got {}",
                key.type_name()
            ));
        };
        let key = key
            .to_str()
            .map_err(|_| "record field name must be UTF-8".to_string())?;
        if !fields.iter().any(|(name, _)| name == key.as_ref()) {
            return Err(format!("record contains unknown field `{key}`"));
        }
    }
    Ok(())
}

#[cfg(feature = "wasm")]
fn convert_result_payload(
    value: Value,
    ty: &Option<Box<ValueType>>,
    branch: &str,
) -> Result<Option<Box<WasmValue>>, String> {
    match ty {
        Some(ty) => lua_to_wasm(value, ty)
            .map(Box::new)
            .map(Some)
            .map_err(|message| format!("result `{branch}` payload: {message}")),
        None if matches!(value, Value::Boolean(true)) => Ok(None),
        None => Err(format!(
            "void result `{branch}` must use boolean `true` as its branch marker"
        )),
    }
}

#[cfg(feature = "wasm")]
fn expected(expected: &str, actual: &Value) -> String {
    format!("expected {expected}, got {}", actual.type_name())
}

#[cfg(feature = "wasm")]
fn invalid_case<'a>(kind: &str, actual: &str, allowed: impl Iterator<Item = &'a String>) -> String {
    let allowed = allowed.map(|case| format!("`{case}`")).collect::<Vec<_>>();
    format!("unknown {kind} `{actual}`; expected {}", allowed.join(", "))
}

#[cfg(feature = "wasm")]
fn value_type_name(ty: &ValueType) -> &'static str {
    match ty {
        ValueType::Bool => "bool",
        ValueType::S8 => "s8",
        ValueType::U8 => "u8",
        ValueType::S16 => "s16",
        ValueType::U16 => "u16",
        ValueType::S32 => "s32",
        ValueType::U32 => "u32",
        ValueType::S64 => "s64",
        ValueType::U64 => "u64",
        ValueType::Float32 => "float32",
        ValueType::Float64 => "float64",
        ValueType::Char => "char",
        ValueType::String => "string",
        ValueType::List(_) => "list",
        ValueType::Record(_) => "record",
        ValueType::Tuple(_) => "tuple",
        ValueType::Variant(_) => "variant",
        ValueType::Enum(_) => "enum",
        ValueType::Option(_) => "option",
        ValueType::Result { .. } => "result",
        ValueType::Flags(_) => "flags",
        ValueType::Unsupported(_) => "unsupported value",
    }
}

#[cfg(feature = "wasm")]
fn wasm_value_name(value: &WasmValue) -> &'static str {
    match value {
        WasmValue::Bool(_) => "bool",
        WasmValue::S8(_) => "s8",
        WasmValue::U8(_) => "u8",
        WasmValue::S16(_) => "s16",
        WasmValue::U16(_) => "u16",
        WasmValue::S32(_) => "s32",
        WasmValue::U32(_) => "u32",
        WasmValue::S64(_) => "s64",
        WasmValue::U64(_) => "u64",
        WasmValue::Float32(_) => "float32",
        WasmValue::Float64(_) => "float64",
        WasmValue::Char(_) => "char",
        WasmValue::String(_) => "string",
        WasmValue::List(_) => "list",
        WasmValue::Record(_) => "record",
        WasmValue::Tuple(_) => "tuple",
        WasmValue::Variant(_, _) => "variant",
        WasmValue::Enum(_) => "enum",
        WasmValue::Option(_) => "option",
        WasmValue::Result(_) => "result",
        WasmValue::Flags(_) => "flags",
    }
}

#[cfg(all(test, feature = "wasm"))]
mod tests {
    use super::*;

    #[test]
    fn scalar_conversion_is_exact_and_range_checked() {
        assert_eq!(
            lua_to_wasm(Value::Boolean(false), &ValueType::Bool).unwrap(),
            WasmValue::Bool(false)
        );
        assert!(lua_to_wasm(Value::Integer(0), &ValueType::Bool)
            .unwrap_err()
            .contains("expected boolean"));
        assert_eq!(
            lua_to_wasm(Value::Integer(255), &ValueType::U8).unwrap(),
            WasmValue::U8(255)
        );
        assert!(lua_to_wasm(Value::Integer(256), &ValueType::U8)
            .unwrap_err()
            .contains("outside the range of u8"));
        assert!(lua_to_wasm(Value::Integer(-1), &ValueType::U64)
            .unwrap_err()
            .contains("outside the range of u64"));
        assert!(lua_to_wasm(Value::Number(1.0), &ValueType::S32)
            .unwrap_err()
            .contains("expected integer"));
    }

    #[test]
    fn variants_use_the_selected_payload_schema_in_both_directions() {
        let lua = Lua::new();
        let payload_type = ValueType::Record(vec![
            ("count".into(), ValueType::U8),
            ("contents".into(), ValueType::List(Box::new(ValueType::U8))),
        ]);
        let variant_type = ValueType::Variant(vec![
            ("empty".into(), None),
            ("data".into(), Some(payload_type.clone())),
        ]);

        let payload = lua.create_table().unwrap();
        payload.set("count", 2).unwrap();
        payload
            .set("contents", lua.create_string([0, 255]).unwrap())
            .unwrap();
        let variant = lua.create_table().unwrap();
        variant.set("tag", "data").unwrap();
        variant.set("value", payload).unwrap();

        let wasm = lua_to_wasm(Value::Table(variant), &variant_type).unwrap();
        assert_eq!(
            wasm,
            WasmValue::Variant(
                "data".into(),
                Some(Box::new(WasmValue::Record(vec![
                    ("count".into(), WasmValue::U8(2)),
                    (
                        "contents".into(),
                        WasmValue::List(vec![WasmValue::U8(0), WasmValue::U8(255)])
                    ),
                ])))
            )
        );

        let lua_value = wasm_to_lua(&lua, wasm, &variant_type).unwrap();
        let Value::Table(variant) = lua_value else {
            panic!("variant should become a table");
        };
        assert_eq!(variant.get::<String>("tag").unwrap(), "data");
        let payload = variant.get::<Table>("value").unwrap();
        assert_eq!(payload.get::<u8>("count").unwrap(), 2);
        assert_eq!(
            payload
                .get::<mlua::String>("contents")
                .unwrap()
                .as_bytes()
                .as_ref(),
            &[0, 255]
        );
    }

    #[test]
    fn structural_errors_include_nested_location() {
        let lua = Lua::new();
        let record_type = ValueType::Record(vec![(
            "items".into(),
            ValueType::List(Box::new(ValueType::U8)),
        )]);
        let record = lua.create_table().unwrap();
        let items = lua.create_table().unwrap();
        items.set(1, 300).unwrap();
        record.set("items", items).unwrap();

        let error = lua_to_wasm(Value::Table(record), &record_type).unwrap_err();
        assert!(error.contains("field `items`: list element 1"), "{error}");
        assert!(error.contains("outside the range of u8"), "{error}");
    }

    #[test]
    fn options_and_sequences_reject_ambiguous_input() {
        let lua = Lua::new();
        let option = ValueType::Option(Box::new(ValueType::Bool));
        for expression in [
            "nil",
            "false",
            "{}",
            "{tag='some'}",
            "{tag='none', value=false}",
        ] {
            let value = lua.load(format!("return {expression}")).eval().unwrap();
            assert!(lua_to_wasm(value, &option).is_err(), "{expression}");
        }
        for expression in ["{[1]=false,[3]=true}", "{[2]=false}", "{false,n=1}"] {
            let value: Value = lua.load(format!("return {expression}")).eval().unwrap();
            assert!(
                lua_to_wasm(value.clone(), &ValueType::List(Box::new(ValueType::Bool))).is_err()
            );
            assert!(lua_to_wasm(value, &ValueType::Tuple(vec![ValueType::Bool])).is_err());
        }
    }

    #[test]
    fn records_reject_unknown_fields() {
        let lua = Lua::new();
        let record = lua.create_table().unwrap();
        record.set("path", "a").unwrap();
        record.set("typo", true).unwrap();
        let ty = ValueType::Record(vec![("path".into(), ValueType::String)]);

        let error = lua_to_wasm(Value::Table(record), &ty).unwrap_err();
        assert!(error.contains("unknown field `typo`"), "{error}");
    }

    #[test]
    fn large_u64_result_is_not_silently_wrapped() {
        let lua = Lua::new();
        let error = wasm_to_lua(&lua, WasmValue::U64(u64::MAX), &ValueType::U64).unwrap_err();
        assert!(error.contains("cannot be represented exactly"), "{error}");
    }

    #[test]
    fn component_error_identifies_component_export_and_parameter() {
        let error = component_error(
            "compiler",
            "compile",
            "invalid parameter `files`: list element 2: expected record table",
        )
        .to_string();
        assert!(error.contains("component `compiler` export `compile`"));
        assert!(error.contains("parameter `files`"));
    }
}
