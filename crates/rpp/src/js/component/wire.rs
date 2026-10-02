//! The wire encoding of component values exchanged with `sdk/components.ts`.

use rpp_wasm::{Function, Value as WasmValue, ValueType};
use serde_json::{json, Map, Value};

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

fn int<T: TryFrom<i64>>(value: &Value, name: &str) -> Result<T, String> {
    value
        .as_i64()
        .and_then(|n| T::try_from(n).ok())
        .ok_or_else(|| format!("expected a {name} integer, got {value}"))
}

fn decimal<T: std::str::FromStr>(value: &Value, name: &str) -> Result<T, String> {
    value
        .as_str()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("expected a {name} decimal string, got {value}"))
}

fn float(value: &Value) -> Result<f64, String> {
    match value {
        Value::Number(n) => n.as_f64().ok_or_else(|| format!("invalid float {value}")),
        Value::String(s) => match s.as_str() {
            "NaN" => Ok(f64::NAN),
            "Infinity" => Ok(f64::INFINITY),
            "-Infinity" => Ok(f64::NEG_INFINITY),
            "-0" => Ok(-0.0),
            _ => Err(format!("invalid float {value}")),
        },
        _ => Err(format!("expected a float, got {value}")),
    }
}

fn array<'a>(value: &'a Value, what: &str) -> Result<&'a [Value], String> {
    value
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| format!("expected {what} array, got {value}"))
}

fn object<'a>(value: &'a Value, what: &str) -> Result<&'a Map<String, Value>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("expected a {what} object, got {value}"))
}

fn string<'a>(value: &'a Value, what: &str) -> Result<&'a str, String> {
    value
        .as_str()
        .ok_or_else(|| format!("expected {what}, got {value}"))
}

fn payload<'a>(
    value: Option<&'a Value>,
    ty: Option<&ValueType>,
) -> Result<Option<&'a Value>, String> {
    let value = value.filter(|v| !v.is_null());
    match (value, ty) {
        (Some(v), Some(_)) => Ok(Some(v)),
        (None, None) => Ok(None),
        (None, Some(_)) => Err("missing payload".into()),
        (Some(_), None) => Err("unexpected payload".into()),
    }
}

fn boxed(
    value: Option<&Value>,
    ty: Option<&ValueType>,
    bytes: &[u8],
) -> Result<Option<Box<WasmValue>>, String> {
    match (value, ty) {
        (Some(v), Some(ty)) => Ok(Some(Box::new(from_wire(v, ty, bytes)?))),
        _ => Ok(None),
    }
}

fn case<'a, T>(cases: &'a [(String, T)], name: &str, what: &str) -> Result<&'a T, String> {
    cases
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, t)| t)
        .ok_or_else(|| format!("unknown {what} case `{name}`"))
}

