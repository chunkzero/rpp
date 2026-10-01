//! A JSON value that keeps object keys in document order, for editing user files
//! without reordering them.

use std::fmt;

use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{Serialize, SerializeMap, Serializer};
use serde_json::Number;

const NUMBER_TOKEN: &str = "$serde_json::private::Number";

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Json {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<Json>),
    Object(Object),
}

/// An insertion-ordered JSON object.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Object(Vec<(String, Json)>);

impl Object {
    pub(crate) fn get_mut(&mut self, key: &str) -> Option<&mut Json> {
        self.0.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Replace the value for `key` in place, or append it.
    pub(crate) fn insert(&mut self, key: String, value: Json) {
        match self.get_mut(&key) {
            Some(slot) => *slot = value,
            None => self.0.push((key, value)),
        }
    }

    pub(crate) fn remove(&mut self, key: &str) -> Option<Json> {
        let index = self.0.iter().position(|(k, _)| k == key)?;
        Some(self.0.remove(index).1)
    }

    /// The value for `key`, inserting `default()` at the end when missing.
    pub(crate) fn entry_or_insert_with(
        &mut self,
        key: &str,
        default: impl FnOnce() -> Json,
    ) -> &mut Json {
        let index = match self.0.iter().position(|(k, _)| k == key) {
            Some(index) => index,
            None => {
                self.0.push((key.to_string(), default()));
                self.0.len() - 1
            }
        };
        &mut self.0[index].1
    }
}

impl Serialize for Json {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Json::Null => serializer.serialize_unit(),
            Json::Bool(b) => serializer.serialize_bool(*b),
            Json::Number(n) => n.serialize(serializer),
            Json::String(s) => serializer.serialize_str(s),
            Json::Array(items) => items.serialize(serializer),
            Json::Object(object) => object.serialize(serializer),
        }
    }
}

impl Serialize for Object {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (key, value) in &self.0 {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Json {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct JsonVisitor;

        impl<'de> Visitor<'de> for JsonVisitor {
            type Value = Json;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("any JSON value")
            }

            fn visit_unit<E>(self) -> Result<Json, E> {
                Ok(Json::Null)
            }

            fn visit_bool<E>(self, v: bool) -> Result<Json, E> {
                Ok(Json::Bool(v))
            }

            fn visit_i64<E>(self, v: i64) -> Result<Json, E> {
                Ok(Json::Number(v.into()))
            }

            fn visit_u64<E>(self, v: u64) -> Result<Json, E> {
                Ok(Json::Number(v.into()))
            }

            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Json, E> {
                Number::from_f64(v)
                    .map(Json::Number)
                    .ok_or_else(|| E::custom("non-finite number"))
            }

            fn visit_str<E>(self, v: &str) -> Result<Json, E> {
                Ok(Json::String(v.to_string()))
            }

            fn visit_string<E>(self, v: String) -> Result<Json, E> {
                Ok(Json::String(v))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Json, A::Error> {
                let mut items = Vec::new();
                while let Some(item) = seq.next_element()? {
                    items.push(item);
                }
                Ok(Json::Array(items))
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Json, A::Error> {
                let mut object = Object::default();
                while let Some(key) = map.next_key::<String>()? {
                    // With `serde_json/arbitrary_precision`, numbers arrive as this one-entry map.
                    if key == NUMBER_TOKEN && object.0.is_empty() {
                        let text: String = map.next_value()?;
                        return text
                            .parse()
                            .map(Json::Number)
                            .map_err(serde::de::Error::custom);
                    }
                    let value = map.next_value()?;
                    object.insert(key, value);
                }
                Ok(Json::Object(object))
            }
        }

        deserializer.deserialize_any(JsonVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_in_document_order() {
        let text = r#"{"z":1,"a":{"y":[true,null,"s"],"b":-2.5},"m":{}}"#;
        let value: Json = serde_json::from_str(text).unwrap();
        assert_eq!(serde_json::to_string(&value).unwrap(), text);
    }
}
