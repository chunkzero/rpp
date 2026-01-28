/// Logging API for plugins.
pub struct LogApi {
    plugin_name: String,
}

impl LogApi {
    pub fn new() -> Self {
        Self {
            plugin_name: String::from("unknown"),
        }
    }

    pub fn with_plugin_name(name: String) -> Self {
        Self { plugin_name: name }
    }

    pub fn debug(&self, message: &str) {
        #[cfg(feature = "tracing")]
        tracing::debug!(plugin = %self.plugin_name, "{}", message);
        #[cfg(not(feature = "tracing"))]
        {
            let _ = message;
        }
    }

    pub fn info(&self, message: &str) {
        #[cfg(feature = "tracing")]
        tracing::info!(plugin = %self.plugin_name, "{}", message);
        #[cfg(not(feature = "tracing"))]
        {
            let _ = message;
        }
    }

    pub fn warn(&self, message: &str) {
        #[cfg(feature = "tracing")]
        tracing::warn!(plugin = %self.plugin_name, "{}", message);
        #[cfg(not(feature = "tracing"))]
        {
            let _ = message;
        }
    }

    pub fn error(&self, message: &str) {
        #[cfg(feature = "tracing")]
        tracing::error!(plugin = %self.plugin_name, "{}", message);
        #[cfg(not(feature = "tracing"))]
        {
            let _ = message;
        }
    }
}

impl Default for LogApi {
    fn default() -> Self {
        Self::new()
    }
}
