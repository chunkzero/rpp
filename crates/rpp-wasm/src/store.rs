//! Per-instance wasmtime store state.

use wasmtime::component::ResourceTable;
use wasmtime::{StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{FsPerms, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::types::Permissions;

pub(crate) struct StoreData {
    pub(crate) wasi: WasiCtx,
    pub(crate) table: ResourceTable,
    pub(crate) limits: StoreLimits,
}

impl StoreData {
    pub(crate) fn new(permissions: Permissions, memory_bytes: usize) -> crate::Result<Self> {
        let mut builder = WasiCtxBuilder::new();
        if !permissions.random {
            // Rust WASIp2 components commonly import the random interfaces for
            // hash-map seeding even when the plugin has no random capability.
            // Keep those components instantiable while making permissionless
            // builds reproducible.
            builder
                .secure_random(wasmtime_wasi::Deterministic::new(vec![0x52, 0x50, 0x50]))
                .insecure_random(wasmtime_wasi::Deterministic::new(vec![0x50, 0x50, 0x52]))
                .insecure_random_seed(0);
        }
        if permissions.stdio {
            builder.inherit_stdout().inherit_stderr();
        }
        for (name, value) in &permissions.environment {
            builder.env(name, value);
        }
        if permissions.network {
            builder.inherit_network().allow_tcp(true).allow_udp(true);
        }
        for preopen in &permissions.preopens {
            let perms = if preopen.writable {
                FsPerms::ReadWrite
            } else {
                FsPerms::ReadOnly
            };
            builder
                .preopened_dir(&preopen.host, &preopen.guest, perms)
                .map_err(|source| crate::Error::WasiDirectory {
                    path: preopen.host.clone(),
                    source,
                })?;
        }

        Ok(Self {
            wasi: builder.build(),
            table: ResourceTable::new(),
            limits: StoreLimitsBuilder::new()
                .memory_size(memory_bytes)
                .memories(8)
                .tables(64)
                .table_elements(1_000_000)
                .instances(128)
                .build(),
        })
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
