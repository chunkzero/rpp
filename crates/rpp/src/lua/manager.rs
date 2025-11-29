use std::{collections::HashMap, fs, path::PathBuf, sync::RwLock};

use std::{collections::HashMap, fs, path::PathBuf, sync::RwLock};

use mlua::{Lua, Value};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::plugin::{LoadedPlugin, PluginConfig};
use crate::Error as RppError;

pub(crate) type Globals = HashMap<String, Value>;
type LuaManagerResult<T> = std::result::Result<T, LuaManagerError>;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PluginInfo {
    id: String,
    version: String,
    description: String,
}

#[derive(Debug)]
pub struct PluginManager {
    lua: Lua,
    globals: Globals,

    loaded_plugins: RwLock<HashMap<String, LoadedPlugin>>,
}

impl PluginManager {
    pub fn new(lua: Lua) -> Self {
        let globals = lua
            .globals()
            .pairs::<String, Value>()
            .filter_map(|ele| ele.ok())
            .filter(|(k, _)| {
                !["_G", "package", "coroutine", "require", "module"].contains(&k.as_str())
            })
            .collect();

        Self::new_with_globals(lua, globals)
    }
    // Why is this fn public? Are you going to use it later?
    pub fn new_with_globals(lua: Lua, globals: Globals) -> Self {
        Self {
            lua,
            globals,
            loaded_plugins: Default::default(),
        }
    }

    pub fn info(&self) -> Vec<PluginInfo> {
        self.loaded_plugins
            .read()
            .expect("RwLock was poisoned")
            .iter()
            .map(|(_, plugin)| PluginInfo {
                id: plugin.id.clone(),
                version: plugin.version.clone(),
                description: plugin.description.clone(),
            })
            .collect()
    }
    // Tihs function and the next have variable similar names. I find similar named functions and variables can be hard to read and potentially lead to mistakes.
    pub fn load_plugins(&self, container_dir: impl Into<PathBuf>) -> crate::Result<()> {
        let path = container_dir.into();

        if !path.is_dir() {
            return Err(LuaManagerError::ContainerNotDirectory { path }.into());
        }

        let dir = path
            .read_dir()
            .map_err(|source| LuaManagerError::DirectoryRead {
                path: path.clone(),
                source,
            })?;

        for entry in dir {
            let child = entry.map_err(|source| LuaManagerError::DirectoryEntry {
                path: path.clone(),
                source,
            })?;
            self.load_plugin(child.path()).map_err(Into::into)?;
        }

        Ok(())
    }

    fn load_plugin(&self, plugin_dir: impl Into<PathBuf>) -> LuaManagerResult<()> {
        let path = plugin_dir.into();

        let (source_dir, config_path) = if path.is_dir() {
            let dir = path.clone();
            (dir, dir.join("plugin.toml"))
        } else {
            let parent = path
                .parent()
                .ok_or(LuaManagerError::PluginConfigMissingParent { path: path.clone() })?
                .to_path_buf();
            (parent, path.clone())
        };

        let text = fs::read_to_string(&config_path).map_err(|source| {
            LuaManagerError::PluginConfigRead {
                path: config_path.clone(),
                source,
            }
        })?;

        let config = toml::from_str::<PluginConfig>(&text).map_err(|source| {
            LuaManagerError::PluginConfigParse {
                path: config_path.clone(),
                source,
            }
        })?;

        let canonical_source = source_dir.canonicalize().map_err(|source| {
            LuaManagerError::PluginPathCanonicalize {
                path: source_dir.clone(),
                source,
            }
        })?;

        let source = canonical_source
            .as_os_str()
            .to_str()
            .ok_or(LuaManagerError::PluginPathInvalidUtf8 {
                path: canonical_source.clone(),
            })?
            .to_owned();

        let plugin = LoadedPlugin::new(
            self.lua.clone(),
            config.id,
            config.version,
            config.description,
            "",
            &format!(
                "{source}{separator}?.lua;{source}{separator}?{separator}init.lua",
                source = source,
                separator = std::path::MAIN_SEPARATOR
            ),
            &self.globals,
        )
        .map_err(|source| LuaManagerError::PluginLoad {
            path: source_dir.clone(),
            source: Box::new(source),
        })?;

        plugin
            .init(source_dir.join("init.lua"))
            .map_err(|source| LuaManagerError::PluginLoad {
                path: source_dir.clone(),
                source: Box::new(source),
            })?;

        let mut plugins = self.loaded_plugins.write().expect("RwLock was poisoned");

        if let Some(existing) = plugins.insert(plugin.id.clone(), plugin) {
            println!(
                "Plugin with ID {} was already loaded, unloading (not implemented yet, undef behavior will happen)",
                existing.id
            );
        }

        drop(plugins);

        Ok(())
    }
}
