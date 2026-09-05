//! Live dynamic component instances.

use std::time::Duration;

use wasmtime::component::{ComponentExportIndex, Instance, Val};
use wasmtime::Store;

use crate::store::StoreData;
use crate::{Error, Result, Value};

/// A live component instance.
pub struct WasmInstance {
    pub(crate) store: Store<StoreData>,
    pub(crate) instance: Instance,
    pub(crate) epoch_ticks: u64,
    pub(crate) deadline: Duration,
}

impl WasmInstance {
    /// Call a flattened export path discovered in [`crate::Schema`].
    pub fn call(&mut self, path: &str, params: &[Value]) -> Result<Vec<Value>> {
        let mut parent: Option<ComponentExportIndex> = None;
        let mut segments = path.split('#').peekable();
        let function_name = loop {
            let segment = segments
                .next()
                .ok_or_else(|| Error::MissingExport(path.to_string()))?;
            if segments.peek().is_none() {
                break segment;
            }
            parent = Some(
                self.instance
                    .get_export_index(&mut self.store, parent.as_ref(), segment)
                    .ok_or_else(|| Error::MissingExport(path.to_string()))?,
            );
        };
        let function_index = self
            .instance
            .get_export_index(&mut self.store, parent.as_ref(), function_name)
            .ok_or_else(|| Error::MissingExport(path.to_string()))?;
        let function = self
            .instance
            .get_func(&mut self.store, function_index)
            .ok_or_else(|| Error::MissingExport(path.to_string()))?;

        let params = params.iter().cloned().map(to_wasmtime).collect::<Vec<_>>();
        let result_count = function.ty(&self.store).results().len();
        let mut results = vec![Val::Bool(false); result_count];
        self.store.set_epoch_deadline(self.epoch_ticks);
        function
            .call(&mut self.store, &params, &mut results)
            .map_err(|error| map_timeout(error, self.deadline))?;
        let converted = results
            .into_iter()
            .map(from_wasmtime)
            .collect::<Result<Vec<_>>>();
        function
            .post_return(&mut self.store)
            .map_err(|error| map_timeout(error, self.deadline))?;
        converted
    }
}

fn to_wasmtime(value: Value) -> Val {
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
        Value::Variant(case, value) => {
            Val::Variant(case, value.map(|value| Box::new(to_wasmtime(*value))))
        }
        Value::Enum(case) => Val::Enum(case),
        Value::Option(value) => Val::Option(value.map(|value| Box::new(to_wasmtime(*value)))),
        Value::Result(result) => Val::Result(match result {
            Ok(value) => Ok(value.map(|value| Box::new(to_wasmtime(*value)))),
            Err(value) => Err(value.map(|value| Box::new(to_wasmtime(*value)))),
        }),
        Value::Flags(flags) => Val::Flags(flags),
    }
}

fn from_wasmtime(value: Val) -> Result<Value> {
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
        Val::List(values) => Value::List(
            values
                .into_iter()
                .map(from_wasmtime)
                .collect::<Result<_>>()?,
        ),
        Val::Record(fields) => Value::Record(
            fields
                .into_iter()
                .map(|(name, value)| Ok((name, from_wasmtime(value)?)))
                .collect::<Result<_>>()?,
        ),
        Val::Tuple(values) => Value::Tuple(
            values
                .into_iter()
                .map(from_wasmtime)
                .collect::<Result<_>>()?,
        ),
        Val::Variant(case, value) => Value::Variant(
            case,
            value
                .map(|value| from_wasmtime(*value).map(Box::new))
                .transpose()?,
        ),
        Val::Enum(case) => Value::Enum(case),
        Val::Option(value) => Value::Option(
            value
                .map(|value| from_wasmtime(*value).map(Box::new))
                .transpose()?,
        ),
        Val::Result(result) => Value::Result(match result {
            Ok(value) => Ok(value
                .map(|value| from_wasmtime(*value).map(Box::new))
                .transpose()?),
            Err(value) => Err(value
                .map(|value| from_wasmtime(*value).map(Box::new))
                .transpose()?),
        }),
        Val::Flags(flags) => Value::Flags(flags),
        Val::Resource(_) | Val::Future(_) | Val::Stream(_) | Val::ErrorContext(_) => {
            return Err(Error::Value(
                "resource, future, stream, and error-context values are unsupported".into(),
            ))
        }
    })
}

pub(crate) fn map_timeout(error: wasmtime::Error, deadline: Duration) -> Error {
    if error
        .downcast_ref::<wasmtime::Trap>()
        .is_some_and(|trap| *trap == wasmtime::Trap::Interrupt)
    {
        Error::Timeout(deadline)
    } else {
        Error::Trap(error)
    }
}
