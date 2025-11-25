use std::collections::{HashMap, HashSet};

use bincode::{Decode, Encode};

#[derive(Debug, Encode, Decode, Default)]
pub struct Cache {
    pub event_handlers: HashSet<String>,
    pub sources: HashMap<String, SourceState>,
}

#[derive(Debug, Encode, Decode)]
pub struct SourceState {
    pub fingerprint: Fingerprint,
    pub dependencies: Vec<String>,
    pub outputs: Vec<String>,
}

#[derive(Debug, Encode, Decode, Clone, Copy)]
pub struct Fingerprint {
    pub mtime: u64,
    pub size: u64,
    pub hash: u64,
}
