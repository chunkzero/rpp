use std::collections::HashMap;

use crate::processor::ProcessorType;

#[derive(Debug)]
pub struct ProcessorManager {
    type_registry: HashMap<String, ProcessorType>,
}

impl ProcessorManager {
    pub fn register_processor_type(&mut self, processor_type: ProcessorType) {
        self.type_registry
            .insert(processor_type.id.clone(), processor_type);
    }

    pub fn get_processor_type(&self, id: &str) -> Option<&ProcessorType> {
        self.type_registry.get(id)
    }
}
