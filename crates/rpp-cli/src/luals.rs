//! Generation of LuaLS / LuaCATS type-definition stubs for the rpp Lua plugin
//! API, written to `.rpp/api/*.lua`.
//!
//! These files give editors (via the LuaLS language server) autocomplete and
//! type information for the `rpp` module, the plugin builder, the `file`
//! object, and the processor/generator `ctx` tables. They are pure annotation
//! stubs (never `require`d at runtime) and exactly mirror the runtime surface
//! in `crates/rpp/src/lua/`.
//!
//! The first line of every generated file carries a [`API_VERSION`] marker so
//! `build`/`dev` can detect stale defs and refresh them.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Context, Result};

/// Bump this whenever the generated API stubs change. Embedded as a marker
/// comment so stale definitions can be detected and refreshed.
pub const API_VERSION: u32 = 2;

const MARKER_PREFIX: &str = "---@meta rpp-api v";

/// The generated definition files: `(filename, contents)`.
fn files() -> Vec<(&'static str, String)> {
    vec![
        ("rpp.lua", rpp_module()),
        ("file.lua", file_and_ctx()),
        ("config.json", luarc_hint()),
    ]
}

/// Write (or refresh) the LuaLS definitions under `<api_dir>` if missing or
/// stale. Returns `true` if any file was (re)written.
pub fn write_if_stale(api_dir: &Path) -> Result<bool> {
    let mut wrote = false;
    std::fs::create_dir_all(api_dir).with_context(|| format!("creating {}", api_dir.display()))?;
    for (name, contents) in files() {
        let path = api_dir.join(name);
        let stale = match std::fs::read_to_string(&path) {
            Ok(existing) => !existing.contains(&format!("{MARKER_PREFIX}{API_VERSION}")),
            Err(_) => true,
        };
        if stale {
            std::fs::write(&path, &contents)
                .with_context(|| format!("writing {}", path.display()))?;
            wrote = true;
        }
    }
    Ok(wrote)
}

