//! Bounded conversions between Lua and the shared serde JSON data model.

use std::collections::HashSet;

use mlua::{Lua, LuaSerdeExt, Table, Value};
use serde::Deserialize;

const MAX_DEPTH: usize = 64;
const MAX_VALUES: usize = 100_000;
const SHAPE: &str = "__rpp_serde_shape";

/// Mark a table's semantic identity independently of its current contents.
pub(crate) fn mark(lua: &Lua, table: Table, array: bool) -> mlua::Result<Table> {
    let name = if array {
        "rpp.serde.array"
    } else {
        "rpp.serde.object"
    };
    let metatable = match lua.named_registry_value::<Table>(name) {
        Ok(table) => table,
        Err(_) => {
            let table = lua.create_table()?;
            table.raw_set(SHAPE, array)?;
            table.raw_set("__metatable", name)?;
            lua.set_named_registry_value(name, table.clone())?;
            table
        }
    };
    table.set_metatable(Some(metatable))?;
    Ok(table)
}

pub(crate) fn constructors(lua: &Lua, module: &Table) -> mlua::Result<()> {
    for (name, array) in [("array", true), ("object", false)] {
        module.set(
            name,
            lua.create_function(move |lua, table: Option<Table>| {
                mark(lua, table.map_or_else(|| lua.create_table(), Ok)?, array)
            })?,
        )?;
    }
    Ok(())
}

#[derive(Default)]
struct Budget {
    values: usize,
    active: HashSet<*const std::ffi::c_void>,
}

impl Budget {
    fn visit(&mut self, depth: usize) -> Result<(), String> {
        if depth > MAX_DEPTH {
            return Err(format!("maximum nesting depth ({MAX_DEPTH}) exceeded"));
        }
        self.values += 1;
        if self.values > MAX_VALUES {
            return Err(format!("maximum value count ({MAX_VALUES}) exceeded"));
        }
        Ok(())
    }
}

/// Decode using the same shapes and null representation consumed by the encoder.
pub(crate) fn json_to_lua(lua: &Lua, value: &serde_json::Value) -> mlua::Result<Value> {
    fn convert(
        lua: &Lua,
        value: &serde_json::Value,
        depth: usize,
        budget: &mut Budget,
    ) -> mlua::Result<Value> {
        budget.visit(depth).map_err(mlua::Error::external)?;
        match value {
            serde_json::Value::Array(values) => {
                let table = mark(lua, lua.create_table()?, true)?;
                for (index, value) in values.iter().enumerate() {
                    table.raw_set(index + 1, convert(lua, value, depth + 1, budget)?)?;
                }
                Ok(Value::Table(table))
            }
            serde_json::Value::Object(values) => {
                let table = mark(lua, lua.create_table()?, false)?;
                for (key, value) in values {
                    table.raw_set(key.as_str(), convert(lua, value, depth + 1, budget)?)?;
                }
                Ok(Value::Table(table))
            }
            _ => lua.to_value(value),
        }
    }
    convert(lua, value, 0, &mut Budget::default())
}

/// Validate TOML before its serde conversion, which also represents datetimes.
pub(crate) fn toml_to_lua(lua: &Lua, value: &toml::Value) -> mlua::Result<Value> {
    fn validate(value: &toml::Value, depth: usize, budget: &mut Budget) -> Result<(), String> {
        budget.visit(depth)?;
        match value {
            toml::Value::Array(values) => {
                for value in values {
                    validate(value, depth + 1, budget)?;
                }
            }
            toml::Value::Table(values) => {
                for value in values.values() {
                    validate(value, depth + 1, budget)?;
                }
            }
            toml::Value::Float(value) if !value.is_finite() => {
                return Err("non-finite number cannot be converted".to_string());
            }
            _ => {}
        }
        Ok(())
    }
    validate(value, 0, &mut Budget::default()).map_err(mlua::Error::external)?;
    let value = serde_json::to_value(value).map_err(mlua::Error::external)?;
    json_to_lua(lua, &value)
}

/// Encode marked containers; plain tables use string-keyed objects or dense arrays.
pub(crate) fn lua_to_json(value: Value) -> Result<serde_json::Value, String> {
    convert(value, 0, &mut Budget::default())
}

fn convert(value: Value, depth: usize, budget: &mut Budget) -> Result<serde_json::Value, String> {
    budget.visit(depth)?;
    match value {
        Value::Nil => Ok(serde_json::Value::Null),
        Value::LightUserData(p) if p.0.is_null() => Ok(serde_json::Value::Null),
        Value::Boolean(b) => Ok(serde_json::Value::Bool(b)),
        Value::Integer(i) => Ok(serde_json::Value::Number(i.into())),
        Value::Number(n) => serde_json::Number::from_f64(n)
            .map(serde_json::Value::Number)
            .ok_or_else(|| "non-finite number cannot be encoded".to_string()),
        Value::String(s) => Ok(serde_json::Value::String(
            s.to_str().map_err(|e| e.to_string())?.to_string(),
        )),
        Value::Table(table) => {
            let pointer = table.to_pointer();
            if !budget.active.insert(pointer) {
                return Err("cyclic table cannot be encoded".to_string());
            }
            let result = table_to_json(table, depth, budget);
            budget.active.remove(&pointer);
            result
        }
        other => Err(format!("cannot encode Lua {} value", other.type_name())),
    }
}

fn table_to_json(
    table: Table,
    depth: usize,
    budget: &mut Budget,
) -> Result<serde_json::Value, String> {
    let shape = table
        .metatable()
        .map(|m| m.raw_get::<Option<bool>>(SHAPE))
        .transpose()
        .map_err(|e| e.to_string())?
        .flatten();
    let mut entries = Vec::new();
    for pair in table.pairs::<Value, Value>() {
        if entries.len() >= MAX_VALUES - budget.values {
            return Err(format!("maximum value count ({MAX_VALUES}) exceeded"));
        }
        entries.push(pair.map_err(|e| e.to_string())?);
    }
    let dense = !entries.is_empty() && entries.iter().all(|(key, _)| {
        matches!(key, Value::Integer(i) if *i >= 1 && (*i as u64) <= entries.len() as u64)
    });
    if shape.unwrap_or(dense) {
        if !entries.is_empty() && !dense {
            return Err("array must have only consecutive integer keys starting at 1".to_string());
        }
        entries.sort_unstable_by_key(|(key, _)| match key {
            Value::Integer(i) => *i,
            _ => 0,
        });
        entries
            .into_iter()
            .map(|(_, value)| convert(value, depth + 1, budget))
            .collect::<Result<Vec<_>, _>>()
            .map(serde_json::Value::Array)
    } else {
        let mut object = serde_json::Map::new();
        for (key, value) in entries {
            let Value::String(key) = key else {
                return Err("object keys must be strings (arrays must be dense)".to_string());
            };
            object.insert(
                key.to_str().map_err(|e| e.to_string())?.to_string(),
                convert(value, depth + 1, budget)?,
            );
        }
        Ok(serde_json::Value::Object(object))
    }
}

/// TOML uses the same container contract and rejects null, which it cannot represent.
pub(crate) fn lua_to_toml(value: Value) -> Result<toml::Value, String> {
    toml::Value::deserialize(lua_to_json(value)?).map_err(|e| e.to_string())
}
