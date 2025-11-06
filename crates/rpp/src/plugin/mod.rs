mod environment;
mod manager;

use std::path::PathBuf;

pub use manager::PluginManager;

use mlua::Table;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::plugin::{environment::create_plugin_environment, manager::Globals};

#[derive(Debug, Serialize, Deserialize)]
pub struct PluginConfig {
    id: String,
    version: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    include: Vec<String>,
}

#[derive(Debug)]
pub struct LoadedPlugin {
    lua: mlua::Lua,
    env: Table,
    pub id: String,
    pub version: String,
    pub description: String,
}

pub static ID_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r#"^[a-zA-Z0-9_-]+$"#).unwrap());

pub static SEMVER_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*)(?:\.(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*))*))?(?:\+([0-9a-zA-Z-]+(?:\.[0-9a-zA-Z-]+)*))?$"#).unwrap()
});

impl LoadedPlugin {
    fn new(
        lua: mlua::Lua,
        id: String,
        version: String,
        description: String,
        cpath: &str,
        path: &str,
        globals: &Globals,
    ) -> crate::Result<Self> {
        if !SEMVER_REGEX.is_match(&version) {
            return Err(crate::Error::InvalidVersion);
        };

        let env = create_plugin_environment(&lua, globals, cpath, path)?;

        Ok(Self {
            lua,
            env,
            id,
            version,
            description,
        })
    }

    fn init(&self, path: impl Into<PathBuf>) -> crate::Result<()> {
        self.lua
            .load(path.into())
            .set_name(&self.id)
            .set_environment(self.env.clone())
            .exec()?;

        Ok(())
    }
}
