//! Resource pack optimization ("squash") for [rpp](https://github.com/rpp/rpp).
//!
//! See `docs/SPEC.md` section 8 for the contract. This is a standalone,
//! blocking crate with no dependency on `rpp`. It provides:
//!
//! - [`squash_dir`] — walk a directory, strip globs, optimize in place (rayon).
//! - [`write_zip`] / [`zip_to_vec`] — deterministic, byte-reproducible zip of a
//!   directory, written atomically to disk or built in memory.
//! - [`copy_tree`] — copy a directory tree for release staging.
//! - [`run_packsquash`] — delegate to an external PackSquash binary.
//!
//! Recoverable per-file problems (invalid JSON, PNG optimization failures) are
//! never returned as errors: the file is passed through unchanged and a warning
//! is collected into [`SquashReport::warnings`].
//!
//! # Example
//! ```no_run
//! use std::path::Path;
//! use rpp_squash::{squash_dir, write_zip, PngLevel, SquashOptions};
//!
//! let opts = SquashOptions {
//!     png: PngLevel::Fast,
//!     strip: vec!["**/.DS_Store".into()],
//!     ..Default::default()
//! };
//!
//! let report = squash_dir(Path::new("dist"), &opts)?;
//! println!("optimized {} files", report.files_optimized);
//! write_zip(Path::new("dist"), Path::new("pack.zip"))?;
//! # Ok::<(), rpp_squash::Error>(())
//! ```

mod dir;
mod error;
mod file;
mod options;
mod packsquash;
mod stage;
mod walk;
mod zip;

pub use dir::{squash_dir, SquashReport};
pub use error::{Error, Result};
pub use options::{PngLevel, SquashOptions};
pub use packsquash::run_packsquash;
pub use stage::copy_tree;
pub use zip::{write_zip, zip_to_vec};
