//! Downloading, verifying and extracting release archives into the cache.
//!
//! Archives use the `rpp-archive` format and include `rpp.json`. They extract to
//! `<cache_root>/<name>/<version>-<sha256>/` through a sibling temp directory that is
//! renamed into place only once the package checks out.

use std::fs;
use std::path::{Path, PathBuf};

use semver::Version;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use crate::error::{Error, Result};
use crate::http::{RegistryClient, MAX_ARCHIVE_BYTES};

use super::{read_package_summary, validate_name};

pub(super) fn install(
    client: &RegistryClient,
    cache_root: &Path,
    name: &str,
    version: &Version,
    url: &str,
    sha256: &str,
) -> Result<PathBuf> {
    validate_name(name)?;
    let expected = sha256.to_ascii_lowercase();
    if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::Registry {
            url: url.to_string(),
            reason: format!("invalid sha256 `{sha256}`"),
        });
    }
    let dir = cache_root.join(name).join(format!("{version}-{expected}"));
    if dir.exists() {
        return Ok(dir);
    }

    let bytes = client
        .get_bytes(url, MAX_ARCHIVE_BYTES)
        .map_err(|f| f.into_error(url))?;
    let actual = format!("{:x}", Sha256::digest(&bytes));
    if actual != expected {
        return Err(Error::HashMismatch {
            name: name.to_string(),
            version: version.clone(),
            expected,
            actual,
        });
    }

    let unpacked = unpack(&bytes, &dir, name, version)?;
    publish(unpacked, &dir)?;
    Ok(dir)
}

/// Unpack `bytes` into a temp directory beside `dir` and check that its `rpp.json`
/// declares `name` and `version`. The temp directory is removed when dropped.
fn unpack(bytes: &[u8], dir: &Path, name: &str, version: &Version) -> Result<TempDir> {
    let parent = dir
        .parent()
        .expect("cache entries are nested in the cache root");
    fs::create_dir_all(parent)
        .map_err(|e| Error::io(format!("creating cache dir {}", parent.display()), e))?;
    let unpacked = tempfile::Builder::new()
        .prefix(".tmp-")
        .tempdir_in(parent)
        .map_err(|e| Error::io(format!("creating temp dir under {}", parent.display()), e))?;
    rpp_archive::unpack(bytes, unpacked.path()).map_err(|source| Error::Archive {
        name: name.to_string(),
        version: version.clone(),
        source,
    })?;
    let found = read_package_summary(unpacked.path())?;
    if found.name != name || found.version != *version {
        return Err(Error::PackageMismatch {
            path: dir.to_path_buf(),
            expected: format!("{name} {version}"),
            found: format!("{} {}", found.name, found.version),
        });
    }
    Ok(unpacked)
}

/// Rename `unpacked` to `dir`. Another process publishing `dir` first is not an error;
/// the redundant temp directory is then removed.
fn publish(unpacked: TempDir, dir: &Path) -> Result<()> {
    match fs::rename(unpacked.path(), dir) {
        Ok(()) => {
            let _ = unpacked.keep();
            Ok(())
        }
        Err(_) if dir.exists() => Ok(()),
        Err(e) => Err(Error::io(
            format!("publishing cache entry {}", dir.display()),
            e,
        )),
    }
}
