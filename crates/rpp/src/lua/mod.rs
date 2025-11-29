pub mod core;
mod error;
pub mod handler;
pub mod plugin;

pub use error::PluginError;

#[derive(Debug)]
pub struct RppLua {
    lua: mlua::Lua,
}

impl RppLua {
    pub fn new() -> Self {
        Self::with_lua(mlua::Lua::new())
    }

    pub fn with_lua(lua: mlua::Lua) -> Self {
        Self { lua }
    }
}
