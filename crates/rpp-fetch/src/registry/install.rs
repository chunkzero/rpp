//! Downloading, verifying and extracting release archives into the cache.
//!
//! Archives are gzipped tars whose entries sit at the archive root (no wrapper
//! directory) and include `rpp.json`. They extract to
//! `<cache_root>/<name>/<version>-<first 16 hex chars of sha256>/`.
