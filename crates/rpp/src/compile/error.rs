use std::path::PathBuf;

use thiserror::Error;

use crate::compile::worker;

#[derive(Debug, Error)]
pub enum CompileError {
    #[error("Failed to read cache file {path}: {source}")]
    CacheRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Error in EventHandler ({id}): {source}")]
    EventHandler {
        id: String,
        #[source]
        source: Box<dyn std::error::Error>,
    },

    #[error("Failed to decode cache state from {path}: {source}")]
    CacheDecode {
        path: PathBuf,
        #[source]
        source: bincode::error::DecodeError,
    },

    #[error("Failed to walk pack entry {path:?}: {source}")]
    Walk {
        path: Option<PathBuf>,
        #[source]
        source: ignore::Error,
    },

    #[error("Failed to read metadata for {path}: {source}")]
    Metadata {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to submit job to worker pool: {source}")]
    Submit {
        #[source]
        source: crossbeam_channel::SendError<Option<worker::Job>>,
    },

    #[error("Failed to receive worker result: {source}")]
    Receive {
        #[source]
        source: crossbeam_channel::RecvError,
    },
}