/// Convert wire JSON to a component value, validating it against `ty`.
pub(super) fn from_wire(v: &Value, ty: &ValueType, bytes: &[u8]) -> Result<WasmValue, String> {
    Ok(match ty {
        ValueType::Bool => WasmValue::Bool(
            v.as_bool()
                .ok_or_else(|| format!("expected a boolean, got {v}"))?,
        ),
        ValueType::S8 => WasmValue::S8(int(v, "s8")?),
        ValueType::U8 => WasmValue::U8(int(v, "u8")?),
        ValueType::S16 => WasmValue::S16(int(v, "s16")?),
        ValueType::U16 => WasmValue::U16(int(v, "u16")?),
        ValueType::S32 => WasmValue::S32(int(v, "s32")?),
        ValueType::U32 => WasmValue::U32(int(v, "u32")?),
        ValueType::S64 => WasmValue::S64(decimal(v, "s64")?),
        ValueType::U64 => WasmValue::U64(decimal(v, "u64")?),
        ValueType::Float32 => float32_from_wire(v)?,
        ValueType::Float64 => WasmValue::Float64(float(v)?),
        ValueType::Char => char_from_wire(v)?,
        ValueType::String => WasmValue::String(string(v, "a string")?.to_string()),
        ValueType::List(inner) if **inner == ValueType::U8 => bytes_from_wire(v, bytes)?,
        ValueType::List(inner) => WasmValue::List(
            array(v, "list")?
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    from_wire(e, inner, bytes).map_err(|m| format!("list element {i}: {m}"))
                })
                .collect::<Result<_, _>>()?,
        ),
        ValueType::Record(fields) => record_from_wire(v, fields, bytes)?,
        ValueType::Tuple(types) => tuple_from_wire(v, types, bytes)?,
        ValueType::Variant(cases) => variant_from_wire(v, cases, bytes)?,
        ValueType::Enum(cases) => {
            let name = string(v, "an enum string")?;
            if !cases.iter().any(|c| c == name) {
                return Err(format!("unknown enum case `{name}`"));
            }
            WasmValue::Enum(name.to_string())
        }
        ValueType::Option(inner) => match array(v, "option")? {
            [] => WasmValue::Option(None),
            [some] => WasmValue::Option(Some(Box::new(from_wire(some, inner, bytes)?))),
            _ => return Err("option expects [] or [value]".into()),
        },
        ValueType::Result { ok, err } => result_from_wire(v, ok.as_deref(), err.as_deref(), bytes)?,
        ValueType::Flags(allowed) => flags_from_wire(v, allowed)?,
        ValueType::Unsupported(kind) => {
            return Err(format!("unsupported component value type `{kind}`"))
        }
    })
}

fn float32_from_wire(v: &Value) -> Result<WasmValue, String> {
    let wide = float(v)?;
    let narrow = wide as f32;
    if wide.is_finite() && !narrow.is_finite() {
        return Err(format!("number {wide} is outside the range of float32"));
    }
    Ok(WasmValue::Float32(narrow))
}

fn char_from_wire(v: &Value) -> Result<WasmValue, String> {
    let mut chars = string(v, "a char string")?.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Ok(WasmValue::Char(c)),
        _ => Err("char expects one Unicode scalar value".into()),
    }
}

/// A byte list, sent as `[offset, length]` into the call's attached bytes.
fn bytes_from_wire(v: &Value, bytes: &[u8]) -> Result<WasmValue, String> {
    let [offset, len] = array(v, "[offset, length]")? else {
        return Err("byte list expects [offset, length]".into());
    };
    let offset = offset
        .as_u64()
        .ok_or("byte list offset must be a non-negative integer")?;
    let len = len
        .as_u64()
        .ok_or("byte list length must be a non-negative integer")?;
    let end = offset
        .checked_add(len)
        .filter(|end| *end <= bytes.len() as u64)
        .ok_or_else(|| {
            format!(
                "byte range {offset}+{len} exceeds the {} attached byte(s)",
                bytes.len()
            )
        })?;
    Ok(WasmValue::List(
        bytes[offset as usize..end as usize]
            .iter()
            .copied()
            .map(WasmValue::U8)
            .collect(),
    ))
}

fn record_from_wire(
    v: &Value,
    fields: &[(String, ValueType)],
    bytes: &[u8],
) -> Result<WasmValue, String> {
    let object = object(v, "record")?;
    if let Some(extra) = object.keys().find(|k| !fields.iter().any(|(n, _)| n == *k)) {
        return Err(format!("unknown record field `{extra}`"));
    }
    let fields = fields
        .iter()
        .map(|(name, ty)| {
            let field = object
                .get(name)
                .ok_or_else(|| format!("missing record field `{name}`"))?;
            let field = from_wire(field, ty, bytes).map_err(|m| format!("field `{name}`: {m}"))?;
            Ok((name.clone(), field))
        })
        .collect::<Result<_, String>>()?;
    Ok(WasmValue::Record(fields))
}

