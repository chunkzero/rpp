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

use anyhow::{anyhow, bail, Context, Result};
use rpp::config::{Config, PluginConfig};
use rpp::engine::{Engine, EngineBuilder};
use rpp::host::{PackInfo, RuntimeAccess};
use rpp::js::{evaluate_config, ConfigPackage, JsPluginFactory, JsPluginLimits};
use rpp::manifest::PluginManifest;
use rpp::model::PluginFactory;
use rpp_fetch::registry::{
    parse_dependencies, resolve, PackageLock, Registry, Update, PACKAGE_MANIFEST,
};
use rpp_wasm::WasmEngine;

use crate::commands::deps::rpp_version;

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
        let root = find_project_root(start).ok_or_else(|| {
            anyhow!(
                "no `{CONFIG_FILE}` found in `{}` or any parent directory\n\
                 (run `rpp init` to scaffold a new project)",
                start.display()
            )
        })?;
        if root.join(CONFIG_FILE).is_file() {
            if root.join(LEGACY_CONFIG_FILE).is_file() {
                bail!(
                    "`{}` contains both `{LEGACY_CONFIG_FILE}` and `{CONFIG_FILE}`; delete \
                     `{LEGACY_CONFIG_FILE}`; see {}",
                    root.display(),
                    rpp::MIGRATION_GUIDE
                );
            }
        } else {
            return Err(legacy_config_error());
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

    /// Remove the build output directory and the `.rpp` cache without building
    /// an [`Engine`].
    pub fn clean_artifacts(&self) -> Result<()> {
        rpp::engine::clean_project_artifacts(&self.config, &self.root)
            .context("cleaning project artifacts")
    }

    /// Load all configured plugins into factories, building an [`Engine`].
    ///
    /// A shared [`WasmEngine`] is created lazily only if a wasm plugin is
    /// present; it is returned so the dev server can reuse it across rebuilds.
    pub fn build_engine(&self) -> Result<Engine> {
        let (engine, _wasm) = self.build_engine_with_wasm(None)?;
        Ok(engine)
    }

    /// Like [`Self::build_engine`] but accepts (and returns) a shared
    /// [`WasmEngine`] so the dev server can reuse one across reloads.
    pub fn build_engine_with_wasm(
        &self,
        wasm_engine: Option<WasmEngine>,
    ) -> Result<(Engine, Option<WasmEngine>)> {
        let factories = self.resolve_factories(wasm_engine.clone())?;
        let wasm_engine = factories.wasm_engine.or(wasm_engine);

        let builder: EngineBuilder = Engine::builder(self.config.clone())
            .project_root(&self.root)
            .plugins(factories.factories);

        let engine = builder
            .build_engine()
            .context("constructing the build engine")?;
        Ok((engine, wasm_engine))
    }

    /// Load each configured `package` plugin from the resolved `rpp.json` dependencies.
    fn resolve_factories(&self, wasm_engine: Option<WasmEngine>) -> Result<ResolvedFactories> {
        let mut shared_wasm = wasm_engine;
        let mut factories = Vec::new();
        for plugin_cfg in &self.config.plugins {
            let name = plugin_cfg.label();
            let package = self.ts.packages.get(name).ok_or_else(|| {
                anyhow!(
                    "plugin package `{name}` is not a dependency in {PACKAGE_MANIFEST}\n\
                     (run `rpp add {name}`)"
                )
            })?;
            let factory = self.load_factory(
                ResolvedPackage {
                    manifest: package.manifest.clone(),
                    root: package.dir.clone(),
                    config: plugin_cfg.clone(),
                },
                &mut shared_wasm,
            )?;
            factories.push(factory);
        }
        Ok(ResolvedFactories {
            factories,
            wasm_engine: shared_wasm,
        })
    }

    fn load_factory(
        &self,
        package: ResolvedPackage,
        shared_wasm: &mut Option<WasmEngine>,
    ) -> Result<Arc<dyn PluginFactory>> {
        let ResolvedPackage {
            manifest,
            root,
            config: plugin_cfg,
        } = package;
        plugin_cfg.validate(&self.config_path())?;
        let id = manifest.id.clone();

        let engine = if manifest.components.is_empty() {
            shared_wasm.clone()
        } else {
            match shared_wasm {
                Some(engine) => Some(engine.clone()),
                None => {
                    let wasm = &self.config.build.wasm;
                    let limits = rpp_wasm::Limits {
                        deadline: std::time::Duration::from_secs(wasm.execution_deadline_seconds),
                        memory_bytes: wasm.memory_limit_mb as usize * 1024 * 1024,
                    };
                    let engine =
                        WasmEngine::new(limits, Some(&self.root.join(".rpp/cache/wasmtime")))
                            .map_err(|error| anyhow!("initializing the wasm engine: {error}"))?;
                    *shared_wasm = Some(engine.clone());
                    Some(engine)
                }
            }
        };
        let mut components = std::collections::BTreeMap::new();
        if let Some(engine) = engine.as_ref() {
            for (name, component) in &manifest.components {
                let path = root.join(&component.module);
                let compiled = engine
                    .load(&path)
                    .with_context(|| format!("loading component `{name}` at {}", path.display()))?;
                components.insert(name.clone(), compiled);
            }
        }
        let access = RuntimeAccess::new(
            plugin_cfg.security,
            plugin_cfg.permissions.clone(),
            self.root.clone(),
            components,
            plugin_cfg.outputs.clone(),
        );
        let pack = PackInfo {
            name: self.config.pack.name.clone(),
            description: self.config.pack.description.clone(),
            format: self.config.pack.pack_format,
        };
        let limits = &self.config.build.limits;
        let limits = JsPluginLimits {
            memory_limit: limits.memory_limit_mb as usize * 1024 * 1024,
            execution_limit: std::time::Duration::from_secs(limits.execution_deadline_seconds),
        };
        let factory: Arc<dyn PluginFactory> = Arc::new(
            JsPluginFactory::load(
                &root,
                plugin_cfg.options.clone(),
                pack,
                limits,
                access,
                &self.source_dir(),
                Some(&self.root.join(".rpp/cache")),
            )
            .with_context(|| format!("loading TypeScript plugin `{id}`"))?,
        );

        Ok(factory)
    }

    /// The absolute output directory.
    pub fn output_dir(&self) -> PathBuf {
        self.root.join(&self.config.build.output)
    }

    /// The absolute source directory.
    pub fn source_dir(&self) -> PathBuf {
        self.root.join(&self.config.build.source)
    }
}

struct ResolvedFactories {
    factories: Vec<Arc<dyn PluginFactory>>,
    wasm_engine: Option<WasmEngine>,
}

#[derive(Clone)]
struct ResolvedPackage {
    manifest: PluginManifest,
    root: PathBuf,
    config: PluginConfig,
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
    let evaluated = evaluate_config(root, &config_packages, JsPluginLimits::default())?;
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

/// Walk up from `start` looking for a directory containing `rpp.config.ts` or the retired
/// `rpp.toml`.
pub fn find_project_root(start: &Path) -> Option<PathBuf> {
    let start = if start.is_absolute() {
        start.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(start)
    };
    let mut dir = start.as_path();
    loop {
        if dir.join(CONFIG_FILE).is_file() || dir.join(LEGACY_CONFIG_FILE).is_file() {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
}
