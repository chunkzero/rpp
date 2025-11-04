use include_directory::{include_directory, Dir};
use std::{collections::HashMap, path::PathBuf};

pub mod plugin;

pub use mlua;

use crate::lua::plugin::PluginEnvironment;

pub static LUA_API: Dir<'_> = include_directory!("$CARGO_MANIFEST_DIR/src/lua/api");
pub static RPP_PLUGIN: Dir<'_> = include_directory!("$CARGO_MANIFEST_DIR/src/lua/rpp");

#[derive(Debug)]
pub struct RppLua {
    lua: mlua::Lua,
    plugins: HashMap<String, PluginEnvironment>,
}

impl RppLua {
    pub fn new() -> Self {
        let lua = mlua::Lua::new();
        Self {
            lua,
            plugins: Default::default(),
        }
    }

    pub fn load_plugin(&mut self, dir: impl Into<PathBuf>) -> crate::Result<PluginEnvironment> {
        let env = PluginEnvironment::new(self.lua.clone(), dir)?;

        Ok(env)
    }
}
