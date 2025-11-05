use include_directory::{include_directory, Dir};

pub static LUA_API: Dir<'_> = include_directory!("$CARGO_MANIFEST_DIR/src/lua/api");
pub static RPP_PLUGIN: Dir<'_> = include_directory!("$CARGO_MANIFEST_DIR/src/lua/rpp");

pub static VERSION: &str = env!("CARGO_PKG_VERSION");
