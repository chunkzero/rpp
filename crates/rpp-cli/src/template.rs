use askama::Template;

#[derive(Template)]
#[template(path = "plugin.toml", escape = "none")]
pub struct PluginTomlTemplate<'a> {
    pub id: &'a str,
    pub version: &'a str,
    pub description: &'a str,
}

#[derive(Template)]
#[template(path = "pack.jsonc", escape = "none")]
pub struct PackJsoncTemplate<'a> {
    pub id: &'a str,
}

#[derive(Template)]
#[template(path = ".luarc.json", escape = "none")]
pub struct LuaRcTemplate<'a> {
    pub api_source: &'a str,
}

#[derive(Template)]
#[template(path = "rpp.toml", escape = "none")]
pub struct RppConfigTemplate<'a> {
    pub root: &'a str,
}
