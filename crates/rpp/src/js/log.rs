//! Routing of plugin `console` output.

use rpp_js::{Log, LogLevel};

/// Emit `logs` labelled with `plugin`. Routes through `tracing` when the feature is enabled,
/// otherwise prints to stderr.
pub(super) fn emit(plugin: &str, logs: &[Log]) {
    for Log { level, message } in logs {
        #[cfg(feature = "tracing")]
        match level {
            LogLevel::Debug => tracing::debug!(plugin, "{message}"),
            LogLevel::Info => tracing::info!(plugin, "{message}"),
            LogLevel::Warn => tracing::warn!(plugin, "{message}"),
            LogLevel::Error => tracing::error!(plugin, "{message}"),
        }
        #[cfg(not(feature = "tracing"))]
        {
            let level = match level {
                LogLevel::Debug => "debug",
                LogLevel::Info => "info",
                LogLevel::Warn => "warn",
                LogLevel::Error => "error",
            };
            eprintln!("[{level}] [{plugin}] {message}");
        }
    }
}
