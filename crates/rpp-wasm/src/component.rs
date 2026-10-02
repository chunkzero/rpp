//! Compiled component handles.

use std::time::Duration;

use wasmtime::component::types::ComponentItem;
use wasmtime::component::{Component, Linker};
use wasmtime::Engine;
use wasmtime_wasi::p2;

use crate::engine::ticks_for;
use crate::instance::map_timeout;
use crate::store::link_instant_subscriptions;
use crate::types::{Function, Permissions, Schema};
use crate::value::value_type;
use crate::{Error, Result, WasmEngine, WasmInstance};

/// A compiled, reusable component library.
#[derive(Clone)]
pub struct CompiledComponent {
    engine: WasmEngine,
    component: Component,
    schema: Schema,
}

impl CompiledComponent {
    pub(crate) fn new(engine: WasmEngine, component: Component) -> Self {
        let schema = schema(&engine.engine, &component);
        Self {
            engine,
            component,
            schema,
        }
    }

    /// Discovered imports and exported function signatures.
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// Instantiate with an explicit capability set.
    pub fn instantiate(&self, permissions: Permissions) -> Result<WasmInstance> {
        self.instantiate_with_ticks(
            permissions,
            self.engine.limits.epoch_ticks(),
            self.engine.limits.deadline,
        )
    }

    /// Like [`Self::instantiate`], with the deadline for start functions capped at `limit`.
    /// A timeout reports the effective deadline.
    pub fn instantiate_with_deadline(
        &self,
        permissions: Permissions,
        limit: Duration,
    ) -> Result<WasmInstance> {
        let deadline = limit.min(self.engine.limits.deadline);
        self.instantiate_with_ticks(permissions, ticks_for(deadline), deadline)
    }

    fn instantiate_with_ticks(
        &self,
        permissions: Permissions,
        ticks: u64,
        deadline: Duration,
    ) -> Result<WasmInstance> {
        validate_imports(&self.schema.imports, &permissions)?;
        let mut linker = Linker::new(&self.engine.engine);
        p2::add_to_linker_sync(&mut linker).map_err(Error::Engine)?;
        if !permissions.clocks {
            link_instant_subscriptions(&mut linker).map_err(Error::Engine)?;
        }

        let mut store = self.engine.new_store(permissions)?;
        store.set_epoch_deadline(ticks);
        let instance = linker
            .instantiate(&mut store, &self.component)
            .map_err(|error| map_timeout(error, deadline))?;
        Ok(WasmInstance {
            store,
            instance,
            epoch_ticks: self.engine.limits.epoch_ticks(),
            deadline: self.engine.limits.deadline,
        })
    }
}

fn schema(engine: &Engine, component: &Component) -> Schema {
    let ty = component.component_type();
    let imports = ty
        .imports(engine)
        .map(|(name, _)| name.to_string())
        .collect();
    let mut functions = Vec::new();
    for (name, item) in ty.exports(engine) {
        collect_functions(engine, name, item.ty, &mut functions);
    }
    functions.sort_by(|a, b| a.path.cmp(&b.path));
    Schema { imports, functions }
}

fn collect_functions(engine: &Engine, path: &str, item: ComponentItem, output: &mut Vec<Function>) {
    match item {
        ComponentItem::ComponentFunc(function) => output.push(Function {
            path: path.to_string(),
            params: function
                .params()
                .map(|(name, ty)| (name.to_string(), value_type(ty)))
                .collect(),
            results: function.results().map(value_type).collect(),
        }),
        ComponentItem::ComponentInstance(instance) => {
            for (name, item) in instance.exports(engine) {
                collect_functions(engine, &format!("{path}#{name}"), item.ty, output);
            }
        }
        _ => {}
    }
}

fn validate_imports(imports: &[String], permissions: &Permissions) -> Result<()> {
    for import in imports {
        let allowed = if import.starts_with("wasi:sockets/") {
            permissions.network
        } else if import.starts_with("wasi:filesystem/") {
            !permissions.preopens.is_empty()
        } else {
            // Clock and random imports receive deterministic values unless
            // granted; cli/io are always linked.
            import.starts_with("wasi:clocks/")
                || import.starts_with("wasi:random/")
                || import.starts_with("wasi:cli/environment")
                || import.starts_with("wasi:cli/exit")
                || import.starts_with("wasi:cli/std")
                || import.starts_with("wasi:cli/terminal")
                || import.starts_with("wasi:io/")
        };
        if !allowed {
            return Err(Error::DeniedCapability(import.clone()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_imports_are_allowed_without_permission() {
        let imports = vec!["wasi:clocks/monotonic-clock@0.2.6".to_string()];
        assert!(validate_imports(&imports, &Permissions::default()).is_ok());
    }
}
