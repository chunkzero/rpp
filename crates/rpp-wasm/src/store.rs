//! Per-instance wasmtime store state.

use wasmtime::component::ResourceTable;
use wasmtime::{StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::types::HostCallbacks;

/// Per-store data: WASI context, resource table, memory limits, the host
/// callbacks, and a flag tracking whether the generate phase is active.
pub(crate) struct StoreData {
    pub(crate) wasi: WasiCtx,
    pub(crate) table: ResourceTable,
    pub(crate) limits: StoreLimits,
    pub(crate) host: Box<dyn HostCallbacks>,
    pub(crate) in_generate: bool,
}

impl StoreData {
    pub(crate) fn new(host: Box<dyn HostCallbacks>, memory_bytes: usize) -> Self {
        let wasi = WasiCtxBuilder::new()
            .inherit_stdout()
            .inherit_stderr()
            .build();
        let limits = StoreLimitsBuilder::new()
            .memory_size(memory_bytes)
            .memories(8)
            .tables(64)
            .table_elements(1_000_000)
            .instances(128)
            .build();
        Self {
            wasi,
            table: ResourceTable::new(),
            limits,
            host,
            in_generate: false,
        }
    }
}

impl WasiView for StoreData {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}
