//! Reuse of an unchanged builtin release archive across builds.
//!
//! After writing the archive, the build records its inputs (squash settings, rpp version,
//! archive path, and the cache manifest that describes the output) together with the
//! archive's size and xxh3. The record lives in the project cache, so `--no-cache` and
//! `rpp clean` drop it.

use std::hash::Hasher;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use twox_hash::XxHash3_64;

use crate::atomic;
use crate::project::Project;

/// Bumped whenever the record layout or the meaning of its inputs changes.
const RECORD_FORMAT: u32 = 1;

/// Everything the release archive bytes depend on, besides the output files themselves.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct ReleaseInputs {
    format: u32,
    rpp: String,
    squash: serde_json::Value,
    zip_path: PathBuf,
    /// xxh3 of the cache manifest, which determines every output path and its contents.
    manifest: u64,
}

#[derive(Serialize, Deserialize)]
struct ReleaseRecord {
    inputs: ReleaseInputs,
    /// Size and xxh3 of the archive written for `inputs`.
    zip: (u64, u64),
}

impl ReleaseInputs {
    /// The inputs of this build's archive, or `None` when the cache manifest is unreadable.
    pub(super) fn current(project: &Project) -> Result<Option<Self>> {
        let Ok(manifest) = std::fs::read(cache_dir(project).join("manifest.bin")) else {
            return Ok(None);
        };
        Ok(Some(Self {
            format: RECORD_FORMAT,
            rpp: env!("CARGO_PKG_VERSION").to_owned(),
            squash: serde_json::to_value(&project.config.build.squash)?,
            zip_path: project.release_zip(),
            manifest: XxHash3_64::oneshot(&manifest),
        }))
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

/// Drop the record before the archive is rewritten, so a failed write cannot leave it stale.
pub(super) fn forget(project: &Project) -> Result<()> {
    let path = record_path(project);
    match std::fs::remove_file(&path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(error).with_context(|| format!("removing {}", path.display()))
        }
        _ => Ok(()),
    }
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
