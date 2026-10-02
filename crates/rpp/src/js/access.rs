//! Runtime capabilities granted to a plugin instance.

use std::collections::BTreeMap;
use std::path::PathBuf;

#[cfg(feature = "wasm")]
use rpp_wasm::CompiledComponent;

use super::factory::JsPluginSpec;
use crate::config::{PluginPermissions, SecurityMode};

/// Host capabilities granted to one plugin.
#[derive(Clone)]
pub(crate) struct RuntimeAccess {
    pub(crate) security: SecurityMode,
    pub(crate) permissions: PluginPermissions,
    pub(crate) project_root: PathBuf,
    pub(crate) outputs: BTreeMap<String, PathBuf>,
    #[cfg(feature = "wasm")]
    pub(crate) components: BTreeMap<String, CompiledComponent>,
}

impl RuntimeAccess {
    /// The access the spec's plugin entry grants.
    pub(crate) fn new(spec: JsPluginSpec<'_>) -> Self {
        Self {
            security: spec.plugin.security,
            permissions: spec.plugin.permissions.clone(),
            project_root: spec.project_root.to_path_buf(),
            outputs: spec.plugin.outputs.clone(),
            #[cfg(feature = "wasm")]
            components: spec.components,
        }
    }

    /// No capabilities beyond the tracked RPP APIs.
    #[cfg(test)]
    pub(crate) fn sandboxed(project_root: PathBuf) -> Self {
        Self {
            security: SecurityMode::Sandboxed,
            permissions: PluginPermissions::default(),
            project_root,
            outputs: BTreeMap::new(),
            #[cfg(feature = "wasm")]
            components: BTreeMap::new(),
        }
    }

    pub(crate) fn allows_process(&self) -> bool {
        !self.permissions.process.is_empty()
    }

    /// Whether calls made through this policy are deterministic from RPP's
    /// tracked inputs. Declared output roots are tracked destinations, not an
    /// ambient capability, so they do not disable cache replay.
    pub(crate) fn is_deterministic(&self) -> bool {
        self.permissions.is_empty()
    }
}
