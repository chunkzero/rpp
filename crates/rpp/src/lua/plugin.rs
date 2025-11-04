use std::path::PathBuf;

#[derive(Debug)]
pub struct PluginEnvironment {
    lua: mlua::Lua,

    root: PathBuf,
    env: mlua::Table,
}

impl PluginEnvironment {
    pub fn new(lua: mlua::Lua, root: impl Into<PathBuf>) -> crate::Result<Self> {
        let root = root.into();

        let env = Self::create_env(&lua, &root)?;

        Ok(Self { lua, root, env })
    }

    pub fn init(&self) -> crate::Result<()> {
        let src = self.root.join("plugin.lua");

        self.lua
            .load(src.clone())
            .set_name(src.to_string_lossy().as_ref())
            .set_environment(self.env.clone())
            .exec()?;

        Ok(())
    }

    fn create_env(lua: &mlua::Lua, root: &std::path::Path) -> crate::Result<mlua::Table> {
        let env = lua.create_table()?;

        Self::setup_basics(lua, &env)?;
        Self::setup_package(lua, &env, root)?;
        Self::setup_require(lua, &env)?;

        Ok(env)
    }

    fn setup_basics(lua: &mlua::Lua, env: &mlua::Table) -> crate::Result<()> {
        let globals = lua.globals();

        env.set("_G", env.clone())?;
        env.set("print", globals.get::<mlua::Function>("print")?)?;

        for name in &[
            "type",
            "pairs",
            "ipairs",
            "next",
            "tostring",
            "tonumber",
            "getmetatable",
            "setmetatable",
        ] {
            if let Ok(value) = globals.get::<mlua::Value>(*name) {
                env.set(*name, value)?;
            }
        }

        Ok(())
    }

    fn setup_package(
        lua: &mlua::Lua,
        env: &mlua::Table,
        root: &std::path::Path,
    ) -> crate::Result<()> {
        let plugin_package = lua.create_table()?;
        let loaded = lua.create_table()?;

        plugin_package.set("loaded", loaded)?;

        plugin_package.set("path", Self::build_path(lua, root)?)?;

        env.set("package", plugin_package)?;

        let searchers = Self::create_searchers(lua, env)?;
        let plugin_package: mlua::Table = env.get("package")?;
        plugin_package.set("searchers", searchers)?;

        Ok(())
    }

    fn build_path(lua: &mlua::Lua, root: &std::path::Path) -> crate::Result<String> {
        let globals = lua.globals();
        let global_package: mlua::Table = globals.get("package")?;
        let old_path: String = global_package.get("path")?;

        let dir_str = root.to_string_lossy();
        let plugin_path = format!("{}/?.lua;{}/?/init.lua;{}", dir_str, dir_str, old_path);

        Ok(plugin_path)
    }

    fn create_searchers(lua: &mlua::Lua, env: &mlua::Table) -> crate::Result<mlua::Table> {
        let searchers = lua.create_table()?;

        let file_searcher = Self::create_file_searcher(lua, env)?;
        searchers.set(1, file_searcher)?;

        Ok(searchers)
    }

    fn create_file_searcher(lua: &mlua::Lua, env: &mlua::Table) -> crate::Result<mlua::Function> {
        let env = env.clone();

        let searcher = lua.create_function({
            let env = env.clone();
            move |lua, module_name: String| -> mlua::Result<mlua::Value> {
                let plugin_package: mlua::Table = env.get("package")?;
                let plugin_path = plugin_package.get::<String>("path")?;

                for pattern in plugin_path.split(';') {
                    let file_path = pattern.replace('?', &module_name);
                    if std::path::Path::new(&file_path).exists() {
                        let loader = lua.create_function({
                            let file_path = file_path.clone();
                            let env = env.clone();
                            move |lua, _: ()| -> mlua::Result<mlua::Value> {
                                let content = std::fs::read_to_string(&file_path)
                                    .map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;

                                lua.load(&content)
                                    .set_name(&file_path)
                                    .set_environment(env.clone())
                                    .call(())
                            }
                        })?;
                        return Ok(mlua::Value::Function(loader));
                    }
                }
                Ok(mlua::Value::Nil)
            }
        })?;

        Ok(searcher)
    }

    fn setup_require(lua: &mlua::Lua, env: &mlua::Table) -> crate::Result<()> {
        let plugin_package: mlua::Table = env.get("package")?;

        let custom_require = lua.create_function({
            let plugin_package = plugin_package.clone();
            move |_lua, module_name: String| -> mlua::Result<mlua::Value> {
                Self::require_module(&plugin_package, module_name)
            }
        })?;

        env.set("require", custom_require)?;

        Ok(())
    }

    fn require_module(
        plugin_package: &mlua::Table,
        module_name: String,
    ) -> mlua::Result<mlua::Value> {
        let loaded: mlua::Table = plugin_package.get("loaded")?;

        if let Ok(cached) = loaded.get::<mlua::Value>(module_name.clone()) {
            if !cached.is_nil() {
                return Ok(cached);
            }
        }

        let searchers: mlua::Table = plugin_package.get("searchers")?;

        for searcher in searchers.sequence_values::<mlua::Function>() {
            let searcher = searcher?;
            let result = searcher.call::<mlua::Value>(module_name.clone())?;

            if let mlua::Value::Function(loader) = result {
                let module_result = loader.call::<mlua::Value>(())?;

                let final_result = if module_result.is_nil() {
                    mlua::Value::Boolean(true)
                } else {
                    module_result
                };

                loaded.set(module_name, final_result.clone())?;
                return Ok(final_result);
            }
        }

        Err(mlua::Error::RuntimeError(format!(
            "module '{}' not found",
            module_name
        )))
    }
}
