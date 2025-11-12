pub mod context;
pub mod file;
pub mod pack;
pub mod plugin;
pub mod processor;
pub mod resources;

pub use mlua;

#[derive(Debug)]
pub struct Rpp {
    pub lua: mlua::Lua,
    pub plugin_manager: plugin::PluginManager,
}

impl Rpp {
    pub fn new() -> Result<Self> {
        let lua = mlua::Lua::new();

        let plugin_manager = plugin::PluginManager::new(lua.clone());

        Ok(Rpp {
            lua,
            plugin_manager,
        })
    }
}

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

    #[error("Invalid version string.")]
    InvalidVersion,

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
