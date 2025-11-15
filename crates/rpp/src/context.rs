use crate::{pack::Pack, processor::ProcessorManager};

#[derive(Debug)]
pub struct BuildContext<'a> {
    pub pack: Pack,

    manager: &'a ProcessorManager,
}

impl<'a> BuildContext<'a> {
    pub fn new(pack: Pack, manager: &'a ProcessorManager) -> Self {
        Self { pack, manager }
    }

    pub fn process_file() {}

    pub fn use_file(source: String) {}
}
