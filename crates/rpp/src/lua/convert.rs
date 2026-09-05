//! Conversions between Lua values and serde data models.
//!
//! mlua's built-in serde support is convenient for decoding into Lua, but
//! encoding Lua tables back out is ambiguous (sequence vs map, integer vs float).
//! These helpers apply consistent rules:
//!
//! - A table whose keys are exactly `1..=n` becomes a JSON/TOML array.
//! - An empty table becomes an empty array.
//! - Any other table becomes an object, with non-string keys stringified.

use mlua::Value;

/// Convert a Lua value into a `serde_json::Value`.
pub(crate) fn lua_to_json(value: Value) -> Result<serde_json::Value, String> {
    match value {
        Value::Nil => Ok(serde_json::Value::Null),
        Value::Boolean(b) => Ok(serde_json::Value::Bool(b)),
        Value::Integer(i) => Ok(serde_json::Value::Number(i.into())),
        Value::Number(n) => serde_json::Number::from_f64(n)
            .map(serde_json::Value::Number)
            .ok_or_else(|| "non-finite number cannot be encoded".to_string()),
        Value::String(s) => Ok(serde_json::Value::String(
            s.to_str().map_err(|e| e.to_string())?.to_string(),
        )),
        Value::Table(t) => table_to_json(t),
        other => Err(format!("cannot encode Lua {} value", other.type_name())),
    }
}

fn table_to_json(table: mlua::Table) -> Result<serde_json::Value, String> {
    let len = table.raw_len();
    let mut is_array = len > 0;
    let mut count = 0usize;
    // Verify the table is a clean 1..=len sequence to treat it as an array.
    if is_array {
        for pair in table.clone().pairs::<Value, Value>() {
            let (k, _) = pair.map_err(|e| e.to_string())?;
            count += 1;
            match k {
                Value::Integer(i) if i >= 1 && (i as usize) <= len => {}
                _ => {
                    is_array = false;
                    break;
                }
            }
        }
        if is_array && count != len {
            is_array = false;
        }
    } else {
        // Empty table => empty array.
        if table.clone().pairs::<Value, Value>().next().is_none() {
            return Ok(serde_json::Value::Array(Vec::new()));
        }
    }

    if is_array {
        let mut arr = Vec::with_capacity(len);
        for i in 1..=len {
            let v: Value = table.raw_get(i).map_err(|e| e.to_string())?;
            arr.push(lua_to_json(v)?);
        }
        Ok(serde_json::Value::Array(arr))
    } else {
        let mut map = serde_json::Map::new();
        for pair in table.pairs::<Value, Value>() {
            let (k, v) = pair.map_err(|e| e.to_string())?;
            let key = match k {
                Value::String(s) => s.to_str().map_err(|e| e.to_string())?.to_string(),
                Value::Integer(i) => i.to_string(),
                Value::Number(n) => n.to_string(),
                other => {
                    return Err(format!(
                        "cannot use Lua {} as object key",
                        other.type_name()
                    ))
                }
            };
            map.insert(key, lua_to_json(v)?);
        }
        Ok(serde_json::Value::Object(map))
    }
}

/// Convert a Lua value into a `toml::Value`.
///
/// TOML has no null; nil/null values are omitted from tables and rejected at the
/// top level.
pub(crate) fn lua_to_toml(value: Value) -> Result<toml::Value, String> {
    let json = lua_to_json(value)?;
    json_to_toml(json)
}

fn json_to_toml(value: serde_json::Value) -> Result<toml::Value, String> {
    match value {
        serde_json::Value::Null => Err("TOML cannot represent nil".to_string()),
        serde_json::Value::Bool(b) => Ok(toml::Value::Boolean(b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(toml::Value::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(toml::Value::Float(f))
            } else {
                Err("number out of range for TOML".to_string())
            }
        }
        serde_json::Value::String(s) => Ok(toml::Value::String(s)),
        serde_json::Value::Array(a) => {
            let mut out = Vec::with_capacity(a.len());
            for v in a {
                if matches!(v, serde_json::Value::Null) {
                    continue;
                }
                out.push(json_to_toml(v)?);
            }
            Ok(toml::Value::Array(out))
        }
        serde_json::Value::Object(o) => {
            let mut table = toml::map::Map::new();
            for (k, v) in o {
                if matches!(v, serde_json::Value::Null) {
                    continue;
                }
                table.insert(k, json_to_toml(v)?);
            }
            Ok(toml::Value::Table(table))
        }
    }
}
