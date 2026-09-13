//! `rpp init`: scaffold a new rpp project (interactive when on a tty).

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::luals;
use crate::project::CONFIG_FILE;
use crate::ui;

/// Arguments for `rpp init`.
#[derive(Debug, Clone, Default)]
pub struct InitArgs {
    /// Target directory (defaults to cwd).
    pub dir: Option<PathBuf>,
    /// Pack name (skips the prompt).
    pub name: Option<String>,
    /// Pack description (skips the prompt).
    pub description: Option<String>,
    /// Pack format (skips the prompt).
    pub pack_format: Option<u32>,
    /// Force non-interactive mode even on a tty.
    pub yes: bool,
}

/// Default pack_format used when none is supplied (a recent stable value).
const DEFAULT_PACK_FORMAT: u32 = 34;

/// Run the init command.
pub fn run(args: InitArgs) -> Result<()> {
    let target = args.dir.clone().unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&target).with_context(|| format!("creating {}", target.display()))?;

    let config_path = target.join(CONFIG_FILE);
    if config_path.exists() {
        bail!(
            "{} already exists; refusing to overwrite",
            config_path.display()
        );
    }

    ui::intro("Create an rpp project");
    let interactive = !args.yes && ui::is_interactive();

    let default_name = target
        .canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().to_string()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "my-pack".to_string());

    let name = match args.name {
        Some(n) => n,
        None if interactive => ui::input("Pack name", &default_name)?,
        None => default_name,
    };
    let description = match args.description {
        Some(d) => d,
        None if interactive => ui::input("Description", "A Minecraft resource pack")?,
        None => "A Minecraft resource pack".to_string(),
    };
    let pack_format = match args.pack_format {
        Some(f) => f,
        None if interactive => loop {
            let answer = ui::input("Pack format", &DEFAULT_PACK_FORMAT.to_string())?;
            match answer.trim().parse::<u32>() {
                Ok(format) if format > 0 => break format,
                _ => ui::warn(format!("`{answer}` is not a valid pack format")),
            }
        },
        None => DEFAULT_PACK_FORMAT,
    };

    scaffold(&target, &name, &description, pack_format)?;

    ui::detail("next: `rpp build`");
    ui::success(format!("Initialized rpp project in {}", target.display()));
    Ok(())
}

/// Write all scaffold files.
fn scaffold(root: &Path, name: &str, description: &str, pack_format: u32) -> Result<()> {
    let destinations = [
        root.join(CONFIG_FILE),
        root.join(".gitignore"),
        root.join("src/pack.mcmeta"),
        root.join("plugins/hello/plugin.toml"),
        root.join("plugins/hello/init.lua"),
        root.join(".rpp/api/rpp.lua"),
        root.join(".rpp/api/file.lua"),
        root.join(".rpp/api/config.json"),
    ];
    if let Some(path) = destinations.iter().find(|path| path.exists()) {
        bail!("{} already exists; refusing to overwrite", path.display());
    }

    write_file(
        &root.join(CONFIG_FILE),
        &rpp_toml(name, description, pack_format),
    )?;
    write_file(&root.join(".gitignore"), GITIGNORE)?;

    let src = root.join("src");
    std::fs::create_dir_all(&src).with_context(|| format!("creating {}", src.display()))?;
    write_file(
        &src.join("pack.mcmeta"),
        &pack_mcmeta(description, pack_format),
    )?;

    // Starter local Lua plugin under plugins/hello/.
    let plugin = root.join("plugins").join("hello");
    std::fs::create_dir_all(&plugin).with_context(|| format!("creating {}", plugin.display()))?;
    write_file(&plugin.join("plugin.toml"), HELLO_PLUGIN_TOML)?;
    write_file(&plugin.join("init.lua"), HELLO_PLUGIN_LUA)?;

    // LuaLS editor definitions.
    luals::write_all_new(&root.join(".rpp").join("api"))?;

    Ok(())
}

fn write_file(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("creating {}", path.display()))?;
    file.write_all(contents.as_bytes())
        .with_context(|| format!("writing {}", path.display()))
}

fn rpp_toml(name: &str, description: &str, pack_format: u32) -> String {
    let name = toml::Value::String(name.to_string());
    let description = toml::Value::String(description.to_string());
    format!(
        r#"[pack]
name = {name}
description = {description}
pack_format = {pack_format}

[build]
source = "src"
output = "dist"
workers = 0

[build.squash]
enabled = true
engine = "builtin"
json = true
png = "fast"
zip = true

[dev]
host = "127.0.0.1"
port = 8080
open = false

# A starter local plugin. Add more with `rpp plugin add <source>`.
[[plugin]]
source = "path:plugins/hello"
[plugin.options]
greeting = "hello"
"#
    )
}

fn pack_mcmeta(description: &str, pack_format: u32) -> String {
    let value = serde_json::json!({
        "pack": {
            "pack_format": pack_format,
            "description": description,
        }
    });
    format!(
        "{}\n",
        serde_json::to_string_pretty(&value).expect("JSON value serializes")
    )
}

const GITIGNORE: &str = "/.rpp/\n/dist/\n";

const HELLO_PLUGIN_TOML: &str = r#"[plugin]
id = "hello"
version = "0.1.0"
description = "A starter rpp plugin"
entry = "init.lua"
"#;

const HELLO_PLUGIN_LUA: &str = r#"-- A starter rpp plugin. See `.rpp/api/` for editor autocomplete.
local rpp = require("rpp")
local plugin = rpp.plugin()

-- Processor: minify every JSON / mcmeta file in the pack.
plugin:processor("minify", {
    files = { "**/*.json", "**/*.mcmeta" },
    priority = 50,
}, function(ctx, file)
    local ok, data = pcall(rpp.json.decode, file.text)
    if ok then
        file.text = rpp.json.encode(data)
    end
end)

-- Generator: emit a tiny build marker listing the pack name.
plugin:generator("marker", function(ctx)
    local note = string.format("built by rpp: %s", ctx.pack.name)
    ctx:emit("rpp_build.txt", note)
end)

return plugin
"#;
