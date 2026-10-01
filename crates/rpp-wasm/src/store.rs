//! Per-instance wasmtime store state.

use std::time::Duration;

use wasmtime::component::{Linker, Resource, ResourceTable};
use wasmtime::{StoreContextMut, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::p2::{subscribe, DynPollable, Pollable};
use wasmtime_wasi::{
    async_trait, FsPerms, HostMonotonicClock, HostWallClock, WasiCtx, WasiCtxBuilder, WasiCtxView,
    WasiView,
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

/// Pollable that is ready as soon as it is polled, so fixed-clock guests never wait.
struct ReadyPollable;

#[async_trait]
impl Pollable for ReadyPollable {
    async fn ready(&mut self) {}
}

fn ready_pollable(
    mut store: StoreContextMut<'_, StoreData>,
) -> wasmtime::Result<(Resource<DynPollable>,)> {
    let table = &mut store.data_mut().table;
    let pollable = table.push(ReadyPollable)?;
    Ok((subscribe(table, pollable)?,))
}

/// Replaces the monotonic-clock subscriptions with always-ready pollables.
/// Time on the fixed clock passes instantly, so subscriptions never block on
/// the host's real timers.
pub(crate) fn link_instant_subscriptions(linker: &mut Linker<StoreData>) -> wasmtime::Result<()> {
    linker.allow_shadowing(true);
    let mut clock = linker.instance("wasi:clocks/monotonic-clock@0.2.12")?;
    clock.func_wrap("subscribe-duration", |store, (_nanos,): (u64,)| {
        ready_pollable(store)
    })?;
    clock.func_wrap("subscribe-instant", |store, (_nanos,): (u64,)| {
        ready_pollable(store)
    })?;
    Ok(())
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
