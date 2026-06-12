//! [`LuaPluginFactory`]: validation-load on the main thread plus per-worker
//! instantiation (spec §4).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::config::{BuildConfig, LuaConfig};
use crate::error::{Error, Result};
use crate::lua::bootstrap::eval_entry;
use crate::lua::ctx::PackInfo;
use crate::lua::instance::{extract_builder, LuaPluginInstance};
use crate::lua::sandbox::{DEFAULT_EXECUTION_LIMIT, DEFAULT_MEMORY_LIMIT};
use crate::manifest::PluginManifest;
use crate::model::{PluginFactory, PluginInstance, ProcessorDef};
use crate::util::canonical::canonical_options_json;
use crate::util::hash::HashWriter;

/// Resource limits applied to each Lua plugin instance.
#[derive(Debug, Clone, Copy)]
pub struct LuaPluginLimits {
    /// Per-state memory limit in bytes.
    pub memory_limit: usize,
    /// Maximum wall-clock time for a single Lua call.
    pub execution_limit: Duration,
}

impl Default for LuaPluginLimits {
    fn default() -> Self {
        Self {
            memory_limit: DEFAULT_MEMORY_LIMIT,
            execution_limit: DEFAULT_EXECUTION_LIMIT,
        }
    }
}

impl LuaPluginLimits {
    /// Read limits from `[build.lua]`; zero values keep the sandbox defaults.
    pub fn from_build_config(build: &BuildConfig) -> Self {
        Self::from_lua_config(&build.lua)
    }

    /// Read limits from `[build.lua]`; zero values keep the sandbox defaults.
    pub fn from_lua_config(lua: &LuaConfig) -> Self {
        Self {
            memory_limit: if lua.memory_limit_mb == 0 {
                DEFAULT_MEMORY_LIMIT
            } else {
                lua.memory_limit_mb as usize * 1024 * 1024
            },
            execution_limit: if lua.execution_deadline_seconds == 0 {
                DEFAULT_EXECUTION_LIMIT
            } else {
                Duration::from_secs(lua.execution_deadline_seconds)
            },
        }
    }
}

/// Immutable, shareable data describing a loaded Lua plugin.
///
/// Cloning is cheap (an `Arc` bump). Shared across worker threads.
#[derive(Clone)]
pub struct LuaPluginFactory {
    shared: Arc<Shared>,
}

struct Shared {
    id: String,
    version: String,
    root: PathBuf,
    entry: String,
    entry_source: String,
    options: toml::Value,
    pack: PackInfo,
    processors: Vec<ProcessorDef>,
    has_generator: bool,
    memory_limit: usize,
    execution_limit: Duration,
    cache_key: u64,
}

impl LuaPluginFactory {
    /// Read sandbox limits from `[build.lua]`.
    pub fn limits_from_build(build: &BuildConfig) -> LuaPluginLimits {
        LuaPluginLimits::from_build_config(build)
    }

    /// Load a Lua plugin from its package directory and validate it.
    ///
    /// Reads `plugin.toml`, runs the entry script in a throwaway sandbox to
    /// extract processor definitions and detect a generator, and computes the
    /// cache key. `pack` carries pack metadata for `ctx.pack`.
    pub fn load(
        dir: impl AsRef<Path>,
        options: toml::Value,
        pack_name: impl Into<String>,
        pack_description: Option<String>,
        pack_format: Option<u32>,
    ) -> Result<Self> {
        Self::load_with_limits(
            dir,
            options,
            pack_name,
            pack_description,
            pack_format,
            LuaPluginLimits::default(),
        )
    }

    /// Like [`Self::load`], but with explicit sandbox resource limits.
    pub fn load_with_limits(
        dir: impl AsRef<Path>,
        options: toml::Value,
        pack_name: impl Into<String>,
        pack_description: Option<String>,
        pack_format: Option<u32>,
        limits: LuaPluginLimits,
    ) -> Result<Self> {
        let dir = dir.as_ref();
        let manifest = PluginManifest::load(dir)?;
        let root = dir.to_path_buf();

        let entry_path = root.join(&manifest.entry);
        let canonical_root = root.canonicalize().map_err(|e| Error::io(&root, e))?;
        let canonical_entry = entry_path
            .canonicalize()
            .map_err(|e| Error::io(&entry_path, e))?;
        if !canonical_entry.starts_with(&canonical_root) {
            return Err(Error::PluginLoad {
                plugin: manifest.id.clone(),
                message: format!("entry `{}` escapes the plugin directory", manifest.entry),
            });
        }
        let entry_source = std::fs::read_to_string(&entry_path).map_err(|e| Error::PluginLoad {
            plugin: manifest.id.clone(),
            message: format!("cannot read entry `{}`: {e}", manifest.entry),
        })?;

        let pack = PackInfo {
            name: pack_name.into(),
            description: pack_description,
            format: pack_format,
        };

        let cache_key = compute_cache_key(&root, &options)?;

        // Validation load: extract processor defs and generator presence.
        let (processors, has_generator) =
            validation_load(&manifest.id, &root, &manifest.entry, &entry_source, limits)?;

        Ok(LuaPluginFactory {
            shared: Arc::new(Shared {
                id: manifest.id,
                version: manifest.version.to_string(),
                root,
                entry: manifest.entry,
                entry_source,
                options,
                pack,
                processors,
                has_generator,
                memory_limit: limits.memory_limit,
                execution_limit: limits.execution_limit,
                cache_key,
            }),
        })
    }

