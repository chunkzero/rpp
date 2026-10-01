//! Per-instance wasmtime store state.

use std::time::Duration;

use wasmtime::component::ResourceTable;
use wasmtime::{StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{
    FsPerms, HostMonotonicClock, HostWallClock, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView,
};

use crate::types::Permissions;

/// Wall clock fixed at the Unix epoch.
struct FixedWallClock;

impl HostWallClock for FixedWallClock {
    fn resolution(&self) -> Duration {
        Duration::from_nanos(1)
    }

    fn now(&self) -> Duration {
        Duration::ZERO
    }
}

/// Monotonic clock fixed at zero.
struct FixedMonotonicClock;

impl HostMonotonicClock for FixedMonotonicClock {
    fn resolution(&self) -> u64 {
        1
    }

    fn now(&self) -> u64 {
        0
    }
}

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
        if !permissions.clocks {
            // Rust WASIp2 components import the clock interfaces even when
            // they never read time; fixed clocks keep them instantiable and
            // reproducible.
            builder
                .wall_clock(FixedWallClock)
                .monotonic_clock(FixedMonotonicClock);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_clocks_report_constants() {
        assert_eq!(FixedWallClock.now(), Duration::ZERO);
        assert_eq!(FixedWallClock.resolution(), Duration::from_nanos(1));
        assert_eq!(FixedMonotonicClock.now(), 0);
        assert_eq!(FixedMonotonicClock.resolution(), 1);
    }
}
