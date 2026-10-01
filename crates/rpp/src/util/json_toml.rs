//! Explicit JSON/TOML conversions. Serde-based conversion between the two is avoided
//! because dependencies may enable `serde_json/arbitrary_precision`, which changes how
//! numbers deserialize.

#![cfg_attr(not(feature = "js"), allow(dead_code))]

use serde_json::Value;

/// The single key of the object that stands for a TOML datetime in tagged JSON.
const DATETIME_KEY: &str = "$__toml_private_datetime";

pub(crate) fn toml_to_json(value: &toml::Value) -> Value {
    match value {
        toml::Value::String(s) => Value::String(s.clone()),
        toml::Value::Integer(i) => Value::from(*i),
        toml::Value::Float(f) => Value::from(*f),
        toml::Value::Boolean(b) => Value::Bool(*b),
        toml::Value::Datetime(d) => Value::String(d.to_string()),
        toml::Value::Array(items) => Value::Array(items.iter().map(toml_to_json).collect()),
        toml::Value::Table(table) => Value::Object(
            table
                .iter()
                .map(|(key, value)| (key.clone(), toml_to_json(value)))
                .collect(),
        ),
    }
}

pub(crate) fn json_to_toml(value: &Value) -> Result<toml::Value, String> {
    Ok(match value {
        Value::Null => return Err("null has no TOML representation".into()),
        Value::Bool(b) => toml::Value::Boolean(*b),
        Value::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => toml::Value::Integer(i),
            (None, Some(f)) => toml::Value::Float(f),
            (None, None) => return Err(format!("number {n} is out of range")),
        },
        Value::String(s) => toml::Value::String(s.clone()),
        Value::Array(items) => {
            toml::Value::Array(items.iter().map(json_to_toml).collect::<Result<_, _>>()?)
        }
        Value::Object(map) => match (map.len(), map.get(DATETIME_KEY)) {
            (1, Some(Value::String(text))) => toml::Value::Datetime(
                text.parse()
                    .map_err(|e| format!("invalid datetime `{text}`: {e}"))?,
            ),
            _ => toml::Value::Table(
                map.iter()
                    .map(|(key, value)| Ok((key.clone(), json_to_toml(value)?)))
                    .collect::<Result<_, String>>()?,
            ),
        },
    })
}