fn tuple_from_wire(v: &Value, types: &[ValueType], bytes: &[u8]) -> Result<WasmValue, String> {
    let items = array(v, "tuple")?;
    if items.len() != types.len() {
        return Err(format!(
            "tuple expects {} element(s), got {}",
            types.len(),
            items.len()
        ));
    }
    let items = items
        .iter()
        .zip(types)
        .enumerate()
        .map(|(i, (e, t))| from_wire(e, t, bytes).map_err(|m| format!("tuple element {i}: {m}")))
        .collect::<Result<_, _>>()?;
    Ok(WasmValue::Tuple(items))
}

fn variant_from_wire(
    v: &Value,
    cases: &[(String, Option<ValueType>)],
    bytes: &[u8],
) -> Result<WasmValue, String> {
    let object = object(v, "variant")?;
    let tag = object
        .get("tag")
        .and_then(Value::as_str)
        .ok_or("variant requires a string `tag`")?;
    let ty = case(cases, tag, "variant")?;
    let value = payload(object.get("val"), ty.as_ref())
        .map_err(|m| format!("variant case `{tag}`: {m}"))?;
    let value =
        boxed(value, ty.as_ref(), bytes).map_err(|m| format!("variant case `{tag}`: {m}"))?;
    Ok(WasmValue::Variant(tag.to_string(), value))
}

fn result_from_wire(
    v: &Value,
    ok: Option<&ValueType>,
    err: Option<&ValueType>,
    bytes: &[u8],
) -> Result<WasmValue, String> {
    let object = object(v, "result")?;
    let (branch, ty) = match (object.get("ok"), object.get("err")) {
        (Some(v), None) => (v, ok),
        (None, Some(v)) => (v, err),
        _ => return Err("result must contain exactly one of `ok` or `err`".into()),
    };
    let value = payload(Some(branch), ty)?;
    let value = boxed(value, ty, bytes)?;
    Ok(WasmValue::Result(if object.contains_key("ok") {
        Ok(value)
    } else {
        Err(value)
    }))
}

fn flags_from_wire(v: &Value, allowed: &[String]) -> Result<WasmValue, String> {
    let mut flags: Vec<String> = Vec::new();
    for item in array(v, "flags")? {
        let name = item
            .as_str()
            .ok_or_else(|| format!("flag must be a string, got {item}"))?;
        if !allowed.iter().any(|a| a == name) {
            return Err(format!("unknown flag `{name}`"));
        }
        if flags.iter().any(|f| f == name) {
            return Err(format!("flag `{name}` was specified more than once"));
        }
        flags.push(name.to_string());
    }
    Ok(WasmValue::Flags(flags))
}

fn float_wire(value: f64) -> Value {
    if value.is_nan() {
        json!("NaN")
    } else if value.is_infinite() {
        json!(if value > 0.0 { "Infinity" } else { "-Infinity" })
    } else if value == 0.0 && value.is_sign_negative() {
        json!("-0")
    } else {
        json!(value)
    }
}

fn mismatch<T>(ty: &ValueType) -> Result<T, String> {
    Err(format!(
        "component returned a value that does not match `{ty:?}`"
    ))
}

fn payload_wire(
    value: Option<Box<WasmValue>>,
    ty: Option<&ValueType>,
    out: &mut Vec<u8>,
) -> Result<Value, String> {
    match (value, ty) {
        (Some(v), Some(ty)) => to_wire(*v, ty, out),
        (None, None) => Ok(Value::Null),
        _ => Err("payload does not match its declared type".into()),
    }
}

