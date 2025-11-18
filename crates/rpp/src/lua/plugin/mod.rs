use std::path::Path;

use serde::{Deserialize, Serialize};

pub mod environment;
pub mod api;

pub struct PluginLoader {
    lua: mlua::Lua,
    env: mlua::Table,
}

impl PluginLoader {
    pub fn new(lua: &mlua::Lua, env: mlua::Table) -> Self {
        Self {
            lua: lua.weak(),
            env,
        }
    }

    pub fn from_dir(lua: &mlua::Lua, path: &Path) -> crate::Result<Self> {
        let config = std::fs::read_to_string(path.join("plugin.toml"))?;

        let config = toml::from_str::<PluginConfig>(&config)?;


        Self {}

        todo!()
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PluginConfig {
    pub id: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub include: Vec<String>,
}
