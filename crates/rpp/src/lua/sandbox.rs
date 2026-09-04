//! Per-plugin sandbox: environment construction, stdlib whitelist, builtin
//! module preloading, and plugin-local `require` resolution (spec §4).

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mlua::{HookTriggers, Lua, Table, Value, Variadic, VmState};
use parking_lot::Mutex;

use crate::lua::builtins;
use crate::lua::builtins::log::{emit, stringify_values, LogLevel};
use crate::lua::runtime::RuntimeAccess;

/// Default per-Lua-state memory limit (256 MB).
pub(crate) const DEFAULT_MEMORY_LIMIT: usize = 256 * 1024 * 1024;
pub(crate) const DEFAULT_EXECUTION_LIMIT: Duration = Duration::from_secs(60);
pub(crate) type Deadline = Arc<Mutex<Option<Instant>>>;

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
/// Holds the plugin's private `_ENV` and the root directory used to resolve
/// `require` of plugin-local modules.
pub(crate) struct Sandbox {
    /// The plugin's private global environment.
    pub(crate) env: Table,
}

impl Sandbox {
    /// Build a fresh sandbox `_ENV` for `plugin_id` rooted at `root`.
    ///
    /// Installs the whitelisted stdlib, `print` → `log.info`, the `rpp` builtin
    /// modules, and a plugin-local `require` that resolves only builtin `rpp*`
    /// modules and files within `root`.
    pub(crate) fn new(
        lua: &Lua,
        plugin_id: &str,
        root: &Path,
        access: RuntimeAccess,
        deadline: Deadline,
    ) -> mlua::Result<Self> {
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let env = lua.create_table()?;

        install_stdlib(lua, &env, &access)?;
        install_print(lua, &env, plugin_id)?;
        install_require(lua, &env, plugin_id, &root, access, deadline)?;

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
fn install_stdlib(lua: &Lua, env: &Table, access: &RuntimeAccess) -> mlua::Result<()> {
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
        "unpack",
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

    if access.has_lua(crate::config::LuaCapability::Io) {
        let value: Value = g.get("io")?;
        env.set("io", value)?;
    }
    if access.has_lua(crate::config::LuaCapability::Os) {
        let value: Value = g.get("os")?;
        env.set("os", value)?;
    }
    if access.has_lua(crate::config::LuaCapability::Load) {
        for name in ["load", "loadfile", "dofile"] {
            let value: Value = g.get(name)?;
            if !value.is_nil() {
                env.set(name, value)?;
            }
        }
    }
    if access.has_lua(crate::config::LuaCapability::Debug) {
        let value: Value = g.get("debug")?;
        env.set("debug", value)?;
    }
    if access.has_lua(crate::config::LuaCapability::Package) {
        let value: Value = g.get("package")?;
        env.set("package", value)?;
    }

    // `collectgarbage` stub (accepts and ignores arguments, returns 0).
    env.set(
        "collectgarbage",
        lua.create_function(|_, _: Variadic<Value>| Ok(0i64))?,
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
    root: &Path,
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

    let root = root.to_path_buf();
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
        let source = resolve_local(&root, &name)?;
        let module: Value = lua
            .load(&source.code)
            .set_name(format!("@{}", source.display))
            .set_environment(env_for_require.clone())
            .eval()?;
        loaded.set(name.as_str(), &module)?;
        Ok(module)
    })?;

    env.set("require", require)?;
    Ok(())
}

struct LocalSource {
    code: String,
    display: String,
}

/// Resolve a plugin-local module name to source, rejecting path escapes.
fn resolve_local(root: &Path, name: &str) -> mlua::Result<LocalSource> {
    // Reject names that could escape the package (`..`, absolute, raw slashes).
    if name.contains("..") || name.starts_with('/') || name.contains('/') || name.contains('\\') {
        return Err(mlua::Error::external(format!(
            "require(`{name}`) is not allowed; dots are the only separator"
        )));
    }

    let rel = name.replace('.', "/");
    let candidates = [
        root.join(format!("{rel}.lua")),
        root.join(&rel).join("init.lua"),
    ];

    for candidate in candidates {
        if let Ok(canon) = candidate.canonicalize() {
            // Defense in depth: ensure the resolved file is under root.
            if !canon.starts_with(root) {
                return Err(mlua::Error::external(format!(
                    "require(`{name}`) escapes the plugin directory"
                )));
            }
            let code = std::fs::read_to_string(&canon)
                .map_err(|e| mlua::Error::external(format!("require(`{name}`): {e}")))?;
            let display = canon
                .strip_prefix(root)
                .unwrap_or(&canon)
                .to_string_lossy()
                .replace('\\', "/");
            return Ok(LocalSource { code, display });
        }
    }

    Err(mlua::Error::external(format!(
        "module `{name}` not found in plugin directory"
    )))
}
