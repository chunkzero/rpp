//! Shared project plumbing: locate `rpp.config.ts`, evaluate it into a [`Config`], resolve the
//! `rpp.json` dependencies (lockfile-aware), construct the plugin factory for each configured
//! plugin, and build an [`Engine`].
//!
//! This is the single place the CLI commands go through to turn a directory on
//! disk into a ready-to-run build engine, so resolution, plugin loading, and
//! error messaging are consistent across `build`, `dev`, and friends.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use rpp::config::{Config, PluginConfig};
use rpp::engine::Engine;
use rpp::js::{evaluate_config, ConfigPackage, JsPluginFactory, JsPluginSpec};
use rpp::manifest::PluginManifest;
use rpp::model::PluginFactory;
use rpp_fetch::registry::{
    parse_dependencies, resolve, PackageLock, Registry, Update, PACKAGE_MANIFEST,
};
use rpp_wasm::WasmEngine;
use semver::Version;

/// The config file name.
pub const CONFIG_FILE: &str = rpp::js::CONFIG_FILE;
/// The retired TOML config file name.
pub const LEGACY_CONFIG_FILE: &str = "rpp.toml";
/// The lockfile name.
pub const LOCK_FILE: &str = "rpp.lock";

/// A located project: the directory containing `rpp.config.ts` plus the evaluated config.
pub struct Project {
    /// The directory containing the project config file.
    pub root: PathBuf,
    /// The evaluated configuration.
    pub config: Config,
    pub(crate) ts: TsProject,
}

/// A resolved `rpp.json` dependency.
#[derive(Debug, Clone)]
pub(crate) struct TsPackage {
    pub(crate) dir: PathBuf,
    pub(crate) manifest: PluginManifest,
}

/// What loading `rpp.config.ts` resolved.
pub(crate) struct TsProject {
    pub(crate) packages: BTreeMap<String, TsPackage>,
    /// Files the config bundle read.
    inputs: Vec<PathBuf>,
}

impl Project {
    /// Locate and load the project containing `start` (searching ancestors).
    pub fn discover(start: &Path) -> Result<Self> {
        let root = nearest(start, |dir| {
            dir.join(CONFIG_FILE).is_file() || dir.join(LEGACY_CONFIG_FILE).is_file()
        })?
        .with_context(|| {
            format!(
                "no `{CONFIG_FILE}` found in `{}` or any parent directory\n\
                 (run `rpp init` to scaffold a new project)",
                start.display()
            )
        })?;
        if !root.join(CONFIG_FILE).is_file() {
            return Err(legacy_config_error());
        }
        if root.join(LEGACY_CONFIG_FILE).is_file() {
            bail!(
                "`{}` contains both `{LEGACY_CONFIG_FILE}` and `{CONFIG_FILE}`; delete \
                 `{LEGACY_CONFIG_FILE}`; see {}",
                root.display(),
                rpp::MIGRATION_GUIDE
            );
        }
        let (config, ts) = load_ts(&root)?;
        Ok(Project { root, config, ts })
    }

    /// Path to `rpp.config.ts`.
    pub fn config_path(&self) -> PathBuf {
        self.root.join(CONFIG_FILE)
    }

    /// Every file whose change requires reloading the project config.
    pub fn config_files(&self) -> Vec<PathBuf> {
        let mut files = vec![
            self.config_path(),
            self.root.join(PACKAGE_MANIFEST),
            self.lock_path(),
        ];
        files.extend(self.ts.inputs.iter().cloned());
        for package in self.ts.packages.values() {
            let manifest = package.dir.join(PACKAGE_MANIFEST);
            if manifest.is_file() {
                files.push(manifest);
            }
        }
        files
    }

    /// Path to `rpp.lock`.
    pub fn lock_path(&self) -> PathBuf {
        self.root.join(LOCK_FILE)
    }

    /// The absolute output directory.
    pub fn output_dir(&self) -> PathBuf {
        self.root.join(&self.config.build.output)
    }

    /// The absolute source directory.
    pub fn source_dir(&self) -> PathBuf {
        self.root.join(&self.config.build.source)
    }

    /// The release archive, `<output>/<pack name>.zip`.
    pub fn release_zip(&self) -> PathBuf {
        self.output_dir()
            .join(format!("{}.zip", self.config.pack.name))
    }

    /// The component limits `build.wasm` sets.
    pub fn wasm_limits(&self) -> rpp_wasm::Limits {
        let wasm = &self.config.build.wasm;
        rpp_wasm::Limits {
            deadline: Duration::from_secs(wasm.execution_deadline_seconds),
            memory_bytes: wasm.memory_limit_mb as usize * 1024 * 1024,
        }
    }

    /// Remove the build output directory and the `.rpp` cache without building
    /// an [`Engine`].
    pub fn clean_artifacts(&self) -> Result<()> {
        rpp::engine::clean_project_artifacts(&self.config, &self.root)
            .context("cleaning project artifacts")
    }

    /// Load every configured plugin and build an [`Engine`].
    ///
    /// Components load through the engine in `wasm`, which is created with
    /// [`Self::wasm_limits`] when a plugin first needs one and is left in the slot for reuse.
    pub fn build_engine(&self, wasm: &mut Option<WasmEngine>) -> Result<Engine> {
        let factories = self
            .config
            .plugins
            .iter()
            .map(|plugin| self.load_factory(plugin, wasm))
            .collect::<Result<Vec<_>>>()?;
        Engine::builder(self.config.clone())
            .project_root(&self.root)
            .plugins(factories)
            .build_engine()
            .context("constructing the build engine")
    }

