use std::path::Path;

use serde::{Deserialize, Serialize};

pub mod environment;

pub struct Plugin {
    lua: mlua::WeakLua,
    env: mlua::Table,
}

impl Plugin {
    pub fn new(lua: &mlua::Lua, env: mlua::Table) -> Self {
        Self {
            lua: lua.weak(),
            env,
        }
    }

    pub fn from_dir(path: &Path) -> crate::Result<Self> {
        let config = std::fs::read_to_string(path.join("plugin.toml"))?;

        let config = toml::from_str::<PluginConfig>(&config)?;

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
