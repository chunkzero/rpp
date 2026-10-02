//! Crate-wide error type.

use thiserror::Error;

/// Result type alias used throughout this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors produced while packing or unpacking an archive.
#[derive(Debug, Error)]
pub enum Error {
    /// The archive holds more entries than installs accept.
    #[error("the archive has more than {max} entries, the most installs accept")]
    TooManyEntries {
        /// The entry limit.
        max: usize,
    },

    /// One file is larger than installs accept.
    #[error("`{path}` is {size} bytes; installs accept files of at most {max} bytes")]
    FileTooLarge {
        /// The file's path inside the archive.
        path: String,
        /// The file's size in bytes.
        size: u64,
        /// The per-file limit in bytes.
        max: u64,
    },

    /// The files add up to more bytes than installs accept.
    #[error("the archive unpacks to more than {max} bytes, the most installs accept")]
    TooLarge {
        /// The total size limit in bytes.
        max: u64,
    },

    /// A path is absolute or escapes the archive root. Packing also rejects empty, `.`
    /// and `..` segments and names Windows cannot extract.
    #[error("`{0}` is not a portable relative path inside the archive")]
    UnsafePath(String),

    /// An entry is neither a regular file nor a directory, such as a symlink.
    #[error("`{0}` is not a regular file or directory")]
    UnsupportedEntry(String),

    /// An I/O error occurred.
    #[error("{context}: {source}")]
    Io {
        /// What was being attempted when the error occurred.
        context: String,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
}

impl Error {
    pub(crate) fn io(context: impl Into<String>) -> impl FnOnce(std::io::Error) -> Self {
        let context = context.into();
        move |source| Error::Io { context, source }
    }
}
