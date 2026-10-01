//! Per-plugin sandbox: environment construction, stdlib whitelist, builtin
//! module preloading, and plugin-local `require` resolution (spec §4).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mlua::{HookTriggers, Lua, Table, Value, Variadic, VmState};
use parking_lot::Mutex;

use crate::config::LuaCapability;
use crate::host::log::{emit, LogLevel};
use crate::host::RuntimeAccess;
use crate::lua::builtins;
use crate::lua::builtins::log::stringify_values;

/// Default per-Lua-state memory limit (256 MB).
pub(crate) const DEFAULT_MEMORY_LIMIT: usize = 256 * 1024 * 1024;
pub(crate) const DEFAULT_EXECUTION_LIMIT: Duration = Duration::from_secs(60);
pub(crate) type Deadline = Arc<Mutex<Option<Instant>>>;

pub(crate) fn has_lua(access: &RuntimeAccess, cap: LuaCapability) -> bool {
    access.is_native() || access.permissions.lua.contains(&cap)
}

pub(crate) fn install_limits(lua: &Lua, memory_limit: usize) -> mlua::Result<Deadline> {
    lua.set_memory_limit(memory_limit)?;
    let deadline: Deadline = Arc::new(Mutex::new(None));
    let hook_deadline = Arc::clone(&deadline);
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(10_000),
        move |_, _| {
            if hook_deadline
                .lock()
                .is_some_and(|deadline| Instant::now() >= deadline)
            {
                return Err(mlua::Error::runtime("Lua execution deadline exceeded"));
            }
            Ok(VmState::Continue)
        },
    )?;
    Ok(deadline)
}

pub(crate) fn run_limited<T>(
    deadline: &Deadline,
    execution_limit: Duration,
    operation: impl FnOnce() -> mlua::Result<T>,
) -> mlua::Result<T> {
    *deadline.lock() = Some(Instant::now() + execution_limit);
    let result = operation();
    *deadline.lock() = None;
    result
}

/// A sandbox bound to one plugin within one Lua state.
///
/// Holds the plugin's private `_ENV` and the immutable sources used to resolve
/// `require` of plugin-local modules.
pub(crate) struct Sandbox {
    /// The plugin's private global environment.
    pub(crate) env: Table,
}

impl Sandbox {
    /// Build a fresh sandbox `_ENV` for `plugin_id` using captured package sources.
    ///
    /// Installs the whitelisted stdlib, `print` → `log.info`, the `rpp` builtin
    /// modules, and a plugin-local `require` that resolves only builtin `rpp*`
    /// modules and files in the package snapshot.
    pub(crate) fn new(
        lua: &Lua,
        plugin_id: &str,
        modules: Arc<BTreeMap<String, Vec<u8>>>,
        access: RuntimeAccess,
        deadline: Deadline,
        memory_limit: usize,
    ) -> mlua::Result<Self> {
        let env = lua.create_table()?;

        install_stdlib(lua, &env, &access, memory_limit)?;
        install_print(lua, &env, plugin_id)?;
        install_require(lua, &env, plugin_id, modules, access, deadline)?;

        // `_G` self-reference for plugins that reflect on globals.
        env.set("_G", &env)?;

        Ok(Sandbox { env })
    }

    /// Load and execute a chunk in this sandbox, returning its result.
    pub(crate) fn exec(&self, lua: &Lua, name: &str, source: &str) -> mlua::Result<Value> {
        lua.load(source)
            .set_name(name)
            .set_environment(self.env.clone())
            .eval()
    }
}