/// Convert a component value to wire JSON, appending byte lists to `out`.
pub(super) fn to_wire(v: WasmValue, ty: &ValueType, out: &mut Vec<u8>) -> Result<Value, String> {
    Ok(match (v, ty) {
        (WasmValue::Bool(v), ValueType::Bool) => json!(v),
        (WasmValue::S8(v), ValueType::S8) => json!(v),
        (WasmValue::U8(v), ValueType::U8) => json!(v),
        (WasmValue::S16(v), ValueType::S16) => json!(v),
        (WasmValue::U16(v), ValueType::U16) => json!(v),
        (WasmValue::S32(v), ValueType::S32) => json!(v),
        (WasmValue::U32(v), ValueType::U32) => json!(v),
        (WasmValue::S64(v), ValueType::S64) => json!(v.to_string()),
        (WasmValue::U64(v), ValueType::U64) => json!(v.to_string()),
        (WasmValue::Float32(v), ValueType::Float32) => float_wire(f64::from(v)),
        (WasmValue::Float64(v), ValueType::Float64) => float_wire(v),
        (WasmValue::Char(v), ValueType::Char) => json!(v.to_string()),
        (WasmValue::String(v), ValueType::String) => json!(v),
        (WasmValue::List(items), ValueType::List(inner)) if **inner == ValueType::U8 => {
            bytes_to_wire(&items, inner, out)?
        }
        (WasmValue::List(items), ValueType::List(inner)) => Value::Array(
            items
                .into_iter()
                .map(|item| to_wire(item, inner, out))
                .collect::<Result<_, _>>()?,
        ),
        (WasmValue::Record(fields), ValueType::Record(types)) if fields.len() == types.len() => {
            record_to_wire(fields, types, out)?
        }
        (WasmValue::Tuple(items), ValueType::Tuple(types)) if items.len() == types.len() => {
            Value::Array(
                items
                    .into_iter()
                    .zip(types)
                    .map(|(item, ty)| to_wire(item, ty, out))
                    .collect::<Result<_, _>>()?,
            )
        }
        (WasmValue::Variant(tag, value), ValueType::Variant(cases)) => {
            let ty = case(cases, &tag, "variant")?;
            let val = payload_wire(value, ty.as_ref(), out)?;
            json!({ "tag": tag, "val": val })
        }
        (WasmValue::Enum(name), ValueType::Enum(cases)) if cases.contains(&name) => json!(name),
        (WasmValue::Option(value), ValueType::Option(inner)) => match value {
            Some(value) => Value::Array(vec![to_wire(*value, inner, out)?]),
            None => Value::Array(Vec::new()),
        },
        (WasmValue::Result(result), ValueType::Result { ok, err }) => match result {
            Ok(value) => json!({ "ok": payload_wire(value, ok.as_deref(), out)? }),
            Err(value) => json!({ "err": payload_wire(value, err.as_deref(), out)? }),
        },
        (WasmValue::Flags(flags), ValueType::Flags(allowed))
            if flags.iter().all(|f| allowed.contains(f)) =>
        {
            json!(flags)
        }
        (_, ty) => return mismatch(ty),
    })
}

/// Append a byte list to `out` and return its `[offset, length]`.
fn bytes_to_wire(
    items: &[WasmValue],
    inner: &ValueType,
    out: &mut Vec<u8>,
) -> Result<Value, String> {
    let offset = out.len();
    for item in items {
        let WasmValue::U8(byte) = item else {
            return mismatch(inner);
        };
        out.push(*byte);
    }
    Ok(json!([offset, items.len()]))
}

