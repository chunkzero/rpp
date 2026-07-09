//! Shared project plumbing: locate `rpp.toml`, load the [`Config`], resolve all
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
use rpp::lua::LuaPluginFactory;
use rpp::manifest::PluginManifest;
use rpp::model::PluginFactory;
use rpp_fetch::{Lockfile, Pin, PluginSource, Resolver};
use rpp_wasm::WasmEngine;

use crate::user_plugins::UserPlugins;

/// The config file name.
pub const CONFIG_FILE: &str = "rpp.toml";
/// The lockfile name.
pub const LOCK_FILE: &str = "rpp.lock";

/// A located project: the directory containing `rpp.toml` plus the parsed
/// config.
pub struct Project {
    /// The directory containing `rpp.toml`.
    pub root: PathBuf,
    /// The parsed configuration.
    pub config: Config,
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

impl Project {
    /// Locate and load the project containing `start` (searching ancestors).
    pub fn discover(start: &Path) -> Result<Self> {
        Self::discover_with_user_plugins(start, UserPlugins::load()?)
    }

    /// Locate a project without consulting machine-global plugin state.
    pub fn discover_isolated(start: &Path) -> Result<Self> {
        let root = find_project_root(start).ok_or_else(|| {
            anyhow!(
                "no `{CONFIG_FILE}` found in `{}` or any parent directory",
                start.display()
            )
        })?;
        Self::discover_with_user_plugins(
            &root,
            UserPlugins {
                root: root.join(".rpp/isolated-home"),
                plugins: Vec::new(),
            },
        )
    }

    fn discover_with_user_plugins(start: &Path, user_plugins: UserPlugins) -> Result<Self> {
        let root = find_project_root(start).ok_or_else(|| {
            anyhow!(
                "no `{CONFIG_FILE}` found in `{}` or any parent directory\n\
                 (run `rpp init` to scaffold a new project)",
                start.display()
            )
        })?;
        let config_path = root.join(CONFIG_FILE);
        let config = Config::load(&config_path)
            .with_context(|| format!("loading {}", config_path.display()))?;
        Ok(Project {
            root,
            config,
            user_plugins,
        })
    }

