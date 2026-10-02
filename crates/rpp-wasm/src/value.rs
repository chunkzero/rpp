//! Conversions between wasmtime component types and values and the public ones.

use wasmtime::component::types::Type;
use wasmtime::component::Val;

use crate::types::{Value, ValueType};
use crate::{Error, Result};

pub(crate) fn value_type(ty: Type) -> ValueType {
    match ty {
        Type::Bool => ValueType::Bool,
        Type::S8 => ValueType::S8,
        Type::U8 => ValueType::U8,
        Type::S16 => ValueType::S16,
        Type::U16 => ValueType::U16,
        Type::S32 => ValueType::S32,
        Type::U32 => ValueType::U32,
        Type::S64 => ValueType::S64,
        Type::U64 => ValueType::U64,
        Type::Float32 => ValueType::Float32,
        Type::Float64 => ValueType::Float64,
        Type::Char => ValueType::Char,
        Type::String => ValueType::String,
        Type::List(list) => ValueType::List(Box::new(value_type(list.ty()))),
        Type::Record(record) => ValueType::Record(
            record
                .fields()
                .map(|field| (field.name.to_string(), value_type(field.ty)))
                .collect(),
        ),
        Type::Tuple(tuple) => ValueType::Tuple(tuple.types().map(value_type).collect()),
        Type::Variant(variant) => ValueType::Variant(
            variant
                .cases()
                .map(|case| (case.name.to_string(), case.ty.map(value_type)))
                .collect(),
        ),
        Type::Enum(enum_) => ValueType::Enum(enum_.names().map(str::to_string).collect()),
        Type::Option(option) => ValueType::Option(Box::new(value_type(option.ty()))),
        Type::Result(result) => ValueType::Result {
            ok: result.ok().map(value_type).map(Box::new),
            err: result.err().map(value_type).map(Box::new),
        },
        Type::Flags(flags) => ValueType::Flags(flags.names().map(str::to_string).collect()),
        Type::Own(_) | Type::Borrow(_) => ValueType::Unsupported("resource".into()),
        Type::Future(_) => ValueType::Unsupported("future".into()),
        Type::Stream(_) => ValueType::Unsupported("stream".into()),
        Type::ErrorContext => ValueType::Unsupported("error-context".into()),
        Type::Map(_) => ValueType::Unsupported("map".into()),
        Type::FixedLengthList(_) => ValueType::Unsupported("fixed-length list".into()),
    }
}

pub(crate) fn to_wasmtime(value: Value) -> Val {
    match value {
        Value::Bool(value) => Val::Bool(value),
        Value::S8(value) => Val::S8(value),
        Value::U8(value) => Val::U8(value),
        Value::S16(value) => Val::S16(value),
        Value::U16(value) => Val::U16(value),
        Value::S32(value) => Val::S32(value),
        Value::U32(value) => Val::U32(value),
        Value::S64(value) => Val::S64(value),
        Value::U64(value) => Val::U64(value),
        Value::Float32(value) => Val::Float32(value),
        Value::Float64(value) => Val::Float64(value),
        Value::Char(value) => Val::Char(value),
        Value::String(value) => Val::String(value),
        Value::List(values) => Val::List(values.into_iter().map(to_wasmtime).collect()),
        Value::Record(fields) => Val::Record(
            fields
                .into_iter()
                .map(|(name, value)| (name, to_wasmtime(value)))
                .collect(),
        ),
        Value::Tuple(values) => Val::Tuple(values.into_iter().map(to_wasmtime).collect()),
        Value::Variant(case, value) => Val::Variant(case, boxed_to_wasmtime(value)),
        Value::Enum(case) => Val::Enum(case),
        Value::Option(value) => Val::Option(boxed_to_wasmtime(value)),
        Value::Result(result) => Val::Result(match result {
            Ok(value) => Ok(boxed_to_wasmtime(value)),
            Err(value) => Err(boxed_to_wasmtime(value)),
        }),
        Value::Flags(flags) => Val::Flags(flags),
    }
}

fn boxed_to_wasmtime(value: Option<Box<Value>>) -> Option<Box<Val>> {
    value.map(|value| Box::new(to_wasmtime(*value)))
}

pub(crate) fn from_wasmtime(value: Val) -> Result<Value> {
    Ok(match value {
        Val::Bool(value) => Value::Bool(value),
        Val::S8(value) => Value::S8(value),
        Val::U8(value) => Value::U8(value),
        Val::S16(value) => Value::S16(value),
        Val::U16(value) => Value::U16(value),
        Val::S32(value) => Value::S32(value),
        Val::U32(value) => Value::U32(value),
        Val::S64(value) => Value::S64(value),
        Val::U64(value) => Value::U64(value),
        Val::Float32(value) => Value::Float32(value),
        Val::Float64(value) => Value::Float64(value),
        Val::Char(value) => Value::Char(value),
        Val::String(value) => Value::String(value),
        Val::List(values) => Value::List(many_from_wasmtime(values)?),
        Val::Record(fields) => Value::Record(
            fields
                .into_iter()
                .map(|(name, value)| Ok((name, from_wasmtime(value)?)))
                .collect::<Result<_>>()?,
        ),
        Val::Tuple(values) => Value::Tuple(many_from_wasmtime(values)?),
        Val::Variant(case, value) => Value::Variant(case, boxed_from_wasmtime(value)?),
        Val::Enum(case) => Value::Enum(case),
        Val::Option(value) => Value::Option(boxed_from_wasmtime(value)?),
        Val::Result(result) => Value::Result(match result {
            Ok(value) => Ok(boxed_from_wasmtime(value)?),
            Err(value) => Err(boxed_from_wasmtime(value)?),
        }),
        Val::Flags(flags) => Value::Flags(flags),
        Val::Resource(_)
        | Val::Future(_)
        | Val::Stream(_)
        | Val::ErrorContext(_)
        | Val::Map(_)
        | Val::FixedLengthList(_) => {
            return Err(Error::Value(
                "resource, future, stream, error-context, map, and fixed-length list values \
                 are unsupported"
                    .into(),
            ))
        }
    })
}

fn many_from_wasmtime(values: Vec<Val>) -> Result<Vec<Value>> {
    values.into_iter().map(from_wasmtime).collect()
}

fn boxed_from_wasmtime(value: Option<Box<Val>>) -> Result<Option<Box<Value>>> {
    value
        .map(|value| from_wasmtime(*value).map(Box::new))
        .transpose()
}
