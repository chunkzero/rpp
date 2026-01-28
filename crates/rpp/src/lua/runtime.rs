use mlua::{Function, Lua, LuaSerdeExt, Table, Value};
use std::collections::HashMap;

use crate::build::BuildError;
use crate::sandbox::SandboxContext;

/// Lua runtime wrapper for plugin execution.
pub struct LuaRuntime {
    lua: Lua,
    loaded_plugins: HashMap<String, mlua::RegistryKey>,
}

impl LuaRuntime {
    /// Create a new Lua runtime with sandboxed environment.
    pub fn new() -> Result<Self, BuildError> {
        let lua = Lua::new();

        // Disable dangerous functions
        lua.globals()
            .set("os", Value::Nil)
            .map_err(|e| BuildError::Plugin(format!("Failed to sandbox: {}", e)))?;
        lua.globals()
            .set("io", Value::Nil)
            .map_err(|e| BuildError::Plugin(format!("Failed to sandbox: {}", e)))?;
        lua.globals()
            .set("loadfile", Value::Nil)
            .map_err(|e| BuildError::Plugin(format!("Failed to sandbox: {}", e)))?;
        lua.globals()
            .set("dofile", Value::Nil)
            .map_err(|e| BuildError::Plugin(format!("Failed to sandbox: {}", e)))?;

        Ok(Self {
            lua,
            loaded_plugins: HashMap::new(),
        })
    }

    /// Load a plugin from source code.
    pub fn load_plugin(&mut self, name: &str, source: &str) -> Result<(), BuildError> {
        let chunk = self.lua.load(source);
        let plugin_table: Table = chunk
            .eval()
            .map_err(|e| BuildError::Plugin(format!("Failed to load '{}': {}", name, e)))?;

        let key = self
            .lua
            .create_registry_value(plugin_table)
            .map_err(|e| BuildError::Plugin(format!("Failed to register '{}': {}", name, e)))?;

        self.loaded_plugins.insert(name.to_string(), key);
        Ok(())
    }

    /// Call a processor plugin's process function.
    pub fn call_processor(
        &self,
        name: &str,
        path: &str,
        content: &[u8],
        _sandbox: &SandboxContext,
    ) -> Result<LuaProcessResult, BuildError> {
        let key = self
            .loaded_plugins
            .get(name)
            .ok_or_else(|| BuildError::Plugin(format!("Plugin '{}' not loaded", name)))?;

        let plugin_table: Table = self
            .lua
            .registry_value(key)
            .map_err(|e| BuildError::Plugin(format!("Failed to get '{}': {}", name, e)))?;

        let process_fn: Function = plugin_table
            .get("process")
            .map_err(|e| BuildError::Plugin(format!("'{}' missing process: {}", name, e)))?;

        // Create context table with nested APIs
        let ctx = self.create_context_table()?;

        // Create input table
        let input = self
            .lua
            .create_table()
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        input
            .set("path", path)
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        let content_str = self
            .lua
            .create_string(content)
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        input
            .set("content", content_str)
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        // Call process function
        let result: Table = process_fn
            .call((ctx, input))
            .map_err(|e| BuildError::Plugin(format!("'{}' failed: {}", name, e)))?;

        // Parse result
        let action: String = result
            .get("action")
            .unwrap_or_else(|_| "continue".to_string());

        match action.as_str() {
            "continue" => {
                let content: mlua::String = result
                    .get("content")
                    .map_err(|e| BuildError::Plugin(format!("Missing content: {}", e)))?;
                let path: Option<String> = result.get("path").ok();

                Ok(LuaProcessResult::Continue {
                    content: content.as_bytes().to_vec(),
                    path,
                })
            }
            "skip" => Ok(LuaProcessResult::Skip),
            "cancel" => Ok(LuaProcessResult::Cancel),
            _ => Err(BuildError::Plugin(format!("Unknown action: {}", action))),
        }
    }

