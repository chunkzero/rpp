use glob::Pattern;
use std::path::Path;
use std::sync::Arc;

use super::{GeneratorPlugin, ProcessorPlugin};

/// Registry of all loaded plugins.
pub struct PluginRegistry {
    processors: Vec<ProcessorEntry>,
    generators: Vec<Arc<dyn GeneratorPlugin>>,
}

struct ProcessorEntry {
    plugin: Arc<dyn ProcessorPlugin>,
    patterns: Vec<Pattern>,
    priority: i32,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            processors: Vec::new(),
            generators: Vec::new(),
        }
    }

    /// Register a processor plugin.
    pub fn register_processor(&mut self, plugin: Arc<dyn ProcessorPlugin>) {
        let patterns: Vec<Pattern> = plugin
            .patterns()
            .iter()
            .filter_map(|p| Pattern::new(p).ok())
            .collect();

        let priority = plugin.priority();

        self.processors.push(ProcessorEntry {
            plugin,
            patterns,
            priority,
        });

        // Keep sorted by priority (ascending - lower runs first)
        self.processors.sort_by_key(|e| e.priority);
    }

    /// Register a generator plugin.
    pub fn register_generator(&mut self, plugin: Arc<dyn GeneratorPlugin>) {
        self.generators.push(plugin);
    }

    /// Get processors that match a file path, in priority order.
    pub fn processors_for_file(&self, path: &Path) -> Vec<Arc<dyn ProcessorPlugin>> {
        let path_str = path.to_string_lossy();

        self.processors
            .iter()
            .filter(|entry| entry.patterns.iter().any(|p| p.matches(&path_str)))
            .map(|entry| Arc::clone(&entry.plugin))
            .collect()
    }

    /// Get all registered generators.
    pub fn generators(&self) -> &[Arc<dyn GeneratorPlugin>] {
        &self.generators
    }

    /// Get all registered processors.
    pub fn all_processors(&self) -> Vec<Arc<dyn ProcessorPlugin>> {
        self.processors
            .iter()
            .map(|e| Arc::clone(&e.plugin))
            .collect()
    }

    /// Get version string for a plugin by name (for cache invalidation).
    pub fn plugin_version(&self, name: &str) -> Option<&str> {
        for entry in &self.processors {
            if entry.plugin.name() == name {
                return Some(entry.plugin.version());
            }
        }
        for gen in &self.generators {
            if gen.name() == name {
                return Some(gen.version());
            }
        }
        None
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::BuildError;
    use crate::plugin::{Plugin, ProcessResult, ProcessingContext, ProcessorPlugin};

    struct TestProcessor {
        name: String,
        patterns: Vec<String>,
        priority: i32,
    }

    impl Plugin for TestProcessor {
        fn name(&self) -> &str {
            &self.name
        }
        fn version(&self) -> &str {
            "1.0.0"
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    impl ProcessorPlugin for TestProcessor {
        fn patterns(&self) -> &[String] {
            &self.patterns
        }
        fn priority(&self) -> i32 {
            self.priority
        }
        fn process(&self, _ctx: &ProcessingContext) -> Result<ProcessResult, BuildError> {
            Ok(ProcessResult::Skip)
        }
    }

    #[test]
    fn test_processor_matching() {
        let mut registry = PluginRegistry::new();

        registry.register_processor(Arc::new(TestProcessor {
            name: "json".into(),
            patterns: vec!["*.json".into()],
            priority: 100,
        }));

        registry.register_processor(Arc::new(TestProcessor {
            name: "png".into(),
            patterns: vec!["*.png".into(), "**/*.png".into()],
            priority: 50,
        }));

        let json_procs = registry.processors_for_file(Path::new("test.json"));
        assert_eq!(json_procs.len(), 1);
        assert_eq!(json_procs[0].name(), "json");

        let png_procs = registry.processors_for_file(Path::new("assets/item.png"));
        assert_eq!(png_procs.len(), 1);
        assert_eq!(png_procs[0].name(), "png");

        let txt_procs = registry.processors_for_file(Path::new("readme.txt"));
        assert_eq!(txt_procs.len(), 0);
    }

    #[test]
    fn test_priority_ordering() {
        let mut registry = PluginRegistry::new();

        // Register in reverse priority order
        registry.register_processor(Arc::new(TestProcessor {
            name: "second".into(),
            patterns: vec!["*".into()],
            priority: 200,
        }));

        registry.register_processor(Arc::new(TestProcessor {
            name: "first".into(),
            patterns: vec!["*".into()],
            priority: 100,
        }));

        let procs = registry.processors_for_file(Path::new("test.txt"));
        assert_eq!(procs[0].name(), "first"); // priority 100
        assert_eq!(procs[1].name(), "second"); // priority 200
    }

    #[test]
    fn test_plugin_version_lookup() {
        let mut registry = PluginRegistry::new();

        registry.register_processor(Arc::new(TestProcessor {
            name: "json".into(),
            patterns: vec!["*.json".into()],
            priority: 100,
        }));

        assert_eq!(registry.plugin_version("json"), Some("1.0.0"));
        assert_eq!(registry.plugin_version("unknown"), None);
    }
}
