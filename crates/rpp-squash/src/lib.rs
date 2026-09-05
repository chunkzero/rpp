//! Resource pack optimization ("squash") for [rpp](https://github.com/rpp/rpp).
//!
//! See `docs/SPEC.md` section 8 for the contract. This is a standalone,
//! blocking crate with no dependency on `rpp`. It provides:
//!
//! - [`squash_file`] — pure single-file optimization (JSON minify, PNG optimize).
//! - [`squash_dir`] — walk a directory, strip globs, optimize in place (rayon).
//! - [`write_zip`] — deterministic, byte-reproducible zip of a directory.
//! - [`run_packsquash`] — delegate to an external PackSquash binary.
//!
//! # Warnings
//! Recoverable per-file problems (invalid JSON, PNG optimization failures) are
//! never returned as errors: the file is passed through unchanged and a warning
//! is surfaced. [`squash_dir`] collects warnings into [`SquashReport::warnings`];
//! [`squash_file`] emits them via `tracing::warn!` when the `tracing` feature is
//! enabled (on by default).
//!
//! # Example
//! ```no_run
//! use std::path::Path;
//! use rpp_squash::{squash_dir, write_zip, PngLevel, SquashOptions, ZipOptions};
//!
//! let opts = SquashOptions::builder()
//!     .json(true)
//!     .png(PngLevel::Fast)
//!     .strip_pattern("**/.DS_Store")
//!     .build();
//!
//! let report = squash_dir(Path::new("dist"), &opts)?;
//! println!("{report}");
//! write_zip(Path::new("dist"), Path::new("dist/pack.zip"), &ZipOptions::default())?;
//! # Ok::<(), rpp_squash::Error>(())
//! ```

mod dir;
mod error;
mod file;
mod options;
mod packsquash;
mod stage;
mod zip;

pub use dir::{squash_dir, FileDetail, SquashReport};
pub use error::{Error, Result};
pub use file::squash_file;
pub use options::{PngLevel, SquashOptions, SquashOptionsBuilder, ZipOptions};
pub use packsquash::{render_options, run_packsquash};
pub use stage::copy_tree;
pub use zip::write_zip;