/// Install the whitelisted standard library into `env`.
fn install_stdlib(
    lua: &Lua,
    env: &Table,
    access: &RuntimeAccess,
    memory_limit: usize,
) -> mlua::Result<()> {
    let g = lua.globals();

    if access.is_native() {
        for pair in g.clone().pairs::<Value, Value>() {
            let (key, value) = pair?;
            env.set(key, value)?;
        }
        return Ok(());
    }

    // Whole tables that are deterministic and safe to expose directly.
    for name in ["string", "table", "utf8"] {
        let value: Value = g.get(name)?;
        env.set(name, value)?;
    }

    // Lua seeds its default PRNG from host process state. Keep deterministic
    // plugins honest by omitting it unless the project explicitly grants
    // randomness (which also disables cache replay for the plugin).
    let source_math: Table = g.get("math")?;
    let math = lua.create_table()?;
    for pair in source_math.pairs::<Value, Value>() {
        let (key, value) = pair?;
        let random_function = match &key {
            Value::String(name) => matches!(name.to_str()?.as_ref(), "random" | "randomseed"),
            _ => false,
        };
        if !random_function || access.permissions.random {
            math.set(key, value)?;
        }
    }
    env.set("math", math)?;

    // Individual safe globals.
    for name in [
        "select",
        "pairs",
        "ipairs",
        "next",
        "tonumber",
        "tostring",
        "type",
        "pcall",
        "xpcall",
        "error",
        "assert",
        "setmetatable",
        "getmetatable",
        "rawget",
        "rawset",
        "rawequal",
        "rawlen",
    ] {
        let value: Value = g.get(name)?;
        if !value.is_nil() {
            env.set(name, value)?;
        }
    }

    // Host clocks make processor/generator results depend on ambient state.
    // Expose only the non-process `os` subset when clocks are explicitly
    // granted; that grant marks the plugin non-replayable.
    if access.permissions.clocks {
        let os_src: Table = g.get("os")?;
        let os = lua.create_table()?;
        for name in ["clock", "time", "date"] {
            let value: Value = os_src.get(name)?;
            os.set(name, value)?;
        }
        env.set("os", os)?;
    }

    if has_lua(access, LuaCapability::Io) {
        let value: Value = g.get("io")?;
        env.set("io", value)?;
    }
    if has_lua(access, LuaCapability::Os) {
        let value: Value = g.get("os")?;
        env.set("os", value)?;
    }
    if has_lua(access, LuaCapability::Load) {
        install_loaders(lua, env, memory_limit)?;
    }
    if has_lua(access, LuaCapability::Debug) {
        let value: Value = g.get("debug")?;
        env.set("debug", value)?;
    }
    if has_lua(access, LuaCapability::Package) {
        // Search paths only: `loadlib`/`cpath` would load native code and
        // `loaded`/`preload`/`searchers` reach the real global table.
        let source: Table = g.get("package")?;
        let package = lua.create_table()?;
        for name in ["path", "config", "searchpath"] {
            package.set(name, source.get::<Value>(name)?)?;
        }
        env.set("package", package)?;
    }

    // `collectgarbage` stub (accepts and ignores arguments, returns 0).
    env.set(
        "collectgarbage",
        lua.create_function(|_, _: Variadic<Value>| Ok(0i64))?,
    )?;

    Ok(())
}

