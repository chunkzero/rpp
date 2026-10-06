//! Immutable last-successful development archive.
use std::path::Path;
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use axum::body::Bytes;
use sha1::{Digest, Sha1};

#[derive(Clone)]
pub(super) struct Pack {
    pub sha1: String,
    pub bytes: Bytes,
}

#[derive(Clone, Default)]
pub(super) struct PackStore(Arc<RwLock<Option<Pack>>>);

impl PackStore {
    pub fn current(&self) -> Option<Pack> {
        self.0.read().unwrap().clone()
    }

    pub fn metadata(&self) -> serde_json::Value {
        self.current().map_or(serde_json::Value::Null, |pack| {
            serde_json::json!({
                "url": format!("/packs/{}.zip", pack.sha1),
                "sha1": pack.sha1,
                "size": pack.bytes.len(),
            })
        })
    }

    /// Archive `output` in memory, leaving out `release_zip` and its staging files, then
    /// swap only after every step succeeds.
    pub fn publish(&self, output: &Path, release_zip: Option<&Path>) -> Result<bool> {
        let bytes =
            rpp_squash::zip_to_vec(output, release_zip).context("creating dev pack archive")?;
        let sha1 = format!("{:x}", Sha1::digest(&bytes));
        let mut current = self.0.write().unwrap();
        if current.as_ref().is_some_and(|pack| pack.sha1 == sha1) {
            return Ok(false);
        }
        *current = Some(Pack {
            sha1,
            bytes: bytes.into(),
        });
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_are_content_addressed_and_survive_publication_failure() {
        let source = tempfile::tempdir().unwrap();
        let file = source.path().join("pack.mcmeta");
        std::fs::write(&file, "{}").unwrap();
        let store = PackStore::default();
        assert!(store.publish(source.path(), None).unwrap());
        let first = store.current().unwrap();
        assert_eq!(first.sha1, format!("{:x}", Sha1::digest(&first.bytes)));
        assert!(!store.publish(source.path(), None).unwrap());
        std::fs::write(file, "changed").unwrap();
        assert!(store.publish(source.path(), None).unwrap());
        let second = store.current().unwrap();
        assert_ne!(first.sha1, second.sha1);
        assert!(store.publish(&source.path().join("missing"), None).is_err());
        assert_eq!(store.current().unwrap().sha1, second.sha1);
    }
}
