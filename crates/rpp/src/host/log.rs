//! Plugin log routing shared by all runtimes.

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
