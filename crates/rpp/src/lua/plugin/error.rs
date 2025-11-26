use thiserror::Error;

#[derive(Debug, Error)]
pub enum PluginError {
    #[error("Failed to load plugin: {id}")]
    Load { id: String },
}
