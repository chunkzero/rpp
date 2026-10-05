//! Evaluating `rpp.config.ts`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rpp_js::{Bundle, BundlePackage, BundleRequest, Call, Cancellation, Clock, Host, HostReply};
use serde_json::Value;

use super::{instance, jsx_modules, log, runtime_limits, JSX_IMPORT_SOURCE, SDK_CONFIG};
use crate::config::{Config, LimitsConfig};
use crate::error::{Error, Result};

const ENTRY_MODULE: &str =
    "import config from \"./rpp.config.ts\";\nexport function main() { return config; }\n";
const LOG_LABEL: &str = "rpp.config.ts";
const EVALUATION_DEADLINE_SECONDS: u64 = 30;

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
/// evaluate it with a fixed clock, the default `build.limits` (with a 30-second deadline) and no
/// host functions, and
/// convert its default export with [`Config::from_ts_json`].
///
/// # Errors
///
/// [`crate::Error::Config`] (attributed to `rpp.config.ts`) for bundling failures,
/// exceptions (with source-mapped stacks, e.g. from a factory's `validate`), a missing
/// or non-JSON default export, or an invalid config.
pub fn evaluate_config(
    project_root: &Path,
    packages: &BTreeMap<String, ConfigPackage>,
) -> Result<EvaluatedConfig> {
    let path = project_root.join(CONFIG_FILE);
    let fail = |message: String| Error::Config {
        path: path.clone(),
        message: message.trim_end().to_string(),
    };

    let bundle = bundle_config(project_root, packages).map_err(|e| fail(e.to_string()))?;

    let limits = runtime_limits(&config_limits());
    let clock = Clock::Fixed {
        timestamp_ms: 0,
        seed: 0,
    };
    let cancellation = Cancellation::new();
    let engine = instance::engine().map_err(&fail)?;
    let (mut runtime, logs) = engine
        .load(LOG_LABEL, &bundle, limits, clock, &cancellation)
        .map_err(|e| fail(e.to_string()))?;
    log::emit(LOG_LABEL, &logs);
    let output = runtime
        .call(
            &engine,
            Call {
                export: "main",
                args: Value::Null,
                bytes: None,
                clock,
            },
            &mut NoHost,
            &cancellation,
        )
        .map_err(|e| fail(e.to_string()))?;
    log::emit(LOG_LABEL, &output.logs);
    if output.value.is_null() {
        return Err(fail(
            "rpp.config.ts must default-export a config object".into(),
        ));
    }

    Ok(EvaluatedConfig {
        config: Config::from_ts_json(&output.value, &path)?,
        inputs: bundle.inputs,
    })
}

/// The default `build.limits` with the shorter config evaluation deadline.
fn config_limits() -> LimitsConfig {
    LimitsConfig {
        execution_deadline_seconds: EVALUATION_DEADLINE_SECONDS,
        ..LimitsConfig::default()
    }
}

fn bundle_config(
    project_root: &Path,
    packages: &BTreeMap<String, ConfigPackage>,
) -> rpp_js::Result<Bundle> {
    rpp_js::bundle(&BundleRequest {
        root: project_root.to_path_buf(),
        entry: "rpp:config-entry".into(),
        virtual_modules: [
            ("rpp:config-entry".to_string(), ENTRY_MODULE.to_string()),
            ("#rpp/config".to_string(), SDK_CONFIG.to_string()),
        ]
        .into_iter()
        .chain(jsx_modules())
        .collect(),
        packages: packages
            .iter()
            .filter_map(|(name, package)| {
                let entry = package.config.clone()?;
                let dir = package.dir.clone();
                Some((format!("#plugins/{name}"), BundlePackage { dir, entry }))
            })
            .collect(),
        jsx_import_source: Some(JSX_IMPORT_SOURCE.to_string()),
    })
}

struct NoHost;

impl Host for NoHost {
    fn call(
        &mut self,
        _name: &str,
        _value: Value,
        _bytes: Option<Vec<u8>>,
    ) -> std::result::Result<HostReply, String> {
        Err("host functions are unavailable in rpp.config.ts".into())
    }
}
