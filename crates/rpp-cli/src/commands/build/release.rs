//! Reuse of an unchanged builtin release archive across builds.
//!
//! After writing the archive, the build records its inputs (builtin squash settings, rpp
//! version, and the engine's digest of the pack output) together with the archive's size
//! and xxh3. The record lives in the project cache, so `--no-cache` and
//! `rpp clean` drop it. The record is only an optimization: a missing or unwritable record
//! means the archive is rewritten.

use std::hash::Hasher;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rpp::config::PngSetting;
use rpp::engine::BuildResult;
use serde::{Deserialize, Serialize};
use twox_hash::XxHash3_64;

use crate::atomic;
use crate::project::Project;

/// Bumped whenever the record layout or the meaning of its inputs changes.
const RECORD_FORMAT: u32 = 2;

/// Everything the release archive bytes depend on.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct ReleaseInputs {
    format: u32,
    rpp: String,
    json: bool,
    png: PngSetting,
    strip: Vec<String>,
    /// The engine's digest of every output path and its contents, which the output
    /// directory holds after the build.
    output: u64,
}

#[derive(Serialize, Deserialize)]
struct ReleaseRecord {
    inputs: ReleaseInputs,
    /// Size and xxh3 of the archive written for `inputs`.
    zip: (u64, u64),
}

impl ReleaseInputs {
    /// The inputs of the archive for the output `result` produced.
    pub(super) fn current(project: &Project, result: &BuildResult) -> Self {
        let squash = &project.config.build.squash;
        Self {
            format: RECORD_FORMAT,
            rpp: env!("CARGO_PKG_VERSION").to_owned(),
            json: squash.json,
            png: squash.png,
            strip: squash.strip.clone(),
            output: result.output_digest,
        }
    }
}

/// Whether the archive at `zip_path` was written for `inputs` and is unmodified since.
pub(super) fn is_current(project: &Project, inputs: &ReleaseInputs, zip_path: &Path) -> bool {
    let Ok(bytes) = std::fs::read(record_path(project)) else {
        return false;
    };
    serde_json::from_slice::<ReleaseRecord>(&bytes).is_ok_and(|record| {
        record.inputs == *inputs && digest(zip_path).is_ok_and(|zip| zip == record.zip)
    })
}

/// Record that the archive at `zip_path` was just written for `inputs`.
pub(super) fn remember(project: &Project, inputs: ReleaseInputs, zip_path: &Path) -> Result<()> {
    let zip = digest(zip_path).with_context(|| format!("hashing {}", zip_path.display()))?;
    let record = serde_json::to_vec(&ReleaseRecord { inputs, zip })?;
    atomic::write(&record_path(project), record)
}

/// The project cache directory, which `--no-cache` removes.
pub(super) fn cache_dir(project: &Project) -> PathBuf {
    project.root.join(".rpp").join("cache")
}

fn record_path(project: &Project) -> PathBuf {
    cache_dir(project).join("release.json")
}

/// Size and xxh3 of the file at `path`.
fn digest(path: &Path) -> std::io::Result<(u64, u64)> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = XxHash3_64::default();
    let mut buffer = vec![0; 1 << 16];
    let mut len = 0;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            return Ok((len, hasher.finish()));
        }
        hasher.write(&buffer[..read]);
        len += read as u64;
    }
}
