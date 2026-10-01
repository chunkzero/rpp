//! [`JsPluginFactory`]: bundling and validation on the main thread.

use std::path::Path;
use std::time::Duration;

use crate::error::Result;
use crate::host::{PackInfo, RuntimeAccess};
use crate::model::{PluginFactory, PluginInstance, ProcessorDef};

/// Resource limits applied to each JavaScript runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsPluginLimits {
    /// V8 heap limit in bytes.
    pub memory_limit: usize,
    /// Maximum wall-clock time for one call.
    pub execution_limit: Duration,
}

impl Default for JsPluginLimits {
    fn default() -> Self {
        Self {
            memory_limit: 256 * 1024 * 1024,
            execution_limit: Duration::from_secs(30),
        }
    }
}

/// A bundled, validated TypeScript plugin. Cheap to clone and shared across workers.
#[derive(Clone)]
pub struct JsPluginFactory {}

impl JsPluginFactory {
    /// Read `plugin.toml` in `dir`, bundle its entry, evaluate it once to read its
    /// processors and handlers, and compute the cache key.
    ///
    /// The cache key covers the bundled code, `plugin.toml`, declared component
    /// bytes, canonical `options`, host access and the rpp version.
    ///
    /// # Errors
    ///
    /// [`crate::Error::PluginLoad`] for bundling or evaluation failures (with
    /// source-mapped stacks), a missing default export, or invalid processor
    /// declarations; I/O and manifest errors otherwise.
    pub fn load(
        dir: impl AsRef<Path>,
        options: toml::Value,
        pack: PackInfo,
        limits: JsPluginLimits,
        access: RuntimeAccess,
    ) -> Result<Self> {
        let _ = (dir.as_ref(), options, pack, limits, access);
        todo!()
    }
}

impl PluginFactory for JsPluginFactory {
    fn id(&self) -> &str {
        todo!()
    }

    fn cache_key(&self) -> u64 {
        todo!()
    }

    fn processors(&self) -> &[ProcessorDef] {
        todo!()
    }

    fn has_generator(&self) -> bool {
        todo!()
    }

    fn instantiate(&self) -> Result<Box<dyn PluginInstance>> {
        todo!()
    }
}
