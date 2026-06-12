//! Per-plugin sandbox: environment construction, stdlib whitelist, builtin
//! module preloading, and plugin-local `require` resolution (spec §4).

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mlua::{HookTriggers, Lua, Table, Value, Variadic, VmState};
use parking_lot::Mutex;

use crate::lua::builtins;
use crate::lua::builtins::log::{emit, LogLevel};

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
    operation: impl FnOnce() -> mlua::Result<T>,
) -> mlua::Result<T> {
    *deadline.lock() = Some(Instant::now() + DEFAULT_EXECUTION_LIMIT);
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
    pub(crate) fn new(lua: &Lua, plugin_id: &str, root: &Path) -> mlua::Result<Self> {
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let env = lua.create_table()?;

        install_stdlib(lua, &env)?;
        install_print(lua, &env, plugin_id)?;
        install_require(lua, &env, plugin_id, &root)?;

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
fn install_stdlib(lua: &Lua, env: &Table) -> mlua::Result<()> {
    let g = lua.globals();

    // Whole tables that are safe to expose directly.
    for name in ["string", "table", "math", "utf8"] {
        let value: Value = g.get(name)?;
        env.set(name, value)?;
    }

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

    // A restricted `os` table: only clock/time/date.
    let os_src: Table = g.get("os")?;
    let os = lua.create_table()?;
    for name in ["clock", "time", "date"] {
        let value: Value = os_src.get(name)?;
        os.set(name, value)?;
    }
    env.set("os", os)?;

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
            let mut parts = Vec::with_capacity(args.len());
            for v in args.iter() {
                let part = match v {
                    Value::String(s) => s.to_string_lossy().to_string(),
                    Value::Integer(i) => i.to_string(),
                    Value::Number(n) => n.to_string(),
                    Value::Boolean(b) => b.to_string(),
                    Value::Nil => "nil".to_string(),
                    other => format!("<{}>", other.type_name()),
                };
                parts.push(part);
            }
            emit(&plugin, LogLevel::Info, &parts.join("\t"));
            Ok(())
        })?,
    )?;
    Ok(())
}

/// Install a sandboxed `require` plus the preloaded builtin module cache.
fn install_require(lua: &Lua, env: &Table, plugin_id: &str, root: &Path) -> mlua::Result<()> {
    // Loaded-module cache, private to this environment.
    let loaded = lua.create_table()?;
    loaded.set("rpp", build_rpp_root(lua, plugin_id)?)?;
    loaded.set("rpp.json", builtins::json::module(lua)?)?;
    loaded.set("rpp.toml", builtins::toml_mod::module(lua)?)?;
    loaded.set("rpp.hash", builtins::hash::module(lua)?)?;
    loaded.set("rpp.path", builtins::path::module(lua)?)?;
    loaded.set("rpp.log", builtins::log::table(lua, plugin_id)?)?;
    loaded.set("rpp.str", builtins::str::module(lua)?)?;

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

/// Build the root `rpp` module table: `rpp.plugin()` plus submodule re-exports.
fn build_rpp_root(lua: &Lua, plugin_id: &str) -> mlua::Result<Table> {
    let t = lua.create_table()?;

    t.set(
        "plugin",
        lua.create_function(move |lua, ()| {
            crate::lua::plugin_builder::PluginBuilder::create_userdata(lua)
        })?,
    )?;

    t.set("json", builtins::json::module(lua)?)?;
    t.set("toml", builtins::toml_mod::module(lua)?)?;
    t.set("hash", builtins::hash::module(lua)?)?;
    t.set("path", builtins::path::module(lua)?)?;
    t.set("log", builtins::log::table(lua, plugin_id)?)?;
    t.set("str", builtins::str::module(lua)?)?;

    Ok(t)
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
            return Ok(LocalSource {
                code,
                display: canon.display().to_string(),
            });
        }
    }

    Err(mlua::Error::external(format!(
        "module `{name}` not found in plugin directory"
    )))
}