/// Install `load`, `loadfile`, and `dofile` bound to the sandbox `_ENV`.
///
/// Lua's own versions default a chunk's `_ENV` to the real global table, which
/// would hand a plugin `io`/`os` it was never granted.
fn install_loaders(lua: &Lua, env: &Table, memory_limit: usize) -> mlua::Result<()> {
    fn source_limit(lua: &Lua, memory_limit: usize) -> usize {
        memory_limit.saturating_sub(lua.used_memory())
    }

    fn ensure_source_size(size: usize, limit: usize) -> mlua::Result<()> {
        if size > limit {
            return Err(mlua::Error::runtime("Lua source exceeds the memory limit"));
        }
        Ok(())
    }

    fn read_source(path: &str, limit: usize) -> std::io::Result<Vec<u8>> {
        use std::io::Read;

        let mut file = std::fs::File::open(path)?;
        let mut source = Vec::new();
        let mut buffer = [0_u8; 8192];
        loop {
            let remaining = limit.saturating_sub(source.len());
            let read_len = remaining.saturating_add(1).min(buffer.len());
            let count = file.read(&mut buffer[..read_len])?;
            if count == 0 {
                return Ok(source);
            }
            if count > remaining {
                return Err(std::io::Error::other("Lua source exceeds the memory limit"));
            }
            source
                .try_reserve_exact(count)
                .map_err(std::io::Error::other)?;
            source.extend_from_slice(&buffer[..count]);
        }
    }

    fn compile(lua: &Lua, source: Vec<u8>, name: &str, env: Table) -> mlua::Result<(Value, Value)> {
        match lua
            .load(source)
            .set_name(name)
            .set_environment(env)
            .into_function()
        {
            Ok(function) => Ok((Value::Function(function), Value::Nil)),
            Err(error) => Ok((
                Value::Nil,
                Value::String(lua.create_string(error.to_string())?),
            )),
        }
    }

    let load_env = env.clone();
    env.set(
        "load",
        lua.create_function(
            move |lua,
                  (chunk, name, _mode, chunk_env): (
                Value,
                Option<String>,
                Value,
                Option<Table>,
            )| {
                let limit = source_limit(lua, memory_limit);
                let source = match chunk {
                    Value::String(text) => {
                        ensure_source_size(text.as_bytes().len(), limit)?;
                        text.as_bytes().to_vec()
                    }
                    Value::Function(reader) => {
                        let mut source = Vec::new();
                        loop {
                            match reader.call::<Value>(())? {
                                Value::String(piece) if !piece.as_bytes().is_empty() => {
                                    ensure_source_size(
                                        source.len().saturating_add(piece.as_bytes().len()),
                                        limit,
                                    )?;
                                    source
                                        .try_reserve_exact(piece.as_bytes().len())
                                        .map_err(mlua::Error::external)?;
                                    source.extend_from_slice(&piece.as_bytes())
                                }
                                _ => break,
                            }
                        }
                        source
                    }
                    other => {
                        return Err(mlua::Error::external(format!(
                            "load expects a string or function, got {}",
                            other.type_name()
                        )))
                    }
                };
                let name = name.unwrap_or_else(|| "=(load)".into());
                compile(
                    lua,
                    source,
                    &name,
                    chunk_env.unwrap_or_else(|| load_env.clone()),
                )
            },
        )?,
    )?;

    let loadfile_env = env.clone();
    env.set(
        "loadfile",
        lua.create_function(
            move |lua, (path, _mode, chunk_env): (String, Value, Option<Table>)| {
                let source = match read_source(&path, source_limit(lua, memory_limit)) {
                    Ok(source) => source,
                    Err(error) => {
                        return Ok((
                            Value::Nil,
                            Value::String(
                                lua.create_string(format!("cannot open {path}: {error}"))?,
                            ),
                        ))
                    }
                };
                compile(
                    lua,
                    source,
                    &format!("@{path}"),
                    chunk_env.unwrap_or_else(|| loadfile_env.clone()),
                )
            },
        )?,
    )?;

    let dofile_env = env.clone();
    env.set(
        "dofile",
        lua.create_function(move |lua, path: String| {
            let source = read_source(&path, source_limit(lua, memory_limit))
                .map_err(|error| mlua::Error::external(format!("cannot open {path}: {error}")))?;
            lua.load(source)
                .set_name(format!("@{path}"))
                .set_environment(dofile_env.clone())
                .call::<mlua::MultiValue>(())
        })?,
    )?;
    Ok(())
}

/// Map the global `print` to `rpp.log.info`.
fn install_print(lua: &Lua, env: &Table, plugin_id: &str) -> mlua::Result<()> {
    let plugin = plugin_id.to_string();
    env.set(
        "print",
        lua.create_function(move |_, args: Variadic<Value>| {
            emit(&plugin, LogLevel::Info, &stringify_values(args));
            Ok(())
        })?,
    )?;
    Ok(())
}

struct RppModules {
    root: Table,
    json: Table,
    toml: Table,
    hash: Table,
    path: Table,
    log: Table,
    str: Table,
    process: Table,
    component: Table,
}

