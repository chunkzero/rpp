use crate::compile::processor::Processor;

pub mod context;
pub mod processor;
pub mod state;

pub struct PackCompiler {
    pack:
    processors: Vec<Box<dyn Processor>>,
}

impl<'a> PackCompiler<'a> {
    pub fn new()
}
