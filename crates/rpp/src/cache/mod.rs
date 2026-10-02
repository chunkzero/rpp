//! Incremental compilation cache (spec §7): manifest + content-addressed store.

pub(crate) mod manifest;
pub(crate) mod store;

pub(crate) use manifest::{
    FileEntry, Fingerprint, GeneratorEntry, GeneratorMutation, Manifest, OutputRef, ReadKind,
    ReadRecord,
};
pub(crate) use store::ObjectStore;
