//! Hashing helpers used by the cache, and plugins.

use twox_hash::XxHash3_64;

/// Compute an xxh3-64 hash of a byte slice.
pub(crate) fn xxh3(bytes: &[u8]) -> u64 {
    XxHash3_64::oneshot(bytes)
}

/// Render bytes as a lowercase hex string.
pub(crate) fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap_or('0'));
        out.push(char::from_digit((b & 0x0f) as u32, 16).unwrap_or('0'));
    }
    out
}

/// Render a `u64` as a fixed-width 16-character lowercase hex string.
pub(crate) fn u64_hex(value: u64) -> String {
    format!("{value:016x}")
}

/// A streaming xxh3-64 accumulator built over multiple writes.
///
/// Used to derive composite keys (for example, the plugin `cache_key`) by
/// feeding a deterministic, ordered sequence of byte chunks.
#[derive(Default)]
pub(crate) struct HashWriter {
    buf: Vec<u8>,
}

impl HashWriter {
    /// Create an empty writer.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Append raw bytes, length-prefixed to avoid ambiguity between chunks.
    pub(crate) fn write(&mut self, bytes: &[u8]) {
        self.buf
            .extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        self.buf.extend_from_slice(bytes);
    }

    /// Append a string (length-prefixed).
    pub(crate) fn write_str(&mut self, s: &str) {
        self.write(s.as_bytes());
    }

    /// Append a `u64` value.
    pub(crate) fn write_u64(&mut self, value: u64) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// Append a boolean value.
    pub(crate) fn write_bool(&mut self, value: bool) {
        self.buf.push(u8::from(value));
    }

    /// Finalize and return the xxh3-64 digest of everything written.
    pub(crate) fn finish(&self) -> u64 {
        xxh3(&self.buf)
    }
}
