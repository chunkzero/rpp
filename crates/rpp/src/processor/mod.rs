use crate::processor::filter::Filter;

pub mod filter;
mod manager;

pub use manager::ProcessorManager;

#[derive(Debug)]
pub struct ProcessorType {
    pub id: String,
    pub filter: Filter,
}