/// Copy LuaLS definitions shipped by a plugin package from `luals/*.lua` into
/// the project API directory. A plugin stub `luals/window.lua` becomes
/// `.rpp/api/window.lua`, making `require("window")` typed in pack-authored Lua
/// sources. `owners` maps stub filenames to the plugin that provided them so a
/// second plugin shipping the same filename is rejected instead of clobbering.
pub fn write_plugin_stubs(
    api_dir: &Path,
    plugin_id: &str,
    plugin_root: &Path,
    owners: &mut BTreeMap<String, String>,
) -> Result<bool> {
    let luals_dir = plugin_root.join("luals");
    if !luals_dir.is_dir() {
        return Ok(false);
    }

    std::fs::create_dir_all(api_dir).with_context(|| format!("creating {}", api_dir.display()))?;
    let mut wrote = false;
    for entry in
        std::fs::read_dir(&luals_dir).with_context(|| format!("reading {}", luals_dir.display()))?
    {
        let entry = entry.with_context(|| format!("reading {}", luals_dir.display()))?;
        let path = entry.path();
        if !entry
            .file_type()
            .with_context(|| format!("reading file type for {}", path.display()))?
            .is_file()
        {
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("lua") {
            continue;
        }

        let name = entry.file_name().to_string_lossy().into_owned();
        if files().iter().any(|(generated, _)| *generated == name) {
            bail!("`luals/{name}` collides with a generated rpp definition file");
        }
        if let Some(owner) = owners.insert(name.clone(), plugin_id.to_string()) {
            if owner != plugin_id {
                bail!("`luals/{name}` is also provided by plugin `{owner}`");
            }
        }
        let contents =
            std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let target = api_dir.join(&name);
        let stale = match std::fs::read(&target) {
            Ok(existing) => existing != contents,
            Err(_) => true,
        };
        if stale {
            std::fs::write(&target, contents)
                .with_context(|| format!("writing {}", target.display()))?;
            wrote = true;
        }
    }
    Ok(wrote)
}

/// Create all definition files without overwriting existing paths.
pub fn write_all_new(api_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(api_dir).with_context(|| format!("creating {}", api_dir.display()))?;
    for (name, contents) in files() {
        let path = api_dir.join(name);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| format!("creating {}", path.display()))?;
        std::io::Write::write_all(&mut file, contents.as_bytes())
            .with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

fn header(extra: &str) -> String {
    format!(
        "{MARKER_PREFIX}{API_VERSION}\n\
         -- Auto-generated rpp Lua API definitions. Do not edit; refreshed by\n\
         -- `rpp build`/`rpp dev`. {extra}\n\n"
    )
}

fn rpp_module() -> String {
    let mut s = header("Provides `require(\"rpp\")` and submodules.");
    s.push_str(
        r#"---@class rpp.PackInfo
---@field name string
---@field description string|nil
---@field format integer|nil

---@class rpp.Log
---@field debug fun(...) Log at debug level (args space-joined).
---@field info  fun(...) Log at info level.
---@field warn  fun(...) Log at warn level.
---@field error fun(...) Log at error level.

---@class rpp.JsonOpts
---@field pretty boolean|nil Pretty-print with indentation (default false).

---@class rpp.Json
local Json = {}
---@param str string
---@return any value
function Json.decode(str) end
---@param value any
---@param opts rpp.JsonOpts|nil
---@return string
function Json.encode(value, opts) end

---@class rpp.Toml
local Toml = {}
---@param str string
---@return any value
function Toml.decode(str) end
---@param value any
---@return string
function Toml.encode(value) end

---@class rpp.Hash
local Hash = {}
---@param s string
---@return string hex
function Hash.xxh3(s) end
---@param s string
---@return string hex
function Hash.sha256(s) end
---@param s string
---@return string hex
function Hash.md5(s) end
---@param s string
---@return integer
function Hash.crc32(s) end

---@class rpp.Path
local Path = {}
---@param ... string
---@return string
function Path.join(...) end
---@param p string
---@return string
function Path.dirname(p) end
---@param p string
---@return string
function Path.basename(p) end
---@param p string
---@return string ext file extension (without the dot)
function Path.ext(p) end
---@param p string
---@param e string
---@return string
function Path.with_ext(p, e) end
---@param glob string
---@param p string
---@return boolean
function Path.match(glob, p) end

---@class rpp.Str
local Str = {}
---@param s string
---@param prefix string
---@return boolean
function Str.starts_with(s, prefix) end
---@param s string
---@param suffix string
---@return boolean
function Str.ends_with(s, suffix) end
---@param s string
---@param sep string
---@return string[]
function Str.split(s, sep) end
---@param s string
---@return string
function Str.trim(s) end

---@class rpp.ComponentHandle
local ComponentHandle = {}
--- Call one exported WIT function. Generated bindgen wrappers provide precise
--- parameter and return types for each export.
---@param export_path string
---@param ... any
---@return any ...
function ComponentHandle.call(self, export_path, ...) end
--- List the component's flattened export paths.
---@return string[]
function ComponentHandle.schema(self) end

---@class rpp.Component
local Component = {}
--- Load a component declared as `[component.<name>]` in plugin.toml.
---@param name string
---@return rpp.ComponentHandle
function Component.load(name) end

---@class rpp.ProcessRequest
---@field program string Executable name or path allowed by plugin permissions.
---@field args string[]|nil
---@field cwd string|nil
---@field env table<string, string>|nil
---@field stdin string|nil Binary-safe standard input.
---@field timeout number|nil Timeout in seconds.

---@class rpp.ProcessOutput
---@field status integer
---@field stdout string Binary-safe captured standard output.
---@field stderr string Binary-safe captured standard error.

---@class rpp.Process
local Process = {}
---@param request rpp.ProcessRequest
---@return rpp.ProcessOutput
function Process.run(request) end

---@class rpp.ProcessorOpts
---@field files string|string[] Glob pattern(s) selecting matching files. Required.
---@field priority integer|nil Lower runs first (default 0).

---@class rpp.Plugin
local Plugin = {}
--- Register a parallel, per-file processor. The callback is pure: it sees only
--- `(ctx, file)` and mutates `file` in place.
---@param name string
---@param opts rpp.ProcessorOpts
---@param fn fun(ctx: rpp.ProcessorCtx, file: rpp.File)
---@return rpp.Plugin self
function Plugin.processor(self, name, opts, fn) end
--- Register the (single) sequential generator, run after all processing.
---@param name string
---@param fn fun(ctx: rpp.GeneratorCtx)
---@return rpp.Plugin self
function Plugin.generator(self, name, fn) end
--- Register a hook fired before processing begins.
---@param fn fun(ctx: rpp.ProcessorCtx)
---@return rpp.Plugin self
function Plugin.on_start(self, fn) end
--- Register a hook fired after the build completes.
---@param fn fun(ctx: rpp.ProcessorCtx, stats: rpp.BuildStats)
---@return rpp.Plugin self
function Plugin.on_finish(self, fn) end

---@class rpp.BuildStats
---@field processed integer
---@field cached integer
---@field generated integer
---@field dropped integer

---@class rpp
---@field json rpp.Json
---@field toml rpp.Toml
---@field hash rpp.Hash
---@field path rpp.Path
---@field log  rpp.Log
---@field str  rpp.Str
---@field component rpp.Component
---@field process rpp.Process
local rpp = {}
--- Create a new plugin builder.
---@return rpp.Plugin
function rpp.plugin() end

return rpp
"#,
    );
    s
}

fn file_and_ctx() -> String {
    let mut s = header("The `file` object and processor/generator `ctx` tables.");
    s.push_str(
        r#"---@class rpp.File
--- The output path (relative, forward-slash). Assigning renames the output.
---@field path string
--- File contents as a Lua string (binary-safe). Alias of `text`.
---@field bytes string
--- File contents as a Lua string. Alias of `bytes`.
---@field text string
local File = {}
--- Exclude this file from output and stop the processor chain.
function File.drop(self) end

---@class rpp.ProcessorCtx
--- Plugin options (`[plugin.options]` from rpp.toml) as a Lua table.
---@field options table
---@field pack rpp.PackInfo
---@field log rpp.Log

---@class rpp.GeneratorCtx : rpp.ProcessorCtx
local GeneratorCtx = {}
--- List processed output files matching an optional glob.
---@param glob string|nil
---@return string[]
function GeneratorCtx.files(self, glob) end
--- List raw source files matching an optional glob.
---@param glob string|nil
---@return string[]
function GeneratorCtx.source_files(self, glob) end
--- Read a processed output file. Returns nil if absent.
---@param path string
---@return string|nil
function GeneratorCtx.read(self, path) end
--- Read a raw source file. Returns nil if absent.
---@param path string
---@return string|nil
function GeneratorCtx.read_source(self, path) end
--- Load and evaluate a UTF-8 Lua authoring source in the plugin environment.
--- The source contents are recorded as a generator dependency.
---@param path string
---@return any
function GeneratorCtx.load_source(self, path) end
--- Add or overwrite an output file.
---@param path string
---@param contents string
function GeneratorCtx.emit(self, path, contents) end
--- Emit a non-pack artifact into a named root declared by `[plugin.outputs]`.
---@param root string
---@param path string
---@param contents string
function GeneratorCtx.emit_output(self, root, path, contents) end
--- Drop an output file.
---@param path string
function GeneratorCtx.remove(self, path) end
"#,
    );
    s
}

/// A `.luarc.json`-style hint file the editor can pick up so it points the
/// language server at these stubs. Written as JSON (the `config.json` name keeps
/// it out of the Lua module search).
fn luarc_hint() -> String {
    format!(
        "{{\n  \"_comment\": \"rpp-api v{API_VERSION}: add `.rpp/api` to \
         `Lua.workspace.library` in your .luarc.json for autocomplete.\",\n  \
         \"Lua.workspace.library\": [\".rpp/api\"],\n  \
         \"Lua.runtime.version\": \"Lua 5.4\"\n}}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definitions_cover_component_and_external_codegen_apis() {
        let module = rpp_module();
        assert!(module.contains("---@field component rpp.Component"));
        assert!(module.contains("function Component.load(name) end"));

        let context = file_and_ctx();
        assert!(context.contains("function GeneratorCtx.load_source(self, path) end"));
        assert!(context.contains("function GeneratorCtx.emit_output(self, root, path, contents)"));
    }
}
