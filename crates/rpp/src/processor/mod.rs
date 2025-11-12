use crate::processor::filter::Filter;

pub mod filter;
mod manager;

pub struct RawFile<'a>(&'a [u8]);

#[derive(Debug)]
pub struct ProcessorType {
    pub id: String,
    pub filter: Filter,
}