    /// Path to `rpp.toml`.
    pub fn config_path(&self) -> PathBuf {
        self.root.join(CONFIG_FILE)
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

    /// Resolve and load every configured plugin into a factory, in order.
    fn resolve_factories(&self, wasm_engine: Option<WasmEngine>) -> Result<ResolvedFactories> {
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

        let mut factories: Vec<LoadedFactory> = Vec::new();
        let mut shared_wasm = wasm_engine;

        let mut global_plugins = BTreeMap::<String, PluginConfig>::new();

        for plugin_cfg in &self.user_plugins.plugins {
            let loaded = self.resolve_factory(
                plugin_cfg,
                &global_resolver,
                &mut global_lock,
                &mut global_lock_dirty,
                &mut shared_wasm,
                PluginScope::Global,
            )?;
            if factories.iter().any(|existing| existing.id == loaded.id) {
                bail!(
                    "global plugin id `{}` is configured more than once",
                    loaded.id
                );
            }
            global_plugins.insert(loaded.id.clone(), plugin_cfg.clone());
            factories.push(loaded);
        }

        for plugin_cfg in &self.config.plugins {
            let loaded = if let Some(id) = plugin_cfg.id.as_deref() {
                let Some(global_cfg) = global_plugins.get(id) else {
                    bail!("project plugin id `{id}` does not match an installed global plugin");
                };
                let effective_cfg = PluginConfig {
                    id: None,
                    source: global_cfg.source.clone(),
                    r#ref: global_cfg.r#ref.clone(),
                    subdir: global_cfg.subdir.clone(),
                    options: plugin_cfg.options.clone(),
                    security: plugin_cfg.security,
                    permissions: plugin_cfg.permissions.clone(),
                    outputs: plugin_cfg.outputs.clone(),
                };
                self.resolve_factory(
                    &effective_cfg,
                    &global_resolver,
                    &mut global_lock,
                    &mut global_lock_dirty,
                    &mut shared_wasm,
                    PluginScope::Project,
                )?
            } else {
                self.resolve_factory(
                    plugin_cfg,
                    &project_resolver,
                    &mut project_lock,
                    &mut project_lock_dirty,
                    &mut shared_wasm,
                    PluginScope::Project,
                )?
            };
            if let Some(index) = factories
                .iter()
                .position(|existing| existing.id == loaded.id)
            {
                if factories[index].scope == PluginScope::Project {
                    bail!(
                        "project plugin id `{}` is configured more than once",
                        loaded.id
                    );
                }
                factories.remove(index);
            }
            factories.push(loaded);
        }

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

        Ok(ResolvedFactories {
            factories: factories.into_iter().map(|loaded| loaded.factory).collect(),
            wasm_engine: shared_wasm,
        })
    }

    fn resolve_factory(
        &self,
        plugin_cfg: &PluginConfig,
        resolver: &Resolver,
        lockfile: &mut Lockfile,
        lock_dirty: &mut bool,
        shared_wasm: &mut Option<WasmEngine>,
        scope: PluginScope,
    ) -> Result<LoadedFactory> {
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
            .get_for(
                &canonical,
                plugin_cfg.r#ref.as_deref(),
                plugin_cfg.subdir.as_deref(),
            )
            .cloned();
        let resolved = resolver
            .resolve(&source, locked.as_ref())
            .with_context(|| format!("resolving plugin `{source_value}`"))?;

        if let Some(pinned) = &resolved.pinned {
            let prev = lockfile.record_resolved(&source, &resolved);
            if pin_changed(prev.as_ref(), pinned, plugin_cfg.subdir.as_deref()) {
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
        let id = manifest.id.clone();
        let _ =
            crate::luals::write_plugin_stubs(&self.root.join(".rpp").join("api"), &resolved.root);

        let engine = if manifest.components.is_empty() {
            shared_wasm.clone()
        } else {
            match shared_wasm {
                Some(engine) => Some(engine.clone()),
                None => {
                    let engine = WasmEngine::with_cache_dir(self.root.join(".rpp/cache/wasmtime"))
                        .map_err(|error| anyhow!("initializing the wasm engine: {error}"))?;
                    *shared_wasm = Some(engine.clone());
                    Some(engine)
                }
            }
        };
        let mut components = std::collections::BTreeMap::new();
        if let Some(engine) = engine.as_ref() {
            for (name, component) in &manifest.components {
                let path = resolved.root.join(&component.module);
                let compiled = engine
                    .load(&path)
                    .with_context(|| format!("loading component `{name}` at {}", path.display()))?;
                components.insert(name.clone(), compiled);
            }
        }
        let access = rpp::lua::RuntimeAccess::new(
            plugin_cfg.security,
            plugin_cfg.permissions.clone(),
            self.root.clone(),
            components,
            plugin_cfg.outputs.clone(),
        );
        let limits = rpp::lua::LuaPluginFactory::limits_from_build(&self.config.build);
        let factory: Arc<dyn PluginFactory> = Arc::new(
            LuaPluginFactory::load_with_limits_and_access(
                &resolved.root,
                plugin_cfg.options.clone(),
                self.config.pack.name.clone(),
                self.config.pack.description.clone(),
                self.config.pack.pack_format,
                limits,
                access,
            )
            .with_context(|| format!("loading Lua plugin `{}`", manifest.id))?,
        );

        Ok(LoadedFactory { id, factory, scope })
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
        .get_for(
            &canonical,
            plugin.r#ref.as_deref(),
            plugin.subdir.as_deref(),
        )
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

/// Find a configured plugin by exact `source` string or resolved plugin id.
pub fn find_plugin_by_id_or_source<'a>(
    project: &'a Project,
    id_or_source: &str,
    lock: &Lockfile,
    resolver: &Resolver,
) -> Result<Option<&'a PluginConfig>> {
    for plugin in &project.config.plugins {
        if plugin.source.as_deref() == Some(id_or_source)
            || plugin.id.as_deref() == Some(id_or_source)
        {
            return Ok(Some(plugin));
        }
        if let Some(meta) = resolve_plugin_meta(plugin, lock, resolver)? {
            if meta.id == id_or_source || meta.canonical == id_or_source {
                return Ok(Some(plugin));
            }
        }
    }
    Ok(None)
}

struct ResolvedFactories {
    factories: Vec<Arc<dyn PluginFactory>>,
    wasm_engine: Option<WasmEngine>,
}

struct LoadedFactory {
    id: String,
    factory: Arc<dyn PluginFactory>,
    scope: PluginScope,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PluginScope {
    Global,
    Project,
}

/// Whether a freshly-resolved pin differs from what the lockfile recorded.
fn pin_changed(locked: Option<&rpp_fetch::LockedPlugin>, pin: &Pin, subdir: Option<&str>) -> bool {
    match locked {
        None => true,
        Some(l) => l.commit != pin.commit || l.ref_ != pin.ref_ || l.subdir.as_deref() != subdir,
    }
}

/// Walk up from `start` looking for a directory containing `rpp.toml`.
pub fn find_project_root(start: &Path) -> Option<PathBuf> {
    let start = if start.is_absolute() {
        start.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(start)
    };
    let mut dir = start.as_path();
    loop {
        if dir.join(CONFIG_FILE).is_file() {
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
