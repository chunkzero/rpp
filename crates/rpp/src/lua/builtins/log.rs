//! `rpp.log` builtin and the shared log table used by `ctx.log`.

use mlua::{Lua, MultiValue, Table};

/// Severity of a plugin log message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    /// Verbose diagnostic detail.
    Debug,
    /// Informational message.
    Info,
    /// A recoverable concern.
    Warn,
    /// An error condition reported by the plugin.
    Error,
}

impl LogLevel {
    #[cfg_attr(feature = "tracing", allow(dead_code))]
    fn as_str(self) -> &'static str {
        match self {
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }
}

/// Emit a plugin log line. Routes through `tracing` when the feature is enabled,
/// otherwise prints to stderr.
pub(crate) fn emit(plugin: &str, level: LogLevel, message: &str) {
    #[cfg(feature = "tracing")]
    {
        match level {
            LogLevel::Debug => tracing::debug!(plugin, "{message}"),
            LogLevel::Info => tracing::info!(plugin, "{message}"),
            LogLevel::Warn => tracing::warn!(plugin, "{message}"),
            LogLevel::Error => tracing::error!(plugin, "{message}"),
        }
    }
    #[cfg(not(feature = "tracing"))]
    {
        eprintln!("[{}] [{plugin}] {message}", level.as_str());
    }
}

/// Build a log table (`debug|info|warn|error`) bound to a plugin id.
///
/// Used both for the `rpp.log` module and for `ctx.log`.
pub(crate) fn table(lua: &Lua, plugin_id: &str) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    for (name, level) in [
        ("debug", LogLevel::Debug),
        ("info", LogLevel::Info),
        ("warn", LogLevel::Warn),
        ("error", LogLevel::Error),
    ] {
        let plugin = plugin_id.to_string();
        t.set(
            name,
            lua.create_function(move |_, args: MultiValue| {
                let message = stringify(args);
                emit(&plugin, level, &message);
                Ok(())
            })?,
        )?;
    }
    Ok(t)
}

/// Concatenate log arguments space-separated, like Lua's `print`.
fn stringify(args: MultiValue) -> String {
    use mlua::Value;
    let mut parts: Vec<String> = Vec::with_capacity(args.len());
    for v in args {
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
    parts.join(" ")
}
