//! Runtime capabilities granted to a plugin instance.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use crate::config::{PluginPermissions, SecurityMode};

#[cfg(feature = "wasm")]
use rpp_wasm::{CompiledComponent, Permissions};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    Load = 0,
    Processor = 1,
    Generator = 2,
    Hook = 3,
}

#[derive(Clone)]
pub(crate) struct PhaseCell(Arc<AtomicU8>);

impl PhaseCell {
    pub(crate) fn new() -> Self {
        Self(Arc::new(AtomicU8::new(Phase::Load as u8)))
    }

    pub(crate) fn set(&self, phase: Phase) {
        self.0.store(phase as u8, Ordering::SeqCst);
    }

    pub(crate) fn get(&self) -> Phase {
        match self.0.load(Ordering::SeqCst) {
            1 => Phase::Processor,
            2 => Phase::Generator,
            3 => Phase::Hook,
            _ => Phase::Load,
        }
    }
}

/// Host capabilities granted to one plugin.
#[derive(Clone)]
pub struct RuntimeAccess {
    pub security: SecurityMode,
    pub permissions: PluginPermissions,
    pub project_root: PathBuf,
    pub outputs: BTreeMap<String, PathBuf>,
    pub(crate) phase: PhaseCell,
    #[cfg(feature = "wasm")]
    pub components: BTreeMap<String, CompiledComponent>,
}

impl RuntimeAccess {
    /// Build an access policy for a plugin loaded by the CLI.
    #[cfg(feature = "wasm")]
    pub fn new(
        security: SecurityMode,
        permissions: PluginPermissions,
        project_root: PathBuf,
        components: BTreeMap<String, CompiledComponent>,
        outputs: BTreeMap<String, PathBuf>,
    ) -> Self {
        Self {
            security,
            permissions,
            project_root,
            outputs,
            phase: PhaseCell::new(),
            components,
        }
    }

    pub fn sandboxed(project_root: PathBuf) -> Self {
        Self {
            security: SecurityMode::Sandboxed,
            permissions: PluginPermissions::default(),
            project_root,
            outputs: BTreeMap::new(),
            phase: PhaseCell::new(),
            #[cfg(feature = "wasm")]
            components: BTreeMap::new(),
        }
    }

    /// Attach named, explicitly configured generated-output roots.
    pub fn with_outputs(mut self, outputs: BTreeMap<String, PathBuf>) -> Self {
        self.outputs = outputs;
        self
    }

    pub(crate) fn is_native(&self) -> bool {
        self.security == SecurityMode::Native
    }

    pub(crate) fn allows_process(&self) -> bool {
        self.is_native() || !self.permissions.process.is_empty()
    }

    /// Whether calls made through this policy are deterministic from RPP's
    /// tracked inputs. Declared output roots are tracked destinations, not an
    /// ambient capability, so they do not disable cache replay.
    pub(crate) fn is_deterministic(&self) -> bool {
        !self.is_native()
            && self.permissions.process.is_empty()
            && self.permissions.environment.is_empty()
            && self.permissions.read.is_empty()
            && self.permissions.write.is_empty()
            && !self.permissions.network
            && !self.permissions.clocks
            && !self.permissions.random
            && !self.permissions.stdio
            && self.permissions.lua.is_empty()
    }

    #[cfg(feature = "wasm")]
    pub(crate) fn wasm_permissions(&self) -> Permissions {
        let env = self
            .permissions
            .environment
            .iter()
            .filter_map(|name| std::env::var(name).ok().map(|value| (name.clone(), value)))
            .collect();
        let mut preopens = Vec::new();
        for path in &self.permissions.read {
            preopens.push(rpp_wasm::Preopen {
                host: self.project_root.join(path),
                guest: path.to_string_lossy().replace('\\', "/"),
                writable: false,
            });
        }
        for path in &self.permissions.write {
            preopens.push(rpp_wasm::Preopen {
                host: self.project_root.join(path),
                guest: path.to_string_lossy().replace('\\', "/"),
                writable: true,
            });
        }
        Permissions {
            clocks: self.is_native() || self.permissions.clocks,
            random: self.is_native() || self.permissions.random,
            stdio: self.is_native() || self.permissions.stdio,
            network: self.is_native() || self.permissions.network,
            environment: env,
            preopens,
        }
    }
}
