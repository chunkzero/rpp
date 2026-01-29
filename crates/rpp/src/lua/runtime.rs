use mlua::{Function, Lua, LuaSerdeExt, Table, Value};
use std::collections::HashMap;
use twox_hash::XxHash3_64;

use crate::build::BuildError;
use crate::sandbox::SandboxContext;

/// Lua runtime wrapper for plugin execution.
pub struct LuaRuntime {
    pub(crate) lua: Lua,
    loaded_plugins: HashMap<(String, String), mlua::RegistryKey>,
    api_context: Option<mlua::RegistryKey>,
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

        // Create and cache API context
        let api_ctx = Self::create_api_context(&lua)?;
        let api_context_key = lua
            .create_registry_value(api_ctx)
            .map_err(|e| BuildError::Plugin(format!("Failed to cache API context: {}", e)))?;

        Ok(Self {
            lua,
            loaded_plugins: HashMap::new(),
            api_context: Some(api_context_key),
        })
    }

    /// Execute a function with access to the sandboxed Lua instance.
    pub fn with_lua<F, R>(&self, f: F) -> Result<R, mlua::Error>
    where
        F: FnOnce(&Lua) -> Result<R, mlua::Error>,
    {
        f(&self.lua)
    }

    /// Create a new runtime with a plugin already loaded.
    pub fn from_source(name: &str, source: &str) -> Result<Self, BuildError> {
        let mut runtime = Self::new()?;
        runtime.load_plugin(name, source)?;
        Ok(runtime)
    }

    /// Load a plugin from source code.
    pub fn load_plugin(&mut self, name: &str, source: &str) -> Result<(), BuildError> {
        // Use version "1.0.0" for backward compatibility
        self.load_plugin_versioned(name, "1.0.0", source)
    }

    /// Load a plugin with explicit version tracking.
    pub fn load_plugin_versioned(
        &mut self,
        name: &str,
        version: &str,
        source: &str,
    ) -> Result<(), BuildError> {
        let key = (name.to_string(), version.to_string());

        // Skip if already loaded
        if self.loaded_plugins.contains_key(&key) {
            return Ok(());
        }

        // Get shared API context
        let api_ctx: Table = self
            .lua
            .registry_value(
                self.api_context
                    .as_ref()
                    .ok_or_else(|| BuildError::Plugin("No API context".into()))?,
            )
            .map_err(|e| BuildError::Plugin(format!("Failed to get API context: {}", e)))?;

        // Create per-plugin environment table
        let plugin_env = self
            .lua
            .create_table()
            .map_err(|e| BuildError::Plugin(format!("Failed to create environment: {}", e)))?;

        // Sandbox: Only provide safe globals
        let globals = self.lua.globals();
        plugin_env
            .set(
                "_VERSION",
                globals
                    .get::<String>("_VERSION")
                    .map_err(|e| BuildError::Plugin(e.to_string()))?,
            )
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        plugin_env
            .set(
                "assert",
                globals
                    .get::<Function>("assert")
                    .map_err(|e| BuildError::Plugin(e.to_string()))?,
            )
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        plugin_env
            .set(
                "error",
                globals
                    .get::<Function>("error")
                    .map_err(|e| BuildError::Plugin(e.to_string()))?,
            )
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        plugin_env
            .set(
                "ipairs",
                globals
                    .get::<Function>("ipairs")
                    .map_err(|e| BuildError::Plugin(e.to_string()))?,
            )
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        plugin_env
            .set(
                "next",
                globals
                    .get::<Function>("next")
                    .map_err(|e| BuildError::Plugin(e.to_string()))?,
            )
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        plugin_env
            .set(
                "pairs",
                globals
                    .get::<Function>("pairs")
                    .map_err(|e| BuildError::Plugin(e.to_string()))?,
            )
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        plugin_env
            .set(
                "pcall",
                globals
                    .get::<Function>("pcall")
                    .map_err(|e| BuildError::Plugin(e.to_string()))?,
            )
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        plugin_env
            .set(
                "select",
                globals
                    .get::<Function>("select")
                    .map_err(|e| BuildError::Plugin(e.to_string()))?,
            )
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        plugin_env
            .set(
                "tonumber",
                globals
                    .get::<Function>("tonumber")
                    .map_err(|e| BuildError::Plugin(e.to_string()))?,
            )
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        plugin_env
            .set(
                "tostring",
                globals
                    .get::<Function>("tostring")
                    .map_err(|e| BuildError::Plugin(e.to_string()))?,
            )
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        plugin_env
            .set(
                "type",
                globals
                    .get::<Function>("type")
                    .map_err(|e| BuildError::Plugin(e.to_string()))?,
            )
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        plugin_env
            .set(
                "xpcall",
                globals
                    .get::<Function>("xpcall")
                    .map_err(|e| BuildError::Plugin(e.to_string()))?,
            )
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        // Add table, string, math libs
        plugin_env
            .set(
                "table",
                globals
                    .get::<Table>("table")
                    .map_err(|e| BuildError::Plugin(e.to_string()))?,
            )
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        plugin_env
            .set(
                "string",
                globals
                    .get::<Table>("string")
                    .map_err(|e| BuildError::Plugin(e.to_string()))?,
            )
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        plugin_env
            .set(
                "math",
                globals
                    .get::<Table>("math")
                    .map_err(|e| BuildError::Plugin(e.to_string()))?,
            )
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        // Set metatable to inherit shared APIs via __index
        let meta = self
            .lua
            .create_table()
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        meta.set("__index", api_ctx)
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        let _ = plugin_env.set_metatable(Some(meta));

        // Load chunk with custom environment
        let chunk = self.lua.load(source).set_environment(plugin_env.clone());
        let plugin_table: Table = chunk
            .eval()
            .map_err(|e| BuildError::Plugin(format!("Failed to load '{}': {}", name, e)))?;

        // Validate plugin table structure
        if plugin_table
            .get::<Function>("process")
            .map_err(|e| BuildError::Plugin(e.to_string()))
            .is_err()
        {
            return Err(BuildError::Plugin(format!(
                "Plugin '{}' missing 'process' function",
                name
            )));
        }

        let registry_key = self
            .lua
            .create_registry_value(plugin_table)
            .map_err(|e| BuildError::Plugin(format!("Failed to register '{}': {}", name, e)))?;

        self.loaded_plugins.insert(key, registry_key);
        Ok(())
    }

    /// Check if a plugin is loaded.
    pub fn has_plugin(&self, name: &str, version: &str) -> bool {
        self.loaded_plugins
            .contains_key(&(name.to_string(), version.to_string()))
    }

    /// Unload a plugin from the runtime.
    pub fn unload_plugin(&mut self, name: &str, version: &str) {
        let key = (name.to_string(), version.to_string());
        if let Some(registry_key) = self.loaded_plugins.remove(&key) {
            let _ = self.lua.remove_registry_value(registry_key);
        }
    }

    /// Call a processor plugin's process function.
    pub fn call_processor(
        &self,
        name: &str,
        path: &str,
        content: &[u8],
        _sandbox: &SandboxContext,
    ) -> Result<LuaProcessResult, BuildError> {
        // Use default version for backward compatibility
        let key = self
            .loaded_plugins
            .get(&(name.to_string(), "1.0.0".to_string()))
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

    /// Create persistent API context (called once during runtime creation).
    fn create_api_context(lua: &Lua) -> Result<Table, BuildError> {
        let ctx = lua
            .create_table()
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        let json = Self::create_json_api_static(lua)?;
        let hash = Self::create_hash_api_static(lua)?;
        let log = Self::create_log_api_static(lua)?;

        ctx.set("json", json)
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        ctx.set("hash", hash)
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        ctx.set("log", log)
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        Ok(ctx)
    }

    /// Process a chain of plugins efficiently in Lua space.
    pub fn process_chain(
        &self,
        path: &str,
        content: &[u8],
        processors: &[(String, String)], // (name, version) tuples
    ) -> Result<ChainResult, BuildError> {
        // Get cached API context
        let api_ctx: Table = self
            .lua
            .registry_value(
                self.api_context
                    .as_ref()
                    .ok_or_else(|| BuildError::Plugin("No API context".into()))?,
            )
            .map_err(|e| BuildError::Plugin(format!("Failed to get API context: {}", e)))?;

        // Create per-call context table with __index to api_ctx
        let call_ctx = self
            .lua
            .create_table()
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        call_ctx
            .set("path", path)
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        let meta = self
            .lua
            .create_table()
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        meta.set("__index", api_ctx)
            .map_err(|e| BuildError::Plugin(e.to_string()))?;
        let _ = call_ctx.set_metatable(Some(meta));

        // Create initial input
        let mut current_content = content.to_vec();
        let mut current_path = path.to_string();
        let mut transformations = Vec::new();

        // Chain through all processors in Lua space
        for (idx, (name, version)) in processors.iter().enumerate() {
            let input_hash = XxHash3_64::oneshot(&current_content);

            // Create input table for this processor
            let input_table = self
                .lua
                .create_table()
                .map_err(|e| BuildError::Plugin(e.to_string()))?;
            input_table
                .set("path", current_path.as_str())
                .map_err(|e| BuildError::Plugin(e.to_string()))?;
            input_table
                .set(
                    "content",
                    self.lua
                        .create_string(&current_content)
                        .map_err(|e| BuildError::Plugin(e.to_string()))?,
                )
                .map_err(|e| BuildError::Plugin(e.to_string()))?;

            let plugin_key = self
                .loaded_plugins
                .get(&(name.clone(), version.clone()))
                .ok_or_else(|| {
                    BuildError::Plugin(format!("Plugin '{}' v{} not loaded", name, version))
                })?;

            let plugin_table: Table = self
                .lua
                .registry_value(plugin_key)
                .map_err(|e| BuildError::Plugin(e.to_string()))?;
            let process_fn: Function = plugin_table
                .get("process")
                .map_err(|e| BuildError::Plugin(e.to_string()))?;

            // Call with per-call context and input
            let result: Value = process_fn
                .call((call_ctx.clone(), input_table))
                .map_err(|e| {
                    BuildError::Plugin(format!("'{}' v{} failed: {}", name, version, e))
                })?;

            // Validate result is a table
            let result_table: Table = match result {
                Value::Table(t) => t,
                Value::Nil => {
                    return Err(BuildError::Plugin(format!(
                        "Plugin '{}' v{} returned nil",
                        name, version
                    )))
                }
                _ => {
                    return Err(BuildError::Plugin(format!(
                        "Plugin '{}' v{} returned non-table",
                        name, version
                    )))
                }
            };

            // Check action
            let action: String = result_table
                .get("action")
                .unwrap_or_else(|_| "continue".to_string());

            match action.as_str() {
                "continue" => {
                    // Extract new content and path
                    let new_content: mlua::String = result_table.get("content").map_err(|e| {
                        BuildError::Plugin(format!(
                            "Plugin '{}' missing 'content' in result: {}",
                            name, e
                        ))
                    })?;
                    let new_path: Option<String> = result_table.get("path").ok();

                    let content_bytes = new_content.as_bytes().to_vec();
                    let output_hash = XxHash3_64::oneshot(&content_bytes);

                    // Record transformation
                    transformations.push((
                        name.clone(),
                        version.clone(),
                        input_hash,
                        output_hash,
                    ));

                    current_content = content_bytes;
                    if let Some(p) = new_path {
                        current_path = p;
                    }
                }
                "skip" => {
                    // Skip means stop processing this file entirely
                    return Ok(ChainResult {
                        content: current_content,
                        output_path: Some(current_path),
                        transformations,
                        cancelled: false,
                        skipped_at: Some(idx),
                    });
                }
                "cancel" => {
                    return Ok(ChainResult {
                        content: current_content,
                        output_path: None,
                        transformations,
                        cancelled: true,
                        skipped_at: None,
                    });
                }
                _ => {
                    return Err(BuildError::Plugin(format!(
                        "Plugin '{}' returned unknown action: {}",
                        name, action
                    )))
                }
            }
        }

        Ok(ChainResult {
            content: current_content,
            output_path: Some(current_path),
            transformations,
            cancelled: false,
            skipped_at: None,
        })
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

    // Static versions of API creation for shared context
    fn create_json_api_static(lua: &Lua) -> Result<Table, BuildError> {
        let json = lua
            .create_table()
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        let decode = lua
            .create_function(|lua, s: mlua::String| {
                let value: serde_json::Value = serde_json::from_slice(s.as_bytes().as_ref())
                    .map_err(|e| mlua::Error::external(e))?;
                lua.to_value(&value)
            })
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        let encode = lua
            .create_function(|lua, value: Value| {
                let json_value: serde_json::Value = lua.from_value(value)?;
                serde_json::to_string_pretty(&json_value).map_err(|e| mlua::Error::external(e))
            })
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        let encode_compact = lua
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

    fn create_hash_api_static(lua: &Lua) -> Result<Table, BuildError> {
        let hash = lua
            .create_table()
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        let xxhash3 = lua
            .create_function(|_, s: mlua::String| {
                use twox_hash::XxHash3_64;
                Ok(XxHash3_64::oneshot(s.as_bytes().as_ref()))
            })
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        let sha256 = lua
            .create_function(|_, s: mlua::String| {
                use sha2::{Digest, Sha256};
                let mut hasher = Sha256::new();
                hasher.update(s.as_bytes().as_ref());
                Ok(format!("{:x}", hasher.finalize()))
            })
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        let md5 = lua
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

    fn create_log_api_static(lua: &Lua) -> Result<Table, BuildError> {
        let log = lua
            .create_table()
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        let debug = lua
            .create_function(|_, msg: String| {
                #[cfg(feature = "tracing")]
                tracing::debug!("{}", msg);
                #[cfg(not(feature = "tracing"))]
                eprintln!("[DEBUG] {}", msg);
                Ok(())
            })
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        let info = lua
            .create_function(|_, msg: String| {
                #[cfg(feature = "tracing")]
                tracing::info!("{}", msg);
                #[cfg(not(feature = "tracing"))]
                eprintln!("[INFO] {}", msg);
                Ok(())
            })
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        let warn = lua
            .create_function(|_, msg: String| {
                #[cfg(feature = "tracing")]
                tracing::warn!("{}", msg);
                #[cfg(not(feature = "tracing"))]
                eprintln!("[WARN] {}", msg);
                Ok(())
            })
            .map_err(|e| BuildError::Plugin(e.to_string()))?;

        let error = lua
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

/// Result from processing a chain of plugins.
pub struct ChainResult {
    pub content: Vec<u8>,
    pub output_path: Option<String>,
    pub transformations: Vec<(String, String, u64, u64)>, // (name, version, input_hash, output_hash)
    pub cancelled: bool,
    pub skipped_at: Option<usize>, // Which processor returned Skip
}