    /// The plugin root directory.
    pub fn root(&self) -> &Path {
        &self.shared.root
    }

    pub(crate) fn pack(&self) -> &PackInfo {
        &self.shared.pack
    }

    pub(crate) fn options(&self) -> &toml::Value {
        &self.shared.options
    }

    pub(crate) fn entry(&self) -> (&str, &str) {
        (&self.shared.entry, &self.shared.entry_source)
    }

    pub(crate) fn memory_limit(&self) -> usize {
        self.shared.memory_limit
    }

    pub(crate) fn execution_limit(&self) -> Duration {
        self.shared.execution_limit
    }
}

impl PluginFactory for LuaPluginFactory {
    fn id(&self) -> &str {
        &self.shared.id
    }

    fn version(&self) -> &str {
        &self.shared.version
    }

    fn cache_key(&self) -> u64 {
        self.shared.cache_key
    }

    fn processors(&self) -> &[ProcessorDef] {
        &self.shared.processors
    }

    fn has_generator(&self) -> bool {
        self.shared.has_generator
    }

    fn instantiate(&self) -> Result<Box<dyn PluginInstance>> {
        Ok(Box::new(LuaPluginInstance::new(self.clone())?))
    }
}

/// Run the entry script once to collect processor defs and generator presence.
fn validation_load(
    plugin_id: &str,
    root: &Path,
    entry_name: &str,
    entry_source: &str,
    limits: LuaPluginLimits,
) -> Result<(Vec<ProcessorDef>, bool)> {
    let eval = eval_entry(
        plugin_id,
        root,
        entry_name,
        entry_source,
        limits.memory_limit,
        limits.execution_limit,
    )?;

    let builder = extract_builder(plugin_id, eval.value)?;
    let inner = builder.inner.lock();

    // Processors are returned in declaration order (as registered).
    let processors: Vec<ProcessorDef> = inner.processors.iter().map(|p| p.def.clone()).collect();

    Ok((processors, inner.generator.is_some()))
}

/// Compute the cache key: xxh3 over sorted `*.lua` files + `plugin.toml` +
/// canonicalized options.
fn compute_cache_key(root: &Path, options: &toml::Value) -> Result<u64> {
    let mut writer = HashWriter::new();

    // Collect all `*.lua` files under root, sorted by relative path.
    let mut lua_files: Vec<(String, Vec<u8>)> = Vec::new();
    collect_lua(root, root, &mut lua_files)?;
    lua_files.sort_by(|a, b| a.0.cmp(&b.0));

    writer.write_str("rpp.lua.plugin.v1");
    for (rel, bytes) in &lua_files {
        writer.write_str(rel);
        writer.write(bytes);
    }

    // plugin.toml
    let manifest_path = root.join("plugin.toml");
    let manifest = std::fs::read(&manifest_path).map_err(|e| Error::io(&manifest_path, e))?;
    writer.write_str("plugin.toml");
    writer.write(&manifest);

    writer.write_str("options");
    writer.write(canonical_options_json(options).as_bytes());

    Ok(writer.finish())
}

fn collect_lua(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Result<()> {
    let entries = std::fs::read_dir(dir).map_err(|e| Error::io(dir, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::io(dir, e))?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|e| Error::io(&path, e))?;
        if file_type.is_dir() {
            collect_lua(root, &path, out)?;
        } else if file_type.is_file() && path.extension().and_then(|e| e.to_str()) == Some("lua") {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = std::fs::read(&path).map_err(|e| Error::io(&path, e))?;
            out.push((rel, bytes));
        }
    }
    Ok(())
}
