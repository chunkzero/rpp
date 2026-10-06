//! Resource pack optimization ("squash") for [rpp](https://github.com/rpp/rpp).
//!
//! See `docs/SPEC.md` section 8 for the contract. This is a standalone,
//! blocking crate with no dependency on `rpp`. It provides:
//!
//! - [`squash_zip`] — strip globs, optimize files in memory (rayon), and write a
//!   deterministic, byte-reproducible release zip atomically.
//! - [`zip_to_vec`] — the same deterministic layout, unoptimized, built in memory.
//! - [`run_packsquash`] — delegate to an external PackSquash binary.
//!
//! Recoverable per-file problems (invalid JSON, PNG optimization failures) are
//! never returned as errors: the file is passed through unchanged and a warning
//! is collected into [`SquashReport::warnings`].
//!
//! # Example
//! ```no_run
//! use std::path::Path;
//! use rpp_squash::{squash_zip, PngLevel, SquashOptions};
//!
//! let opts = SquashOptions {
//!     png: PngLevel::Fast,
//!     strip: vec!["**/.DS_Store".into()],
//!     ..Default::default()
//! };
//!
//! let report = squash_zip(Path::new("dist"), Path::new("pack.zip"), &opts)?;
//! println!("optimized {} files", report.files_optimized);
//! # Ok::<(), rpp_squash::Error>(())
//! ```

mod error;
mod file;
mod optimize;
mod options;
mod packsquash;
mod walk;
mod zip;

pub use error::{Error, Result};
pub use optimize::SquashReport;
pub use options::{PngLevel, SquashOptions};
pub use packsquash::run_packsquash;
pub use zip::{squash_zip, zip_to_vec};
