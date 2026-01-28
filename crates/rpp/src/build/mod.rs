//! Build pipeline types and orchestration.

mod cache;
mod discovery;
mod engine;
mod error;
mod finalize;
mod output;
mod process;
mod types;

pub use cache::BuildCache;
pub use discovery::DiscoveryPhase;
pub use engine::{BuildEngine, BuildEngineBuilder};
pub use error::BuildError;
pub use finalize::FinalizePhase;
pub use output::OutputWriter;
pub use process::{ProcessPhase, ProcessPhaseResult};
pub use types::{
    BuildResult, CachedEntry, FileEntry, FileIndex, Fingerprint, GeneratedFile, ProcessedFile,
    Transformation,
};
