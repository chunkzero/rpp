use std::{collections::HashMap, path::PathBuf};

pub struct PluginEnvironmentBuilder {
    lua: mlua::WeakLua,
    env: mlua::Table,
    package: mlua::Table,
}

impl PluginEnvironmentBuilder {
    pub fn new(lua: &mlua::Lua) -> crate::Result<Self> {
        let env = lua.create_table()?;
        let package = Self::create_package(lua)?;

        env.set("_G", &env)?;
        env.set("package", &package)?;

        Ok(Self {
            lua: lua.weak(),
            env,
            package,
        })
    }

    fn create_package(lua: &mlua::Lua) -> crate::Result<mlua::Table> {
        let table = lua.create_table()?;

        // set these for some compatibility
        table.set("config", lua.globals().get::<mlua::Value>("config")?)?;
        table.set("preload", lua.create_table()?)?;

        Ok(table)
    }
    // Similar fn names again.
    pub fn with_global(self, key: &str, value: impl mlua::IntoLua) -> crate::Result<Self> {
        self.env.set(key, value)?;
        Ok(self)
    }

    pub fn with_globals<'a>(
        self,
        globals: impl IntoIterator<Item = (&'a String, &'a mlua::Value)>,
    ) -> crate::Result<Self> {
        for (k, v) in globals {
            self.env.set(k.as_str(), v)?;
        }

        Ok(self)
    }

    pub fn with_require(self, loaders: Vec<Box<dyn ChunkLoader>>) -> crate::Result<Self> {
        self.env
            .set("require", Require::new(self.env.clone(), loaders))?;

        Ok(self)
    }

    pub fn with_preloader(
        self,
        module_name: &str,
        preloader: mlua::Function,
        set_env: bool,
    ) -> crate::Result<Self> {
        let preload = match self.package.get::<mlua::Value>("preload")? {
            mlua::Value::Table(table) => table,
            _ => {
                return Err(crate::Error::Custom(
                    "Table `preload` did not exist on environment package".into(),
                ))
            }
        };

        if set_env {
            preloader.set_environment(self.env.clone())?;
        }

        preload.set(module_name, preloader)?;

        Ok(self)
    }

    pub fn build(self) -> mlua::Table {
        self.env
    }
}

pub trait ChunkLoader {
    fn load_chunk(
        &self,
        lua: &mlua::Lua,
        env: &mlua::Table,
        module_name: &str,
    ) -> crate::Result<Option<mlua::Function>>;
}

pub struct Require {
    env: mlua::Table,

    loaded: HashMap<String, mlua::Value>,
    // I'll be honest I'm not taking the time to understand every aspect of your code base so this is just food for thought: Whenever possible, you want to use generics instead of dyn traits. This saves the program from having to make a vtable and saves you from having to worry about dyn compatibility. Not sure if you need it here, but generally it's typically return types that sometimes need to use dyn traits.  
    loaders: Vec<Box<dyn ChunkLoader>>,
}

impl Require {
    pub fn new(env: mlua::Table, loaders: Vec<Box<dyn ChunkLoader>>) -> Self {
        Self {
            env,
            loaded: Default::default(),
            loaders,
        }
    }
}

impl mlua::UserData for Require {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut(
            mlua::MetaMethod::Call,
            |lua, this, module_name: String| -> mlua::Result<mlua::Value> {
                if let Some(value) = this.loaded.get(&module_name) {
                    return Ok(value.clone());
                }

                for loader in this.loaders.iter() {
                    if let Some(chunk) = loader.load_chunk(lua, &this.env, &module_name).map_err(
                        |err| match err {
                            crate::Error::Lua(error) => error,
                            err => mlua::Error::external(err),
                        },
                    )? {
                        chunk.set_environment(this.env.clone())?;

                        let value = chunk.call::<mlua::Value>(())?;

                        let result = if value.is_nil() {
                            mlua::Value::Boolean(true)
                        } else {
                            value
                        };

                        this.loaded.insert(module_name, result.clone());
                        return Ok(result);
                    }
                }

                Err(mlua::Error::RuntimeError(format!(
                    "module '{}' not found",
                    module_name
                )))
            },
        );
    }
}

#[derive(Debug)]
pub struct PreloadChunkLoader;

impl ChunkLoader for PreloadChunkLoader {
    fn load_chunk(
        &self,
        _: &mlua::Lua,
        env: &mlua::Table,
        module_name: &str,
    ) -> crate::Result<Option<mlua::Function>> {
        let package = match env.get::<mlua::Value>("package")? {
            mlua::Value::Table(table) => table,
            _ => return Ok(None),
        };

        match package.get::<mlua::Value>("preload")? {
            mlua::Value::Nil => Ok(None),
            mlua::Value::Table(table) => table
                .get::<mlua::Function>(module_name)
                .map(|function| Some(function))
                .map_err(|err| err.into()),
            _ => Ok(None),
        }
    }
}

#[derive(Debug)]
pub struct PathChunkLoader(PathBuf);

impl PathChunkLoader {
    pub fn new(path: impl Into<PathBuf>) -> std::io::Result<Box<Self>> {
        Ok(Box::new(Self(path.into().canonicalize()?)))
    }
}

impl ChunkLoader for PathChunkLoader {
    fn load_chunk(
        &self,
        lua: &mlua::Lua,
        _: &mlua::Table,
        module_name: &str,
    ) -> crate::Result<Option<mlua::Function>> {
        let mut path = self.0.clone();

        for part in module_name.split('.') {
            path.push(part);
        }

        path.set_extension("lua");

        if path.exists() {
            return Ok(Some(lua.load(path).into_function()?));
        }

        path.set_extension("");
        path.push("init.lua");
        if path.exists() {
            return Ok(Some(lua.load(path).into_function()?));
        }

        Ok(None)
    }
}
