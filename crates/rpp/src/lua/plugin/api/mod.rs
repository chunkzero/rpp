use std::path::PathBuf;

use include_directory::{include_directory, Dir};

use crate::lua::plugin::environment::ChunkLoader;

static LIB: Dir<'_> = include_directory!("$CARGO_MANIFEST_DIR/src/lua/plugin/api/lib");

pub struct ApiChunkLoader;

impl ChunkLoader for ApiChunkLoader {
    fn load_chunk(
        &self,
        lua: &mlua::Lua,
        _: &mlua::Table,
        module_name: &str,
    ) -> crate::Result<Option<mlua::Function>> {
        LIB.get_entry(module_name.split('.').collect::<PathBuf>())
            .map(|entry| match entry {
                include_directory::DirEntry::Dir(dir) => dir
                    .get_file("init.lua")
                    .map(|file| lua.load(file.contents()).into_function()),
                include_directory::DirEntry::File(file) => {
                    Some(lua.load(file.contents()).into_function())
                }
            })
            .flatten()
            .transpose()
            .map_err(|err| err.into())
    }
}
