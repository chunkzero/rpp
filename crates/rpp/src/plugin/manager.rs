use std::{collections::HashMap, fs, path::PathBuf, sync::RwLock};

use mlua::Lua;
use serde::{Deserialize, Serialize};

use crate::plugin::{LoadedPlugin, PluginConfig};

pub(crate) type Globals = HashMap<String, mlua::Value>;

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
            .pairs::<String, mlua::Value>()
            .filter_map(|ele| ele.ok())
            .filter(|(k, _)| {
                !["_G", "package", "coroutine", "require", "module"].contains(&k.as_str())
            })
            .collect();

        Self::new_with_globals(lua, globals)
    }

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

    pub fn load_plugins(&self, container_dir: impl Into<PathBuf>) -> crate::Result<()> {
        let path = container_dir.into();

        if !path.is_dir() {
            return Err(crate::Error::PluginError(String::from(
                "Plugin container directory was not a directory",
            )));
        };

        let dir = path.read_dir()?;
        for ele in dir {
            let child = ele?;
            self.load_plugin(child.path())?;
        }

        Ok(())
    }

    fn load_plugin(&self, plugin_dir: impl Into<PathBuf>) -> crate::Result<()> {
        let path = plugin_dir.into();

        let source_dir: PathBuf;

        let config: PluginConfig = if path.is_dir() {
            source_dir = path;

            // look for toml config
            let config = source_dir.join("plugin.toml");

            let text = fs::read_to_string(&config)?;

            toml::from_str(&text).map_err(|err| {
                crate::Error::PluginError(format!(
                    "Failed to parse plugin config: {}",
                    err.message()
                ))
            })?
        } else {
            source_dir = path
                .parent()
                .ok_or(crate::Error::PluginError(String::from(
                    "Plugin config had no parent",
                )))?
                .into();

            let text = fs::read_to_string(&source_dir)?;

            toml::from_str(&text).map_err(|err| {
                crate::Error::PluginError(format!(
                    "Failed to parse plugin config: {}",
                    err.message()
                ))
            })?
        };

        let plugin = LoadedPlugin::new(
            self.lua.clone(),
            config.id,
            config.version,
            config.description,
            "",
            &(format!(
                "{source}{separator}?.lua;{source}{separator}?{separator}init.lua",
                source = source_dir
                    .canonicalize()
                    .unwrap()
                    .as_os_str()
                    .to_str()
                    .unwrap(),
                separator = std::path::MAIN_SEPARATOR
            )),
            &self.globals,
        )?;

        // TODO: configurable source dir and whatnot
        plugin.init(source_dir.join("init.lua"))?;

        let mut plugins = self.loaded_plugins.write().expect("RwLock was poisoned");

        if let Some(existing) = plugins.insert(plugin.id.clone(), plugin) {
            if cfg!(feature = "tracing") {
                tracing::warn!("Plugin with ID {} was already loaded, unloading (not implemented yet, undef behavior will happen)", existing.id);
            };
        }

        drop(plugins);

        Ok(())
    }
}