fn record_to_wire(
    fields: Vec<(String, WasmValue)>,
    types: &[(String, ValueType)],
    out: &mut Vec<u8>,
) -> Result<Value, String> {
    let mut object = Map::new();
    for ((name, value), (expected, ty)) in fields.into_iter().zip(types) {
        if &name != expected {
            return Err(format!(
                "record field `{name}` where `{expected}` was declared"
            ));
        }
        object.insert(name, to_wire(value, ty, out)?);
    }
    Ok(Value::Object(object))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(ty: &ValueType, v: Value, bytes: &[u8]) -> Result<WasmValue, String> {
        from_wire(&v, ty, bytes)
    }

    fn round_trip(ty: &ValueType, v: Value) -> Value {
        let value = wire(ty, v, &[]).unwrap();
        to_wire(value, ty, &mut Vec::new()).unwrap()
    }

    #[test]
    fn wire_u64_s64_decimal_round_trip() {
        let big = json!("18446744073709551615");
        assert_eq!(round_trip(&ValueType::U64, big.clone()), big);
        let low = json!("-9223372036854775808");
        assert_eq!(round_trip(&ValueType::S64, low.clone()), low);
        assert!(wire(&ValueType::U64, json!(5), &[]).is_err());
        assert!(wire(&ValueType::U64, json!("-1"), &[]).is_err());
    }

    #[test]
    fn wire_bytes_slice_attachment_both_directions() {
        let ty = ValueType::Tuple(vec![
            ValueType::List(Box::new(ValueType::U8)),
            ValueType::List(Box::new(ValueType::U8)),
        ]);
        let value = wire(&ty, json!([[1, 2], [0, 1]]), &[9, 8, 7]).unwrap();
        let expected = |a: &[u8], b: &[u8]| {
            let bytes = |s: &[u8]| WasmValue::List(s.iter().copied().map(WasmValue::U8).collect());
            WasmValue::Tuple(vec![bytes(a), bytes(b)])
        };
        assert_eq!(value, expected(&[8, 7], &[9]));
        let mut out = vec![0xff];
        let reply = to_wire(expected(&[8, 7], &[9]), &ty, &mut out).unwrap();
        assert_eq!(reply, json!([[1, 2], [3, 1]]));
        assert_eq!(out, [0xff, 8, 7, 9]);
    }

    #[test]
    fn wire_nested_option_array_encoding() {
        let ty = ValueType::Option(Box::new(ValueType::Option(Box::new(ValueType::U32))));
        for v in [json!([]), json!([[]]), json!([[7]])] {
            assert_eq!(round_trip(&ty, v.clone()), v);
        }
        assert!(wire(&ty, json!([[], []]), &[]).is_err());
    }

    #[test]
    fn wire_rejects_out_of_range_and_bad_offsets() {
        let bytes_ty = ValueType::List(Box::new(ValueType::U8));
        assert!(wire(&ValueType::U8, json!(256), &[]).is_err());
        assert!(wire(&ValueType::S8, json!(-129), &[]).is_err());
        assert!(wire(&ValueType::U32, json!(1.5), &[]).is_err());
        assert!(wire(&bytes_ty, json!([2, 2]), &[1, 2, 3]).is_err());
        assert!(wire(&bytes_ty, json!([u64::MAX, 2]), &[1, 2, 3]).is_err());
        assert!(wire(&bytes_ty, json!([-1, 1]), &[1]).is_err());
        assert!(wire(&ValueType::Float32, json!(1e300), &[]).is_err());
        assert!(wire(&ValueType::Char, json!("ab"), &[]).is_err());
    }

    #[test]
    fn wire_nonfinite_floats() {
        for (ty, text) in [
            (ValueType::Float64, "Infinity"),
            (ValueType::Float32, "-Infinity"),
        ] {
            assert_eq!(round_trip(&ty, json!(text)), json!(text));
        }
        let nan = wire(&ValueType::Float64, json!("NaN"), &[]).unwrap();
        assert!(matches!(nan, WasmValue::Float64(f) if f.is_nan()));
        assert_eq!(
            to_wire(nan, &ValueType::Float64, &mut Vec::new()).unwrap(),
            json!("NaN")
        );
        assert!(wire(&ValueType::Float64, json!("inf"), &[]).is_err());
    }

    #[test]
    fn wire_negative_zero() {
        for ty in [ValueType::Float32, ValueType::Float64] {
            assert_eq!(round_trip(&ty, json!("-0")), json!("-0"));
            assert_eq!(round_trip(&ty, json!(0.0)), json!(0.0));
        }
    }

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