    fn create_context_table(&self) -> Result<Table, BuildError> {
        let ctx = self
            .lua
            .create_table()
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        let json = self.create_json_api()?;
        let hash = self.create_hash_api()?;
        let log = self.create_log_api()?;

        ctx.set("json", json)
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        ctx.set("hash", hash)
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        ctx.set("log", log)
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        Ok(ctx)
    }

    fn create_json_api(&self) -> Result<Table, BuildError> {
        let json = self
            .lua
            .create_table()
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        let decode = self
            .lua
            .create_function(|lua, s: mlua::String| {
                let value: serde_json::Value = serde_json::from_slice(s.as_bytes().as_ref())
                    .map_err(|e| mlua::Error::external(e))?;
                lua.to_value(&value)
            })
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        let encode = self
            .lua
            .create_function(|lua, value: Value| {
                let json_value: serde_json::Value = lua.from_value(value)?;
                serde_json::to_string_pretty(&json_value).map_err(|e| mlua::Error::external(e))
            })
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        let encode_compact = self
            .lua
            .create_function(|lua, value: Value| {
                let json_value: serde_json::Value = lua.from_value(value)?;
                serde_json::to_string(&json_value).map_err(|e| mlua::Error::external(e))
            })
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        json.set("decode", decode)
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        json.set("encode", encode)
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        json.set("encode_compact", encode_compact)
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        Ok(json)
    }

    fn create_hash_api(&self) -> Result<Table, BuildError> {
        let hash = self
            .lua
            .create_table()
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        let xxhash3 = self
            .lua
            .create_function(|_, s: mlua::String| {
                use twox_hash::XxHash3_64;
                Ok(XxHash3_64::oneshot(s.as_bytes().as_ref()))
            })
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        let sha256 = self
            .lua
            .create_function(|_, s: mlua::String| {
                use sha2::{Digest, Sha256};
                let mut hasher = Sha256::new();
                hasher.update(s.as_bytes().as_ref());
                Ok(format!("{:x}", hasher.finalize()))
            })
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        let md5 = self
            .lua
            .create_function(|_, s: mlua::String| {
                let digest = md5::compute(s.as_bytes().as_ref());
                Ok(format!("{:x}", digest))
            })
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        hash.set("xxhash3", xxhash3)
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        hash.set("sha256", sha256)
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        hash.set("md5", md5)
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        Ok(hash)
    }

    fn create_log_api(&self) -> Result<Table, BuildError> {
        let log = self
            .lua
            .create_table()
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        let debug = self
            .lua
            .create_function(|_, msg: String| {
                #[cfg(feature = "tracing")]
                tracing::debug!("{}", msg);
                #[cfg(not(feature = "tracing"))]
                eprintln!("[DEBUG] {}", msg);
                Ok(())
            })
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        let info = self
            .lua
            .create_function(|_, msg: String| {
                #[cfg(feature = "tracing")]
                tracing::info!("{}", msg);
                #[cfg(not(feature = "tracing"))]
                eprintln!("[INFO] {}", msg);
                Ok(())
            })
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        let warn = self
            .lua
            .create_function(|_, msg: String| {
                #[cfg(feature = "tracing")]
                tracing::warn!("{}", msg);
                #[cfg(not(feature = "tracing"))]
                eprintln!("[WARN] {}", msg);
                Ok(())
            })
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        let error = self
            .lua
            .create_function(|_, msg: String| {
                #[cfg(feature = "tracing")]
                tracing::error!("{}", msg);
                #[cfg(not(feature = "tracing"))]
                eprintln!("[ERROR] {}", msg);
                Ok(())
            })
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        log.set("debug", debug)
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        log.set("info", info)
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        log.set("warn", warn)
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        log.set("error", error)
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        Ok(log)
    }
}

/// Result from a Lua processor call.
pub enum LuaProcessResult {
    Continue {
        content: Vec<u8>,
        path: Option<String>,
    },
    Skip,
    Cancel,
}
