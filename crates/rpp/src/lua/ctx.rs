//! Construction of the `ctx` tables passed to processors and generators (spec §4).

use mlua::{Lua, LuaSerdeExt, Table};

use crate::host::PackInfo;
use crate::lua::builtins;

/// Build the base ctx table shared by processors and generators:
/// `options`, `pack`, and `log`.
pub(crate) fn base_ctx(
    lua: &Lua,
    plugin_id: &str,
    options: &toml::Value,
    pack: &PackInfo,
) -> mlua::Result<Table> {
    let ctx = lua.create_table()?;

    ctx.set("options", lua.to_value(options)?)?;

    let pack_t = lua.create_table()?;
    pack_t.set("name", pack.name.clone())?;
    pack_t.set("description", pack.description.clone())?;
    pack_t.set("format", pack.format)?;
    ctx.set("pack", pack_t)?;

    ctx.set("log", builtins::log::table(lua, plugin_id)?)?;

    Ok(ctx)
}
