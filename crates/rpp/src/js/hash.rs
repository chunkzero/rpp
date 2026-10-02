//! Hex and checksum helpers exposed to plugins.

use sha2::{Digest, Sha256};

use crate::util::hash::{u64_hex, xxh3};

/// Render bytes as a lowercase hex string.
fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// XXH3-64 of `bytes` as 16 lowercase hex digits.
pub(super) fn xxh3_hex(bytes: &[u8]) -> String {
    u64_hex(xxh3(bytes))
}

/// SHA-256 of `bytes` as lowercase hex.
pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    to_hex(&Sha256::digest(bytes))
}

/// MD5 of `bytes` as lowercase hex.
pub(super) fn md5_hex(bytes: &[u8]) -> String {
    to_hex(&md5::compute(bytes).0)
}

/// CRC-32 (IEEE) of `bytes`.
pub(super) fn crc32(bytes: &[u8]) -> u32 {
    crc32fast::hash(bytes)
}
