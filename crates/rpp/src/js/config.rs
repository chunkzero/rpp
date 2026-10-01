//! Evaluating `rpp.config.ts`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::error::Result;
use crate::js::JsPluginLimits;

/// The project config file name.
pub const CONFIG_FILE: &str = "rpp.config.ts";

/// A resolved dependency whose config module `rpp.config.ts` may import as
/// `#plugins/<name>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigPackage {
    /// The package root.
    pub dir: PathBuf,
    /// Its config module, relative to `dir` ([`crate::manifest::PluginManifest::config`]).
    pub config: Option<String>,
}

/// An evaluated project config.
#[derive(Debug, Clone)]
pub struct EvaluatedConfig {
    /// The project configuration.
    pub config: Config,
    /// Every real file the config bundle read, for watching.
    pub inputs: Vec<PathBuf>,
}

/// Bundle `<project_root>/rpp.config.ts` with `#rpp/config` (the embedded
/// `sdk/config.ts`) and `#plugins/<name>` for each package that has a config module,
/// evaluate it with a fixed clock and no host functions, and convert its default
/// export with [`Config::from_ts_json`].
///
/// # Errors
///
/// [`crate::Error::Config`] (attributed to `rpp.config.ts`) for bundling failures,
/// exceptions (with source-mapped stacks, e.g. from a factory's `validate`), a missing
/// or non-JSON default export, or an invalid config.
pub fn evaluate_config(
    project_root: &Path,
    packages: &BTreeMap<String, ConfigPackage>,
    limits: JsPluginLimits,
) -> Result<EvaluatedConfig> {
    let _ = (project_root, packages, limits);
    todo!()
}
