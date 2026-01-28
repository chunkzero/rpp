use super::Plugin;
use crate::build::{BuildError, GeneratedFile, ProcessedFile};
use std::path::Path;

/// Context passed to generator plugins.
pub struct GeneratorContext<'a> {
    /// All files that have been processed (available for inspection)
    pub processed_files: &'a [ProcessedFile],
    /// Plugin configuration from rpp.toml
    pub config: &'a toml::Value,
    /// Output directory root
    pub output_dir: &'a Path,
}

/// Trait for generator plugins that create new files.
pub trait GeneratorPlugin: Plugin {
    /// Generate new files based on processed content.
    /// Called after all processors have run.
    fn generate(&self, ctx: &GeneratorContext) -> Result<Vec<GeneratedFile>, BuildError>;
}