    /// Load `plugin` from its resolved `rpp.json` dependency.
    fn load_factory(
        &self,
        plugin: &PluginConfig,
        wasm: &mut Option<WasmEngine>,
    ) -> Result<Arc<dyn PluginFactory>> {
        let name = &plugin.package;
        let package = self.ts.packages.get(name).with_context(|| {
            format!(
                "plugin package `{name}` is not a dependency in {PACKAGE_MANIFEST}\n\
                 (run `rpp add {name}`)"
            )
        })?;
        let mut components = BTreeMap::new();
        for (component, spec) in &package.manifest.components {
            let path = package.dir.join(&spec.module);
            let compiled = self.wasm_engine(wasm)?.load(&path).with_context(|| {
                format!("loading component `{component}` at {}", path.display())
            })?;
            components.insert(component.clone(), compiled);
        }
        let factory = JsPluginFactory::load(JsPluginSpec {
            dir: &package.dir,
            project_root: &self.root,
            config: &self.config,
            plugin,
            components,
        })
        .with_context(|| format!("loading TypeScript plugin `{}`", package.manifest.id))?;
        Ok(Arc::new(factory))
    }

    /// The engine in `slot`, created first when the slot is empty.
    fn wasm_engine<'a>(&self, slot: &'a mut Option<WasmEngine>) -> Result<&'a WasmEngine> {
        if slot.is_none() {
            let cache = self.root.join(".rpp/cache/wasmtime");
            let engine = WasmEngine::new(self.wasm_limits(), Some(&cache))
                .context("initializing the wasm engine")?;
            *slot = Some(engine);
        }
        Ok(slot.as_ref().expect("the wasm engine was just created"))
    }
}

/// The nearest of `start` (resolved against the current directory) and its ancestors
/// for which `pred` holds.
pub(crate) fn nearest(start: &Path, pred: impl Fn(&Path) -> bool) -> Result<Option<PathBuf>> {
    let start = std::path::absolute(start).context("reading the current directory")?;
    Ok(start
        .ancestors()
        .find(|dir| pred(dir))
        .map(Path::to_path_buf))
}

/// Whether `path` is an `rpp.json` plugin manifest (one with `name` and `version`).
pub(crate) fn is_plugin_manifest(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .is_some_and(|json| json.get("name").is_some() && json.get("version").is_some())
}

/// The version of this rpp, which dependency `rpp` ranges must match.
pub(crate) fn rpp_version() -> Result<Version> {
    Version::parse(env!("CARGO_PKG_VERSION")).context("parsing the rpp version")
}

/// Resolve `rpp.json` dependencies against `rpp.lock` without updating pins, saving the
/// lock when resolution changed it. A missing `rpp.json` has no dependencies.
pub(crate) fn resolve_ts_packages(root: &Path) -> Result<BTreeMap<String, TsPackage>> {
    let manifest_path = root.join(PACKAGE_MANIFEST);
    let dependencies = if manifest_path.is_file() {
        let text = std::fs::read_to_string(&manifest_path)
            .with_context(|| format!("reading {}", manifest_path.display()))?;
        parse_dependencies(&text)?
    } else {
        Vec::new()
    };
    let lock_path = root.join(LOCK_FILE);
    let mut lock = PackageLock::load(&lock_path)?;
    let before = lock.clone();
    let resolved = resolve(
        &Registry::from_env()?,
        root,
        &dependencies,
        &mut lock,
        &rpp_version()?,
        &Update::None,
    )
    .map_err(guide_legacy_package)
    .context("resolving dependencies")?;
    if lock != before {
        lock.save(&lock_path)
            .with_context(|| format!("writing {}", lock_path.display()))?;
    }
    resolved
        .into_iter()
        .map(|package| {
            let manifest = PluginManifest::load(&package.root).with_context(|| {
                format!(
                    "reading plugin manifest for `{}` at {}",
                    package.name,
                    package.root.display()
                )
            })?;
            let dir = package.root;
            Ok((package.name, TsPackage { dir, manifest }))
        })
        .collect()
}

fn load_ts(root: &Path) -> Result<(Config, TsProject)> {
    let packages = resolve_ts_packages(root)?;
    let config_packages = packages
        .iter()
        .map(|(name, package)| {
            (
                name.clone(),
                ConfigPackage {
                    dir: package.dir.clone(),
                    config: package.manifest.config.clone(),
                },
            )
        })
        .collect();
    let evaluated = evaluate_config(root, &config_packages)?;
    Ok((
        evaluated.config,
        TsProject {
            packages,
            inputs: evaluated.inputs,
        },
    ))
}

/// Replace a missing-`rpp.json` error for a directory holding only a legacy `plugin.toml` with
/// the guided rejection from [`PluginManifest::load`].
pub(crate) fn guide_legacy_package(error: rpp_fetch::Error) -> anyhow::Error {
    if let rpp_fetch::Error::MissingPackageManifest(dir) = &error {
        if dir.join("plugin.toml").is_file() {
            if let Err(guided) = PluginManifest::load(dir) {
                return guided.into();
            }
        }
    }
    error.into()
}

/// The rejection for a project still configured by `rpp.toml`.
pub(crate) fn legacy_config_error() -> anyhow::Error {
    anyhow!(
        "`{LEGACY_CONFIG_FILE}` is no longer supported; move it to `{CONFIG_FILE}` and list \
         plugins in `{PACKAGE_MANIFEST}`; see {}",
        rpp::MIGRATION_GUIDE
    )
}
