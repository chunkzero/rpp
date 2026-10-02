//! The JSON descriptors of component exports sent in `component.load` replies.

use rpp_wasm::{Function, ValueType};
use serde_json::{json, Value};

/// The JSON descriptor of an export: `{ path, params: [[name, type]], results: [type] }`.
pub(super) fn function_json(function: &Function) -> Value {
    json!({
        "path": function.path,
        "params": function
            .params
            .iter()
            .map(|(name, ty)| json!([name, type_json(ty)]))
            .collect::<Vec<_>>(),
        "results": function.results.iter().map(type_json).collect::<Vec<_>>(),
    })
}

/// The JSON descriptor of a value type.
fn type_json(ty: &ValueType) -> Value {
    let optional = |ty: Option<&ValueType>| ty.map_or(Value::Null, type_json);
    match ty {
        ValueType::Bool => json!("bool"),
        ValueType::S8 => json!("s8"),
        ValueType::U8 => json!("u8"),
        ValueType::S16 => json!("s16"),
        ValueType::U16 => json!("u16"),
        ValueType::S32 => json!("s32"),
        ValueType::U32 => json!("u32"),
        ValueType::S64 => json!("s64"),
        ValueType::U64 => json!("u64"),
        ValueType::Float32 => json!("float32"),
        ValueType::Float64 => json!("float64"),
        ValueType::Char => json!("char"),
        ValueType::String => json!("string"),
        ValueType::List(inner) => json!({ "list": type_json(inner) }),
        ValueType::Record(fields) => {
            json!({ "record": fields.iter().map(|(n, t)| json!([n, type_json(t)])).collect::<Vec<_>>() })
        }
        ValueType::Tuple(types) => {
            json!({ "tuple": types.iter().map(type_json).collect::<Vec<_>>() })
        }
        ValueType::Variant(cases) => {
            json!({ "variant": cases.iter().map(|(n, t)| json!([n, optional(t.as_ref())])).collect::<Vec<_>>() })
        }
        ValueType::Enum(cases) => json!({ "enum": cases }),
        ValueType::Option(inner) => json!({ "option": type_json(inner) }),
        ValueType::Result { ok, err } => {
            json!({ "result": { "ok": optional(ok.as_deref()), "err": optional(err.as_deref()) } })
        }
        ValueType::Flags(names) => json!({ "flags": names }),
        ValueType::Unsupported(kind) => json!({ "unsupported": kind }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_json_covers_every_value_type() {
        let u = Box::new(ValueType::U32);
        let cases = [
            (ValueType::Bool, json!("bool")),
            (ValueType::S8, json!("s8")),
            (ValueType::U8, json!("u8")),
            (ValueType::S16, json!("s16")),
            (ValueType::U16, json!("u16")),
            (ValueType::S32, json!("s32")),
            (ValueType::U32, json!("u32")),
            (ValueType::S64, json!("s64")),
            (ValueType::U64, json!("u64")),
            (ValueType::Float32, json!("float32")),
            (ValueType::Float64, json!("float64")),
            (ValueType::Char, json!("char")),
            (ValueType::String, json!("string")),
            (ValueType::List(u.clone()), json!({ "list": "u32" })),
            (
                ValueType::Record(vec![("a-b".into(), ValueType::Bool)]),
                json!({ "record": [["a-b", "bool"]] }),
            ),
            (
                ValueType::Tuple(vec![ValueType::Char]),
                json!({ "tuple": ["char"] }),
            ),
            (
                ValueType::Variant(vec![("x".into(), None), ("y".into(), Some(ValueType::U8))]),
                json!({ "variant": [["x", null], ["y", "u8"]] }),
            ),
            (ValueType::Enum(vec!["a".into()]), json!({ "enum": ["a"] })),
            (ValueType::Option(u.clone()), json!({ "option": "u32" })),
            (
                ValueType::Result {
                    ok: None,
                    err: Some(u),
                },
                json!({ "result": { "ok": null, "err": "u32" } }),
            ),
            (
                ValueType::Flags(vec!["r".into()]),
                json!({ "flags": ["r"] }),
            ),
            (
                ValueType::Unsupported("resource".into()),
                json!({ "unsupported": "resource" }),
            ),
        ];
        for (ty, expected) in cases {
            assert_eq!(type_json(&ty), expected);
        }
    }
}
