use std::collections::HashMap;

use crate::processor::ProcessorType;

pub struct ProcessorManager {
    type_registry: HashMap<String, ProcessorType>,
}

impl ProcessorManager {}
