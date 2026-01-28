use sha2::{Digest, Sha256};
use twox_hash::XxHash3_64;

/// Hashing API for plugins.
pub struct HashApi;

impl HashApi {
    pub fn new() -> Self {
        Self
    }

    /// Fast non-cryptographic hash (xxhash3_64).
    pub fn xxhash3(&self, data: &[u8]) -> u64 {
        XxHash3_64::oneshot(data)
    }

    /// SHA-256 cryptographic hash (lowercase hex).
    pub fn sha256(&self, data: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data);
        format!("{:x}", hasher.finalize())
    }

    /// MD5 hash (lowercase hex). For compatibility only.
    pub fn md5(&self, data: &[u8]) -> String {
        let digest = md5::compute(data);
        format!("{:x}", digest)
    }
}

impl Default for HashApi {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xxhash3() {
        let api = HashApi::new();
        assert_eq!(api.xxhash3(b"hello"), api.xxhash3(b"hello"));
        assert_ne!(api.xxhash3(b"hello"), api.xxhash3(b"world"));
    }

    #[test]
    fn test_sha256() {
        let api = HashApi::new();
        assert_eq!(
            api.sha256(b"hello"),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }
}
