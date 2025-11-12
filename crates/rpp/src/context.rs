use std::{borrow::Cow, rc::Rc};

use crate::{pack::Pack, processor::ProcessorType};

#[derive(Debug)]
pub struct BuildContext<'a> {
    pub pack: Pack,

    processors: &'a [ProcessorType],
}

impl<'a> BuildContext<'a> {
    pub fn new(pack: Pack) -> Self {
        Self {
            pack,
            processors: &[],
        }
    }

    pub fn process_file() {}

    pub fn use_file(source: String) {}
}
