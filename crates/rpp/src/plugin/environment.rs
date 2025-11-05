use crate::plugin::manager::Globals;

pub fn create_plugin_environment(
    lua: &mlua::Lua,
    globals: &Globals,
    cpath: &str,
    path: &str,
) -> crate::Result<mlua::Table> {
    let table = lua.create_table()?;

    table.set("_G", table.clone())?;
    for (k, v) in globals {
        table.set(k.as_str(), v)?;
    }

    let package = create_package(lua, &table, cpath, path)?;
    table.set("package", &package)?;

    table.set("require", create_require(lua, &package)?)?;

    Ok(table)
}

fn create_package(
    lua: &mlua::Lua,
    env: &mlua::Table,
    cpath: &str,
    path: &str,
) -> crate::Result<mlua::Table> {
    let globals_package: mlua::Table = lua.globals().get("package")?;

    let package = lua.create_table()?;
    let loaded = lua.create_table()?;
    let preload = lua.create_table()?;

    for ele in globals_package
        .get::<mlua::Table>("preload")?
        .pairs::<mlua::Value, mlua::Value>()
    {
        let (key, value) = ele?;
        preload.set(key, value)?;
    }

    package.set("config", globals_package.get::<mlua::Value>("config")?)?;
    package.set("loaded", loaded)?;
    package.set("cpath", cpath)?;
    package.set("path", path)?;
    package.set("preload", preload)?;
    package.set("searchers", create_searchers(lua, env)?)?;

    Ok(package)
}

fn create_searchers(lua: &mlua::Lua, env: &mlua::Table) -> crate::Result<mlua::Table> {
    let searchers = lua.create_table()?;

    searchers.set(1, create_preload_searcher(lua, env)?)?;
    searchers.set(2, create_file_searcher(lua, env)?)?;

    Ok(searchers)
}

fn create_preload_searcher(lua: &mlua::Lua, env: &mlua::Table) -> crate::Result<mlua::Function> {
    lua.create_function({
        let env = env.clone();
        move |_, module_name: String| -> mlua::Result<mlua::Value> {
            match env.get::<mlua::Value>("package") {
                Ok(mlua::Value::Table(package)) => match package.get::<mlua::Value>("preload") {
                    Ok(mlua::Value::Table(preload)) => preload.get(module_name),
                    _ => Ok(mlua::Value::Nil),
                },
                _ => Ok(mlua::Value::Nil),
            }
        }
    })
    .map_err(|err| err.into())
}

fn create_file_searcher(lua: &mlua::Lua, env: &mlua::Table) -> crate::Result<mlua::Function> {
    lua.create_function({
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
    })
    .map_err(|err| err.into())
}

fn create_require(lua: &mlua::Lua, package: &mlua::Table) -> crate::Result<mlua::Function> {
    lua.create_function({
        let plugin_package = package.clone();
        move |_, module_name: String| -> mlua::Result<mlua::Value> {
            require_module(&plugin_package, module_name)
        }
    })
    .map_err(|err| err.into())
}

fn require_module(plugin_package: &mlua::Table, module_name: String) -> mlua::Result<mlua::Value> {
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
