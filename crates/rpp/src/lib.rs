pub mod compile;

mod rpp;
pub(crate) mod util;

#[cfg(feature = "lua")]
pub mod lua;
#[cfg(feature = "lua")]
pub use mlua;

pub use rpp::*;

#[cfg(feature = "lua")]
use crate::lua::plugin::PluginError;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Compile(#[from] compile::CompileError),

    #[cfg(feature = "lua")]
    #[error(transparent)]
    Lua(#[from] mlua::Error),

    #[cfg(feature = "lua")]
    #[error(transparent)]
    Plugin(#[from] PluginError),

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

    #[error("Error creating FileMeta: {0}")]
    FileMeta(String),

    #[error("{0}")]
    Custom(String),
}

unsafe impl Send for Error {}
unsafe impl Sync for Error {}

pub type Result<T> = std::result::Result<T, Error>;
