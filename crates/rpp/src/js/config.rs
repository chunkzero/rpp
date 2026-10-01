//! Evaluating `rpp.config.ts`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rpp_js::{
    BundlePackage, BundleRequest, Call, Cancellation, Clock, Host, HostReply, Limits, LogLevel,
};
use serde_json::Value;

use crate::config::Config;
use crate::error::{Error, Result};
use crate::host::log::{self, LogLevel as HostLevel};
use crate::js::{instance, JsPluginLimits};

const CONFIG_SDK: &str = include_str!("sdk/config.ts");
const ENTRY_MODULE: &str =
    "import config from \"./rpp.config.ts\";\nexport function main() { return config; }\n";
const LOG_LABEL: &str = "rpp.config.ts";

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
    let path = project_root.join(CONFIG_FILE);
    let fail = |message: String| Error::Config {
        path: path.clone(),
        message: message.trim_end().to_string(),
    };

    let bundle = rpp_js::bundle(&BundleRequest {
        root: project_root.to_path_buf(),
        entry: "rpp:config-entry".into(),
        virtual_modules: BTreeMap::from([
            ("rpp:config-entry".to_string(), ENTRY_MODULE.to_string()),
            ("#rpp/config".to_string(), CONFIG_SDK.to_string()),
        ]),
        packages: packages
            .iter()
            .filter_map(|(name, package)| {
                let entry = package.config.clone()?;
                Some((
                    format!("#plugins/{name}"),
                    BundlePackage {
                        dir: package.dir.clone(),
                        entry,
                    },
                ))
            })
            .collect(),
    })
    .map_err(|e| fail(e.to_string()))?;

    let limits = Limits {
        heap_bytes: limits.memory_limit,
        time: limits.execution_limit,
    };
    let clock = Clock::Fixed {
        timestamp_ms: 0,
        seed: 0,
    };
    let cancellation = Cancellation::new();
    let engine = instance::engine().map_err(&fail)?;
    let (mut runtime, logs) = engine
        .load(LOG_LABEL, &bundle, limits, clock, &cancellation)
        .map_err(|e| fail(e.to_string()))?;
    emit_logs(logs);
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
    emit_logs(output.logs);
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

fn emit_logs(logs: Vec<rpp_js::Log>) {
    for entry in logs {
        let level = match entry.level {
            LogLevel::Debug => HostLevel::Debug,
            LogLevel::Info => HostLevel::Info,
            LogLevel::Warn => HostLevel::Warn,
            LogLevel::Error => HostLevel::Error,
        };
        log::emit(LOG_LABEL, level, &entry.message);
    }
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