fn build_rpp_modules(
    lua: &Lua,
    plugin_id: &str,
    access: RuntimeAccess,
    deadline: Deadline,
) -> mlua::Result<RppModules> {
    let json = builtins::json::module(lua)?;
    let toml = builtins::toml_mod::module(lua)?;
    let hash = builtins::hash::module(lua)?;
    let path = builtins::path::module(lua)?;
    let log = builtins::log::table(lua, plugin_id)?;
    let str = builtins::str::module(lua)?;
    let process = builtins::process::module(lua, access.clone(), deadline)?;
    let component = crate::lua::component::module(lua, access)?;

    let root = lua.create_table()?;
    root.set(
        "plugin",
        lua.create_function(move |lua, ()| {
            crate::lua::plugin_builder::PluginBuilder::create_userdata(lua)
        })?,
    )?;
    root.set("json", json.clone())?;
    root.set("toml", toml.clone())?;
    root.set("hash", hash.clone())?;
    root.set("path", path.clone())?;
    root.set("log", log.clone())?;
    root.set("str", str.clone())?;
    root.set("process", process.clone())?;
    root.set("component", component.clone())?;

    Ok(RppModules {
        root,
        json,
        toml,
        hash,
        path,
        log,
        str,
        process,
        component,
    })
}

/// Install a sandboxed `require` plus the preloaded builtin module cache.
fn install_require(
    lua: &Lua,
    env: &Table,
    plugin_id: &str,
    sources: Arc<BTreeMap<String, Vec<u8>>>,
    access: RuntimeAccess,
    deadline: Deadline,
) -> mlua::Result<()> {
    let modules = build_rpp_modules(lua, plugin_id, access, deadline)?;

    // Loaded-module cache, private to this environment.
    let loaded = lua.create_table()?;
    loaded.set("rpp", modules.root)?;
    loaded.set("rpp.json", modules.json)?;
    loaded.set("rpp.toml", modules.toml)?;
    loaded.set("rpp.hash", modules.hash)?;
    loaded.set("rpp.path", modules.path)?;
    loaded.set("rpp.log", modules.log)?;
    loaded.set("rpp.str", modules.str)?;
    loaded.set("rpp.process", modules.process)?;
    loaded.set("rpp.component", modules.component)?;

    let env_for_require = env.clone();
    let require = lua.create_function(move |lua, name: mlua::String| {
        let name = name.to_str()?.to_string();

        // Cache hit.
        if let Ok(cached) = loaded.get::<Value>(name.as_str()) {
            if !cached.is_nil() {
                return Ok(cached);
            }
        }

        // Builtin rpp modules are only those explicitly registered above.
        if name == "rpp" || name.starts_with("rpp.") {
            return Err(mlua::Error::external(format!(
                "module `{name}` not found (unknown rpp builtin)"
            )));
        }

        // Plugin-local resolution.
        let source = resolve_local(&sources, &name)?;
        let module: Value = lua
            .load(source.code)
            .set_name(format!("@{}", source.display))
            .set_environment(env_for_require.clone())
            .eval()?;
        loaded.set(name.as_str(), &module)?;
        Ok(module)
    })?;

    env.set("require", require)?;
    Ok(())
}

struct LocalSource<'a> {
    code: &'a [u8],
    display: String,
}

/// Resolve a plugin-local module name to source, rejecting path escapes.
fn resolve_local<'a>(
    sources: &'a BTreeMap<String, Vec<u8>>,
    name: &str,
) -> mlua::Result<LocalSource<'a>> {
    // Reject names that could escape the package (`..`, absolute, raw slashes).
    if name.contains("..") || name.starts_with('/') || name.contains('/') || name.contains('\\') {
        return Err(mlua::Error::external(format!(
            "require(`{name}`) is not allowed; dots are the only separator"
        )));
    }

    let rel = name.replace('.', "/");
    let candidates = [format!("{rel}.lua"), format!("{rel}/init.lua")];
    for display in candidates {
        if let Some(code) = sources.get(&display) {
            return Ok(LocalSource { code, display });
        }
    }

    Err(mlua::Error::external(format!(
        "module `{name}` not found in plugin directory"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loader_sources_are_bounded() {
        let lua = Lua::new();
        let limit = lua.used_memory() + 8;
        let env = lua.create_table().unwrap();
        install_loaders(&lua, &env, limit).unwrap();

        let load: mlua::Function = env.get("load").unwrap();
        let error = load
            .call::<mlua::MultiValue>(("return 'source larger than limit'",))
            .unwrap_err();
        assert!(error.to_string().contains("memory limit"), "{error}");
    }
}
