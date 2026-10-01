//! Downloading, verifying and extracting release archives into the cache.
//!
//! Archives are gzipped tars whose entries sit at the archive root (no wrapper
//! directory) and include `rpp.json`. They extract to
//! `<cache_root>/<name>/<version>-<first 16 hex chars of sha256>/`.

use std::path::{Path, PathBuf};

use semver::Version;
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::extract::extract_archive;
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
    let Some(prefix) = expected
        .get(..16)
        .filter(|p| p.bytes().all(|b| b.is_ascii_hexdigit()))
    else {
        return Err(Error::Registry {
            url: url.to_string(),
            reason: format!("invalid sha256 `{sha256}`"),
        });
    };
    let dir = cache_root.join(name).join(format!("{version}-{prefix}"));
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

    extract_archive(&bytes, &dir, false, |unpacked| {
        let found = read_package_summary(unpacked)?;
        if found.name == name && found.version == *version {
            return Ok(());
        }
        Err(Error::PackageMismatch {
            path: dir.clone(),
            expected: format!("{name} {version}"),
            found: format!("{} {}", found.name, found.version),
        })
    })?;
    Ok(dir)
}
