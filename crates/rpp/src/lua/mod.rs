pub mod core;
pub mod plugin;
pub mod processor;

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
