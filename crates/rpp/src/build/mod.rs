use mlua::{FromLua, FromLuaMulti, Table};
use thiserror::Error;

mod ignore;

#[derive(Error, Debug)]
enum BuildError {
    #[error(transparent)]
    Lua(#[from] mlua::Error),
}

struct BuildContext {}

impl BuildContext {
    fn to_lua(self, lua: &mlua::Lua) -> Result<mlua::Table, BuildError> {
        let table = lua.create_table().map_err(|err| BuildError::Lua(err))?;

        Ok(table)
    }
}
impl FromLua for BuildContext {
    fn from_lua(value: mlua::Value, lua: &mlua::Lua) -> mlua::Result<Self> {
        todo!()
    }
}
