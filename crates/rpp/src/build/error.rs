use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BuildError {
    #[error("Discovery error: {0}")]
    Discovery(String),

    #[error("Failed to read file {path}: {source}")]
    FileRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to write file {path}: {source}")]
    FileWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Processor '{processor}' failed on {path}: {message}")]
    ProcessorFailed {
        processor: String,
        path: PathBuf,
        message: String,
    },

    #[error("Generator '{generator}' failed: {message}")]
    GeneratorFailed { generator: String, message: String },

    #[error("Cache error: {0}")]
    Cache(String),

    #[error("Worker error: {0}")]
    Worker(String),

    #[error("Plugin error: {0}")]
    Plugin(String),

    #[error("Cannot modify registry after it has been shared")]
    RegistryLocked,
}
