pub mod build;
pub mod pack;
pub mod resources;
pub(crate) mod util;

#[cfg(feature = "lua")]
pub mod lua;
#[cfg(feature = "lua")]
pub use mlua;

use std::collections::HashMap;

use crate::{build::processor::Processor, pack::Pack};

pub struct ResourcePackProcessor {
    packs: HashMap<String, Pack>,
    processors: HashMap<String, Box<dyn Processor>>,
}

impl ResourcePackProcessor {}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Lua(#[from] mlua::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Regex(#[from] regex::Error),
    #[error(transparent)]
    Ignore(#[from] ignore::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Jsonc(#[from] jsonc_parser::errors::ParseError),
    #[error(transparent)]
    TomlSer(#[from] toml::ser::Error),
    #[error(transparent)]
    TomlDe(#[from] toml::de::Error),

    #[error("Invalid version string")]
    InvalidVersion,

    #[cfg(feature = "lua")]
    #[error("Plugin error: {0}")]
    Plugin(String),

    #[error("Error processing: {0}")]
    Process(String),

    #[error("Error creating FileMeta: {0}")]
    FileMeta(String),

    #[error("{0}")]
    Custom(String),
}

unsafe impl Send for Error {}
unsafe impl Sync for Error {}

pub type Result<T> = std::result::Result<T, Error>;
