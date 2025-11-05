pub mod build;
pub mod lua;
pub mod pack;
pub mod plugin;

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

    #[error("Invalid version string.")]
    InvalidVersion,

    #[error("Plugin error: {0}")]
    PluginError(String),
}

unsafe impl Send for Error {}
unsafe impl Sync for Error {}

pub type Result<T> = std::result::Result<T, Error>;
