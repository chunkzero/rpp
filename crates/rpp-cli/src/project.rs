//! Shared project plumbing: locate `rpp.toml` (or `rpp.config.ts`), load the [`Config`], resolve all
//! configured plugins (lockfile-aware), construct the right plugin factory for
//! each, and build an [`Engine`].
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
use rpp::lua::{LuaPluginFactory, LuaPluginLimits};
use rpp::manifest::PluginManifest;
use rpp::model::PluginFactory;
use rpp_fetch::registry::{
    parse_dependencies, resolve, PackageLock, Registry, Update, PACKAGE_MANIFEST,
};
use rpp_fetch::{Lockfile, Pin, PluginSource, Resolver};
use rpp_wasm::WasmEngine;

use crate::commands::deps::rpp_version;
use crate::user_plugins::UserPlugins;

/// The config file name.
pub const CONFIG_FILE: &str = "rpp.toml";
/// The TypeScript config file name.
pub const TS_CONFIG_FILE: &str = rpp::js::CONFIG_FILE;
/// The lockfile name.
pub const LOCK_FILE: &str = "rpp.lock";

/// A located project: the directory containing `rpp.toml` or `rpp.config.ts`
/// plus the parsed config.
pub struct Project {
    /// The directory containing the project config file.
    pub root: PathBuf,
    /// The parsed configuration.
    pub config: Config,
    /// Present for `rpp.config.ts` projects.
    pub(crate) ts: Option<TsProject>,
    /// User-level plugins loaded from `~/.rpp/plugins.toml`.
    pub(crate) user_plugins: UserPlugins,
}

