//! Shared project plumbing: locate `rpp.toml`, load the [`Config`], resolve all
//! configured plugins (lockfile-aware), construct the right plugin factory for
//! each, and build an [`Engine`].
//!
//! This is the single place the CLI commands go through to turn a directory on
//! disk into a ready-to-run build engine, so resolution, plugin loading, and
//! error messaging are consistent across `build`, `dev`, and friends.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use rpp::config::{Config, PluginConfig};
use rpp::engine::{Engine, EngineBuilder};
use rpp::lua::LuaPluginFactory;
use rpp::manifest::{PluginManifest, Runtime};
use rpp::model::PluginFactory;
use rpp::wasm::WasmPluginFactory;
use rpp_fetch::{Lockfile, Pin, PluginSource, Resolver};
use rpp_wasm::WasmEngine;

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
        Ok(Project { root, config })
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
        validate_build_dirs(&self.config, &self.root)?;
        let output = self.output_dir();
        let rpp_dir = self.root.join(".rpp");
        for dir in [&output, &rpp_dir] {
            if dir.exists() {
                std::fs::remove_dir_all(dir)
                    .with_context(|| format!("removing {}", dir.display()))?;
            }
        }
        Ok(())
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
        let resolver = Resolver::new(&self.root).context("initializing the plugin resolver")?;
        let lock_path = self.lock_path();
        let mut lockfile = Lockfile::load(&lock_path)
            .with_context(|| format!("reading {}", lock_path.display()))?;
        let mut lock_dirty = false;

        let mut factories: Vec<Arc<dyn PluginFactory>> = Vec::new();
        let mut shared_wasm = wasm_engine;

        for plugin_cfg in &self.config.plugins {
            let source = PluginSource::parse(
                &plugin_cfg.source,
                plugin_cfg.r#ref.as_deref(),
                plugin_cfg.subdir.as_deref(),
            )
            .with_context(|| format!("invalid plugin source `{}`", plugin_cfg.source))?;

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
                .with_context(|| format!("resolving plugin `{}`", plugin_cfg.source))?;

            if let Some(pinned) = &resolved.pinned {
                let prev = lockfile.record_resolved(&source, &resolved);
                if pin_changed(prev.as_ref(), pinned, plugin_cfg.subdir.as_deref()) {
                    lock_dirty = true;
                }
            }

            let manifest = PluginManifest::load(&resolved.root).with_context(|| {
                format!(
                    "reading plugin manifest for `{}` at {}",
                    plugin_cfg.source,
                    resolved.root.display()
                )
            })?;

            let factory: Arc<dyn PluginFactory> = match manifest.runtime {
                Runtime::Lua => {
                    let limits = rpp::lua::LuaPluginFactory::limits_from_build(&self.config.build);
                    Arc::new(
                        LuaPluginFactory::load_with_limits(
                            &resolved.root,
                            plugin_cfg.options.clone(),
                            self.config.pack.name.clone(),
                            self.config.pack.description.clone(),
                            self.config.pack.pack_format,
                            limits,
                        )
                        .with_context(|| format!("loading Lua plugin `{}`", manifest.id))?,
                    )
                }
                Runtime::Wasm => {
                    let engine = match &shared_wasm {
                        Some(e) => e.clone(),
                        None => {
                            let e = WasmEngine::new()
                                .map_err(|e| anyhow!("initializing the wasm engine: {e}"))?;
                            shared_wasm = Some(e.clone());
                            e
                        }
                    };
                    Arc::new(
                        WasmPluginFactory::load(
                            &engine,
                            &resolved.root,
                            &manifest,
                            plugin_cfg.options.clone(),
                        )
                        .with_context(|| format!("loading WASM plugin `{}`", manifest.id))?,
                    )
                }
            };

            factories.push(factory);
        }

        if lock_dirty {
            lockfile
                .save(&lock_path)
                .with_context(|| format!("writing {}", lock_path.display()))?;
        }

        Ok(ResolvedFactories {
            factories,
            wasm_engine: shared_wasm,
        })
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
    let parsed = PluginSource::parse(
        &plugin.source,
        plugin.r#ref.as_deref(),
        plugin.subdir.as_deref(),
    )
    .with_context(|| format!("invalid plugin source `{}`", plugin.source))?;
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
        .with_context(|| format!("resolving plugin `{}`", plugin.source))?;
    let (id, version) = validate_plugin_dir(&resolved.root)?;

    Ok(Some(PluginMeta {
        id,
        version,
        source: plugin.source.clone(),
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
        if plugin.source == id_or_source {
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

/// Whether a freshly-resolved pin differs from what the lockfile recorded.
fn pin_changed(locked: Option<&rpp_fetch::LockedPlugin>, pin: &Pin, subdir: Option<&str>) -> bool {
    match locked {
        None => true,
        Some(l) => l.commit != pin.commit || l.ref_ != pin.ref_ || l.subdir.as_deref() != subdir,
    }
}

/// Mirror [`Engine`]'s build-directory validation so `clean` cannot delete
/// paths outside the project.
fn validate_build_dirs(config: &Config, project_root: &Path) -> Result<()> {
    let source = &config.build.source;
    let output = &config.build.output;
    for (label, path) in [("source", source), ("output", output)] {
        if path.as_os_str().is_empty()
            || path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            bail!(
                "`build.{label}` in {} must be a normalized project-relative path",
                project_root.join(CONFIG_FILE).display()
            );
        }
        if path.starts_with(".rpp") {
            bail!(
                "`build.{label}` in {} must not be inside `.rpp`",
                project_root.join(CONFIG_FILE).display()
            );
        }
    }
    if source == output || source.starts_with(output) || output.starts_with(source) {
        bail!(
            "`build.source` and `build.output` in {} must be separate directories",
            project_root.join(CONFIG_FILE).display()
        );
    }
    Ok(())
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
    if manifest.runtime == Runtime::Wasm && manifest.module.is_none() {
        bail!("wasm plugin `{}` is missing a `module` entry", manifest.id);
    }
    Ok((manifest.id, manifest.version.to_string()))
}
