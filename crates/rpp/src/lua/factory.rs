//! [`LuaPluginFactory`]: validation-load on the main thread plus per-worker
//! instantiation (spec §4).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::config::LuaConfig;
use crate::error::{Error, Result};
use crate::host::{PackInfo, PhaseCell, RuntimeAccess};
use crate::lua::bootstrap::eval_entry;
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

impl From<&LuaConfig> for LuaPluginLimits {
    /// Read limits from `[build.lua]` (validated non-zero by `Config::load`).
    fn from(lua: &LuaConfig) -> Self {
        Self {
            memory_limit: lua.memory_limit_mb as usize * 1024 * 1024,
            execution_limit: Duration::from_secs(lua.execution_deadline_seconds),
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
    root: PathBuf,
    entry: String,
    entry_source: String,
    modules: Arc<BTreeMap<String, Vec<u8>>>,
    options: toml::Value,
    pack: PackInfo,
    processors: Vec<ProcessorDef>,
    has_generator: bool,
    memory_limit: usize,
    execution_limit: Duration,
    access: RuntimeAccess,
    cache_key: u64,
    overrides: Vec<String>,
}

impl LuaPluginFactory {
    /// Load a Lua plugin from its package directory and validate it.
    ///
    /// Reads `plugin.toml`, runs the entry script in a throwaway sandbox to
    /// extract processor definitions and detect a generator, and computes the
    /// cache key. `pack` carries pack metadata for `ctx.pack`; `access` is the
    /// host capability policy from the `[[plugin]]` entry.
    pub fn load(
        dir: impl AsRef<Path>,
        options: toml::Value,
        pack: PackInfo,
        limits: LuaPluginLimits,
        access: RuntimeAccess,
    ) -> Result<Self> {
        let dir = dir.as_ref();
        let manifest_path = dir.join("plugin.toml");
        let manifest_source =
            std::fs::read_to_string(&manifest_path).map_err(|e| Error::io(&manifest_path, e))?;
        let manifest = PluginManifest::parse(&manifest_source, &manifest_path)?;
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
        let mut modules = BTreeMap::new();
        collect_lua(&canonical_root, &canonical_root, &mut modules)?;
        let entry_bytes = if let Some(bytes) = modules.get(&manifest.entry) {
            bytes.clone()
        } else {
            std::fs::read(&canonical_entry).map_err(|e| Error::io(&entry_path, e))?
        };
        let entry_source = String::from_utf8(entry_bytes).map_err(|e| Error::PluginLoad {
            plugin: manifest.id.clone(),
            message: format!("cannot read entry `{}`: {e}", manifest.entry),
        })?;
        let modules = Arc::new(modules);
        let cache_key = compute_cache_key(
            &root,
            &manifest,
            &manifest_source,
            &entry_source,
            &modules,
            &options,
            &access,
        )?;

        // Validation load: extract processor defs and generator presence.
        let (processors, has_generator) = validation_load(
            &manifest.id,
            Arc::clone(&modules),
            &manifest.entry,
            &entry_source,
            limits,
            access.clone(),
        )?;

        Ok(LuaPluginFactory {
            shared: Arc::new(Shared {
                id: manifest.id,
                overrides: manifest.overrides,
                root,
                entry: manifest.entry,
                entry_source,
                modules,
                options,
                pack,
                processors,
                has_generator,
                memory_limit: limits.memory_limit,
                execution_limit: limits.execution_limit,
                access,
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

    pub(crate) fn modules(&self) -> Arc<BTreeMap<String, Vec<u8>>> {
        Arc::clone(&self.shared.modules)
    }

    pub(crate) fn memory_limit(&self) -> usize {
        self.shared.memory_limit
    }

    pub(crate) fn execution_limit(&self) -> Duration {
        self.shared.execution_limit
    }

    /// The plugin's access policy with a fresh phase tracker. Phase is per
    /// instance: sharing one cell between worker threads would let one
    /// worker's processor call observe another's phase transitions.
    pub(crate) fn access(&self) -> RuntimeAccess {
        let mut access = self.shared.access.clone();
        access.phase = PhaseCell::new();
        access
    }
}

impl PluginFactory for LuaPluginFactory {
    fn id(&self) -> &str {
        &self.shared.id
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

    fn cacheable_processors(&self) -> bool {
        self.shared.access.is_deterministic()
    }

    fn cacheable_generator(&self) -> bool {
        self.shared.access.is_deterministic()
    }

    fn output_roots(&self) -> std::collections::BTreeMap<String, PathBuf> {
        self.shared.access.outputs.clone()
    }

    fn overrides(&self) -> &[String] {
        &self.shared.overrides
    }

    fn instantiate(&self) -> Result<Box<dyn PluginInstance>> {
        Ok(Box::new(LuaPluginInstance::new(self.clone())?))
    }
}

/// Run the entry script once to collect processor defs and generator presence.
fn validation_load(
    plugin_id: &str,
    modules: Arc<BTreeMap<String, Vec<u8>>>,
    entry_name: &str,
    entry_source: &str,
    limits: LuaPluginLimits,
    access: RuntimeAccess,
) -> Result<(Vec<ProcessorDef>, bool)> {
    let eval = eval_entry(
        plugin_id,
        modules,
        entry_name,
        entry_source,
        limits.memory_limit,
        limits.execution_limit,
        access,
    )?;

    let builder = extract_builder(plugin_id, eval.value)?;
    let inner = builder.inner.lock();

    // Processors are returned in declaration order (as registered).
    let processors: Vec<ProcessorDef> = inner.processors.iter().map(|p| p.def.clone()).collect();

    Ok((processors, inner.generator.is_some()))
}

/// Compute the cache key: xxh3 over sorted `*.lua` files + `plugin.toml` +
/// canonicalized options.
fn compute_cache_key(
    root: &Path,
    manifest: &PluginManifest,
    manifest_source: &str,
    entry_source: &str,
    modules: &BTreeMap<String, Vec<u8>>,
    options: &toml::Value,
    access: &RuntimeAccess,
) -> Result<u64> {
    let mut writer = HashWriter::new();

    writer.write_str("rpp.lua.plugin.v2");
    for (rel, bytes) in modules {
        writer.write_str(rel);
        writer.write(bytes);
    }
    writer.write_str("entry");
    writer.write_str(&manifest.entry);
    writer.write(entry_source.as_bytes());
    writer.write_str("plugin.toml");
    writer.write(manifest_source.as_bytes());

    // A Lua wrapper and its component binary form one plugin implementation.
    // Hash declared component bytes explicitly so replacing a `.wasm` file
    // invalidates processor/generator replay even when Lua and config are
    // unchanged.
    for (name, component) in &manifest.components {
        let path = root.join(&component.module);
        let bytes = std::fs::read(&path).map_err(|e| Error::io(&path, e))?;
        writer.write_str("component");
        writer.write_str(name);
        writer.write_str(&component.module);
        writer.write(&bytes);
    }

    writer.write_str("options");
    writer.write(canonical_options_json(options).as_bytes());

    writer.write_str("host-access");
    let access_key =
        serde_json::to_vec(&(access.security, &access.permissions, &access.outputs))
            .map_err(|error| Error::Build(format!("failed to hash plugin host access: {error}")))?;
    writer.write(&access_key);

    Ok(writer.finish())
}

fn collect_lua(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) -> Result<()> {
    let entries = std::fs::read_dir(dir).map_err(|e| Error::io(dir, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::io(dir, e))?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|e| Error::io(&path, e))?;
        if file_type.is_symlink() {
            let target = path.canonicalize().map_err(|e| Error::io(&path, e))?;
            if !target.starts_with(root) {
                return Err(Error::Build(format!(
                    "Lua package symlink `{}` escapes the plugin directory",
                    path.display()
                )));
            }
            if target.is_dir() {
                return Err(Error::Build(format!(
                    "Lua package directory symlinks are unsupported: `{}`",
                    path.display()
                )));
            }
        }
        if file_type.is_dir() {
            collect_lua(root, &path, out)?;
        } else if (file_type.is_file() || file_type.is_symlink())
            && path.extension().and_then(|e| e.to_str()) == Some("lua")
        {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = std::fs::read(&path).map_err(|e| Error::io(&path, e))?;
            out.insert(rel, bytes);
        }
    }
    Ok(())
}