/// Resolved plugin identity for list/remove/update matching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginMeta {
    /// The plugin id from `plugin.toml`.
    pub id: String,
    /// The plugin version string.
    pub version: String,
    /// The configured `source` string from `rpp.toml`.
    pub source: String,
    /// The canonical source key used in the lockfile.
    pub canonical: String,
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
        Self::discover_with_user_plugins(start, UserPlugins::load()?)
    }

    /// Locate a project without consulting machine-global plugin state.
    pub fn discover_isolated(start: &Path) -> Result<Self> {
        let mut project = Self::discover_with_user_plugins(start, UserPlugins::default())?;
        project.user_plugins.root = project.root.join(".rpp/isolated-home");
        Ok(project)
    }

    fn discover_with_user_plugins(start: &Path, user_plugins: UserPlugins) -> Result<Self> {
        let root = find_project_root(start).ok_or_else(|| {
            anyhow!(
                "no `{CONFIG_FILE}` or `{TS_CONFIG_FILE}` found in `{}` or any parent directory\n\
                 (run `rpp init` to scaffold a new project)",
                start.display()
            )
        })?;
        if root.join(CONFIG_FILE).is_file() && root.join(TS_CONFIG_FILE).is_file() {
            bail!(
                "`{}` contains both `{CONFIG_FILE}` and `{TS_CONFIG_FILE}`; remove one",
                root.display()
            );
        }
        let (config, ts) = if root.join(TS_CONFIG_FILE).is_file() {
            let (config, ts) = load_ts(&root)?;
            (config, Some(ts))
        } else {
            let config_path = root.join(CONFIG_FILE);
            let config = Config::load(&config_path)
                .with_context(|| format!("loading {}", config_path.display()))?;
            (config, None)
        };
        Ok(Project {
            root,
            config,
            ts,
            user_plugins,
        })
    }

    /// Path to `rpp.toml`, or `rpp.config.ts` for TypeScript projects.
    pub fn config_path(&self) -> PathBuf {
        let name = if self.ts.is_some() {
            TS_CONFIG_FILE
        } else {
            CONFIG_FILE
        };
        self.root.join(name)
    }

    /// Every file whose change requires reloading the project config.
    pub fn config_files(&self) -> Vec<PathBuf> {
        let mut files = vec![self.config_path()];
        if let Some(ts) = &self.ts {
            files.push(self.root.join(PACKAGE_MANIFEST));
            files.push(self.lock_path());
            files.extend(ts.inputs.iter().cloned());
            for package in ts.packages.values() {
                files.extend(
                    [PACKAGE_MANIFEST, "plugin.toml"]
                        .iter()
                        .map(|name| package.dir.join(name))
                        .filter(|path| path.is_file()),
                );
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

    /// Resolve all `[[plugin]]` entries to plugin factories, building an
    /// [`Engine`]. Newly-resolved GitHub pins are written back to `rpp.lock`.
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

    /// Resolve package metadata, select overrides, then load effective plugins in order.
    fn resolve_factories(&self, wasm_engine: Option<WasmEngine>) -> Result<ResolvedFactories> {
        if let Some(ts) = &self.ts {
            return self.resolve_ts_factories(ts, wasm_engine);
        }
        let global_resolver = Resolver::new(&self.user_plugins.root)
            .context("initializing the global plugin resolver")?;
        let project_resolver =
            Resolver::new(&self.root).context("initializing the project plugin resolver")?;
        let global_lock_path = self.user_plugins.lock_path();
        let project_lock_path = self.lock_path();
        let mut global_lock = Lockfile::load(&global_lock_path)
            .with_context(|| format!("reading {}", global_lock_path.display()))?;
        let mut project_lock = Lockfile::load(&project_lock_path)
            .with_context(|| format!("reading {}", project_lock_path.display()))?;
        let mut global_lock_dirty = false;
        let mut project_lock_dirty = false;

        let mut selected: Vec<ResolvedPackage> = Vec::new();
        let mut shared_wasm = wasm_engine;
        let mut global_plugins = BTreeMap::new();

        for plugin_cfg in &self.user_plugins.plugins {
            let package = self.resolve_package(
                plugin_cfg,
                &global_resolver,
                &mut global_lock,
                &mut global_lock_dirty,
                PluginScope::Global,
            )?;
            let id = package.manifest.id.clone();
            if global_plugins.insert(id.clone(), package.clone()).is_some() {
                bail!("global plugin id `{id}` is configured more than once");
            }
            selected.push(package);
        }

        for plugin_cfg in &self.config.plugins {
            let package = if let Some(id) = plugin_cfg.id.as_deref() {
                let Some(global) = global_plugins.get(id) else {
                    bail!("project plugin id `{id}` does not match an installed global plugin");
                };
                ResolvedPackage {
                    config: plugin_cfg.clone(),
                    scope: PluginScope::Project,
                    ..global.clone()
                }
            } else {
                self.resolve_package(
                    plugin_cfg,
                    &project_resolver,
                    &mut project_lock,
                    &mut project_lock_dirty,
                    PluginScope::Project,
                )?
            };
            if let Some(index) = selected
                .iter()
                .position(|existing| existing.manifest.id == package.manifest.id)
            {
                if selected[index].scope == PluginScope::Project {
                    bail!(
                        "project plugin id `{}` is configured more than once",
                        package.manifest.id
                    );
                }
                selected.remove(index);
            }
            selected.push(package);
        }

        let factories = selected
            .into_iter()
            .map(|package| self.load_factory(package, &mut shared_wasm))
            .collect::<Result<Vec<_>>>()?;

        // Pins for requests no longer configured (including pre-v2 default-branch
        // pins that now load as explicit refs) are dropped on save.
        global_lock_dirty |= global_lock.prune(&configured_sources(&self.user_plugins.plugins));
        project_lock_dirty |= project_lock.prune(&configured_sources(&self.config.plugins));
        if global_lock_dirty {
            self.user_plugins.ensure_root()?;
            global_lock
                .save(&global_lock_path)
                .with_context(|| format!("writing {}", global_lock_path.display()))?;
        }
        if project_lock_dirty {
            project_lock
                .save(&project_lock_path)
                .with_context(|| format!("writing {}", project_lock_path.display()))?;
        }

        self.write_plugin_stubs(&factories);

        Ok(ResolvedFactories {
            factories: factories.into_iter().map(|loaded| loaded.factory).collect(),
            wasm_engine: shared_wasm,
        })
    }

    /// Load each configured `package` plugin from the resolved `rpp.json` dependencies.
    fn resolve_ts_factories(
        &self,
        ts: &TsProject,
        wasm_engine: Option<WasmEngine>,
    ) -> Result<ResolvedFactories> {
        let mut shared_wasm = wasm_engine;
        let mut factories = Vec::new();
        for plugin_cfg in &self.config.plugins {
            let name = plugin_cfg.label();
            let package = ts.packages.get(name).ok_or_else(|| {
                anyhow!(
                    "plugin package `{name}` is not a dependency in {PACKAGE_MANIFEST}\n\
                     (run `rpp add {name}`)"
                )
            })?;
            let loaded = self.load_factory(
                ResolvedPackage {
                    manifest: package.manifest.clone(),
                    root: package.dir.clone(),
                    config: plugin_cfg.clone(),
                    scope: PluginScope::Project,
                },
                &mut shared_wasm,
            )?;
            factories.push(loaded.factory);
        }
        Ok(ResolvedFactories {
            factories,
            wasm_engine: shared_wasm,
        })
    }

    /// Refresh editor definitions shipped by plugins (`luals/*.lua`) into
    /// `.rpp/api`. Best-effort: failures are reported, never fatal.
    fn write_plugin_stubs(&self, factories: &[LoadedFactory]) {
        let api_dir = self.root.join(".rpp").join("api");
        let mut owners = BTreeMap::new();
        for loaded in factories {
            if let Err(error) =
                crate::luals::write_plugin_stubs(&api_dir, &loaded.id, &loaded.root, &mut owners)
            {
                tracing::warn!("plugin `{}` editor definitions: {error:#}", loaded.id);
            }
        }
    }

    fn resolve_package(
        &self,
        plugin_cfg: &PluginConfig,
        resolver: &Resolver,
        lockfile: &mut Lockfile,
        lock_dirty: &mut bool,
        scope: PluginScope,
    ) -> Result<ResolvedPackage> {
        let source_value = plugin_cfg
            .source
            .as_deref()
            .ok_or_else(|| anyhow!("plugin `{}` has no source to resolve", plugin_cfg.label()))?;
        let source = PluginSource::parse(
            source_value,
            plugin_cfg.r#ref.as_deref(),
            plugin_cfg.subdir.as_deref(),
        )
        .with_context(|| format!("invalid plugin source `{source_value}`"))?;

        let canonical = source.canonical();
        let locked = lockfile
            .get_for(&canonical, source.requested_ref(), source.subdir())
            .cloned();
        let resolved = resolver
            .resolve(&source, locked.as_ref())
            .with_context(|| format!("resolving plugin `{source_value}`"))?;

        if let Some(pinned) = &resolved.pinned {
            let prev = lockfile.record_resolved(&source, &resolved);
            if pin_changed(prev.as_ref(), pinned, source.subdir()) {
                *lock_dirty = true;
            }
        }

        let manifest = PluginManifest::load(&resolved.root).with_context(|| {
            format!(
                "reading plugin manifest for `{}` at {}",
                plugin_cfg.label(),
                resolved.root.display()
            )
        })?;
        Ok(ResolvedPackage {
            manifest,
            root: resolved.root,
            config: plugin_cfg.clone(),
            scope,
        })
    }

    fn load_factory(
        &self,
        package: ResolvedPackage,
        shared_wasm: &mut Option<WasmEngine>,
    ) -> Result<LoadedFactory> {
        let ResolvedPackage {
            manifest,
            root,
            config: plugin_cfg,
            scope,
        } = package;
        let config_path = match scope {
            PluginScope::Global => self.user_plugins.manifest_path(),
            PluginScope::Project => self.config_path(),
        };
        plugin_cfg.validate(&config_path)?;
        let id = manifest.id.clone();
        if self.ts.is_some() && !rpp::js::is_js_entry(&manifest.entry) {
            bail!("plugin `{id}` is a Lua plugin; {TS_CONFIG_FILE} projects support TypeScript plugins only");
        }

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
                    let engine = WasmEngine::with_limits_and_cache(
                        limits,
                        self.root.join(".rpp/cache/wasmtime"),
                    )
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
        let factory: Arc<dyn PluginFactory> = if rpp::js::is_js_entry(&manifest.entry) {
            let lua = &self.config.build.lua;
            let limits = JsPluginLimits {
                memory_limit: lua.memory_limit_mb as usize * 1024 * 1024,
                execution_limit: std::time::Duration::from_secs(lua.execution_deadline_seconds),
            };
            Arc::new(
                JsPluginFactory::load(
                    &root,
                    plugin_cfg.options.clone(),
                    pack,
                    limits,
                    access,
                    &self.source_dir(),
                )
                .with_context(|| format!("loading TypeScript plugin `{id}`"))?,
            )
        } else {
            Arc::new(
                LuaPluginFactory::load(
                    &root,
                    plugin_cfg.options.clone(),
                    pack,
                    LuaPluginLimits::from(&self.config.build.lua),
                    access,
                )
                .with_context(|| format!("loading Lua plugin `{id}`"))?,
            )
        };

        Ok(LoadedFactory { id, root, factory })
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

/// Resolve a configured plugin to its id/version, when possible.
///
/// GitHub sources without a lockfile pin return `None` (the resolver would
/// need a network fetch).
pub fn resolve_plugin_meta(
    plugin: &PluginConfig,
    lock: &Lockfile,
    resolver: &Resolver,
) -> Result<Option<PluginMeta>> {
    let Some(source_value) = plugin.source.as_deref() else {
        return Ok(None);
    };
    let parsed = PluginSource::parse(
        source_value,
        plugin.r#ref.as_deref(),
        plugin.subdir.as_deref(),
    )
    .with_context(|| format!("invalid plugin source `{source_value}`"))?;
    let canonical = parsed.canonical();

    let locked = lock
        .get_for(&canonical, parsed.requested_ref(), parsed.subdir())
        .cloned();
    if matches!(parsed, PluginSource::GitHub { .. }) && locked.is_none() {
        return Ok(None);
    }

    let resolved = resolver
        .resolve(&parsed, locked.as_ref())
        .with_context(|| format!("resolving plugin `{source_value}`"))?;
    let (id, version) = validate_plugin_dir(&resolved.root)?;

    Ok(Some(PluginMeta {
        id,
        version,
        source: source_value.to_string(),
        canonical,
    }))
}

struct ResolvedFactories {
    factories: Vec<Arc<dyn PluginFactory>>,
    wasm_engine: Option<WasmEngine>,
}

struct LoadedFactory {
    id: String,
    root: PathBuf,
    factory: Arc<dyn PluginFactory>,
}

#[derive(Clone)]
struct ResolvedPackage {
    manifest: PluginManifest,
    root: PathBuf,
    config: PluginConfig,
    scope: PluginScope,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PluginScope {
    Global,
    Project,
}

/// Whether a freshly-resolved pin differs from what the lockfile recorded.
fn configured_sources(plugins: &[PluginConfig]) -> Vec<PluginSource> {
    plugins
        .iter()
        .filter_map(|plugin| {
            let source = plugin.source.as_deref()?;
            PluginSource::parse(source, plugin.r#ref.as_deref(), plugin.subdir.as_deref()).ok()
        })
        .collect()
}

fn pin_changed(locked: Option<&rpp_fetch::LockedPlugin>, pin: &Pin, subdir: Option<&str>) -> bool {
    match locked {
        None => true,
        Some(l) => l.commit != pin.commit || l.ref_ != pin.ref_ || l.subdir.as_deref() != subdir,
    }
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

/// Walk up from `start` looking for a directory containing `rpp.toml` or `rpp.config.ts`.
pub fn find_project_root(start: &Path) -> Option<PathBuf> {
    let start = if start.is_absolute() {
        start.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(start)
    };
    let mut dir = start.as_path();
    loop {
        if dir.join(CONFIG_FILE).is_file() || dir.join(TS_CONFIG_FILE).is_file() {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
}

/// Validate that a plugin package directory exists and parses (used by
/// `plugin add`). Returns the `(id, version)` summary.
pub fn validate_plugin_dir(dir: &Path) -> Result<(String, String)> {
    let manifest = PluginManifest::load(dir)
        .with_context(|| format!("reading plugin manifest at {}", dir.display()))?;
    Ok((manifest.id, manifest.version.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(root: &Path, id: &str, lua: &str, component: bool) {
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(
            root.join("plugin.toml"),
            format!(
                "[plugin]\nid = \"{id}\"\nversion = \"1.0.0\"\n{}",
                if component {
                    "[component.broken]\nmodule = \"missing.wasm\"\n"
                } else {
                    ""
                }
            ),
        )
        .unwrap();
        std::fs::write(root.join("init.lua"), lua).unwrap();
    }

    fn project(root: &Path, global: &str, local: &str) -> Project {
        let config = Config::parse(
            &format!("[pack]\nname = \"test\"\n{local}"),
            root.join("rpp.toml"),
        )
        .unwrap();
        let globals = Config::parse(
            &format!("[pack]\nname = \"global\"\n{global}"),
            root.join("plugins.toml"),
        )
        .unwrap();
        Project {
            root: root.into(),
            config,
            ts: None,
            user_plugins: UserPlugins {
                root: root.into(),
                plugins: globals.plugins,
                ..UserPlugins::default()
            },
        }
    }

    #[test]
    fn overrides_skip_broken_global_runtimes_and_preserve_order() {
        for component in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path();
            let valid = "return require('rpp').plugin()";
            package(
                &root.join("broken"),
                "shared",
                "error('overridden plugin executed')",
                component,
            );
            for id in ["first", "last", "local"] {
                package(
                    &root.join(id),
                    if id == "local" { "shared" } else { id },
                    valid,
                    false,
                );
            }
            let project = project(root,
                "[[plugin]]\nsource = 'path:first'\n[[plugin]]\nsource = 'path:broken'\n[[plugin]]\nsource = 'path:last'\n",
                "[[plugin]]\nsource = 'path:local'\n");
            let loaded = project.resolve_factories(None).unwrap();
            assert_eq!(
                loaded.factories.iter().map(|f| f.id()).collect::<Vec<_>>(),
                ["first", "last", "shared"]
            );
            assert!(loaded.wasm_engine.is_none());
        }
    }

    #[test]
    fn id_override_loads_only_with_project_capabilities() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        package(
            &root.join("shared"),
            "shared",
            "assert(os.time()); return require('rpp').plugin()",
            false,
        );
        let project = project(root, "[[plugin]]\nsource = 'path:shared'\n",
            "[[plugin]]\nid = 'shared'\nsecurity = 'trusted'\n[plugin.permissions]\nclocks = true\n");
        let loaded = project.resolve_factories(None).unwrap();
        assert_eq!(loaded.factories.len(), 1);
        assert_eq!(loaded.factories[0].id(), "shared");
    }
}
