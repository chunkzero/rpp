//! [`LuaPluginFactory`]: validation-load on the main thread plus per-worker
//! instantiation (spec §4).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mlua::Lua;

use crate::error::{Error, Result};
use crate::lua::ctx::PackInfo;
use crate::lua::instance::LuaPluginInstance;
use crate::lua::sandbox::{install_limits, run_limited, Sandbox, DEFAULT_MEMORY_LIMIT};
use crate::lua::traceback;
use crate::manifest::PluginManifest;
use crate::model::{PluginFactory, PluginInstance, ProcessorDef};
use crate::util::hash::HashWriter;

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
    cache_key: u64,
}

impl LuaPluginFactory {
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
            validation_load(&manifest.id, &root, &manifest.entry, &entry_source)?;

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
                memory_limit: DEFAULT_MEMORY_LIMIT,
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
) -> Result<(Vec<ProcessorDef>, bool)> {
    let lua = Lua::new();
    let deadline = install_limits(&lua, DEFAULT_MEMORY_LIMIT).map_err(|e| Error::PluginLoad {
        plugin: plugin_id.to_string(),
        message: traceback::render(&e),
    })?;
    let sandbox = Sandbox::new(&lua, plugin_id, root).map_err(|e| Error::PluginLoad {
        plugin: plugin_id.to_string(),
        message: traceback::render(&e),
    })?;

    let value = run_limited(&deadline, || {
        sandbox.exec(&lua, &format!("@{entry_name}"), entry_source)
    })
    .map_err(|e| Error::PluginLoad {
        plugin: plugin_id.to_string(),
        message: traceback::render(&e),
    })?;

    let builder = crate::lua::instance::extract_builder(plugin_id, value)?;
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

    // Canonicalized options: re-serialize so formatting differences do not matter.
    let canonical = canonicalize_options(options);
    writer.write_str("options");
    writer.write(canonical.as_bytes());

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

/// Produce a deterministic string form of options independent of TOML formatting.
fn canonicalize_options(options: &toml::Value) -> String {
    // serde_json with sorted keys gives a stable canonical form.
    let json: serde_json::Value = serde_json::to_value(options).unwrap_or(serde_json::Value::Null);
    canonical_json(&json)
}

fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut parts = Vec::with_capacity(keys.len());
            for k in keys {
                parts.push(format!(
                    "{}:{}",
                    serde_json::to_string(k).unwrap_or_default(),
                    canonical_json(&map[k])
                ));
            }
            format!("{{{}}}", parts.join(","))
        }
        serde_json::Value::Array(arr) => {
            let parts: Vec<String> = arr.iter().map(canonical_json).collect();
            format!("[{}]", parts.join(","))
        }
        other => other.to_string(),
    }
}
