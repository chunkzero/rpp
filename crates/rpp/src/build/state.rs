use std::collections::HashMap;

use bincode::{Decode, Encode};

#[derive(Debug, Encode, Decode)]
pub struct PackState {
    pub inputs: HashMap<String, InputState>,
    pub outputs: HashMap<String, OutputState>,
}

#[derive(Debug, Encode, Decode)]
pub struct InputState {
    pub hash: String,
    pub dependencies: Vec<String>,
    pub outputs: Vec<String>,
}

#[derive(Debug, Encode, Decode, Hash)]
pub struct OutputState {
    pub source: String,
    pub hash: String,
}
