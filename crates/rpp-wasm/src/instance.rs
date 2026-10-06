//! Live dynamic component instances.

use std::time::Duration;

use wasmtime::component::{ComponentExportIndex, Instance, Val};
use wasmtime::Store;

use crate::engine::ticks_for;
use crate::store::StoreData;
use crate::value::{from_wasmtime, to_wasmtime};
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
        self.call_with_ticks(path, params, self.epoch_ticks, self.deadline)
    }

    /// Like [`Self::call`], with the deadline capped at `limit`. A timeout reports
    /// the effective deadline.
    pub fn call_with_deadline(
        &mut self,
        path: &str,
        params: &[Value],
        limit: Duration,
    ) -> Result<Vec<Value>> {
        let deadline = limit.min(self.deadline);
        self.call_with_ticks(path, params, ticks_for(deadline), deadline)
    }

    fn call_with_ticks(
        &mut self,
        path: &str,
        params: &[Value],
        ticks: u64,
        deadline: Duration,
    ) -> Result<Vec<Value>> {
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

        let params = params.iter().map(to_wasmtime).collect::<Vec<_>>();
        let result_count = function.ty(&self.store).results().len();
        let mut results = vec![Val::Bool(false); result_count];
        self.store.set_epoch_deadline(ticks);
        function
            .call(&mut self.store, &params, &mut results)
            .map_err(|error| map_timeout(error, deadline))?;
        results.into_iter().map(from_wasmtime).collect()
    }
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
