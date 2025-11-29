use std::{error::Error, path::PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("Failed to read plugin config {path}: {source}")]
    ConfigRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to parse plugin config {path}: {source}")]
    ConfigParse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("Failed to canonicalize plugin path {path}: {source}")]
    Canonicalize {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Plugin path {path} is not valid UTF-8")]
    PathInvalidUtf8 { path: PathBuf },

    #[error("Failed to load plugin from {path}: {source}")]
    Load {
        path: PathBuf,
        #[source]
        source: Box<dyn Error>,
    },
}
