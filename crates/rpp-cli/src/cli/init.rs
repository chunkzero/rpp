use anyhow::Result;
use clap::Args;
use dialoguer::{theme::ColorfulTheme, Input, Select};
use std::fs;
use std::path::PathBuf;

#[derive(Args)]
pub struct InitCommand {
    /// Project directory (defaults to current directory)
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Skip interactive prompts and use defaults
    #[arg(short, long)]
    pub yes: bool,

    /// Template to use (minimal, basic, full)
    #[arg(short, long)]
    pub template: Option<String>,
}

impl InitCommand {
    pub fn run(&self) -> Result<()> {
        let target_dir = &self.path;

        // Check if directory is empty
        if target_dir.exists() {
            let entries: Vec<_> = fs::read_dir(target_dir)?
                .filter_map(|e| e.ok())
                .collect();

            if !entries.is_empty() && !self.yes {
                let confirm = dialoguer::Confirm::with_theme(&ColorfulTheme::default())
                    .with_prompt("Directory is not empty. Continue?")
                    .interact()?;

                if !confirm {
                    return Ok(());
                }
            }
        } else {
            fs::create_dir_all(target_dir)?;
        }

        // Get project name from directory name or prompt
        let default_name = target_dir
            .canonicalize()?
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("rpp-project")
            .to_string();

        let name = if self.yes {
            default_name
        } else {
            Input::with_theme(&ColorfulTheme::default())
                .with_prompt("Project name")
                .default(default_name)
                .interact_text()?
        };

        let description = if self.yes {
            "My RPP resource pack".to_string()
        } else {
            Input::with_theme(&ColorfulTheme::default())
                .with_prompt("Description")
                .default("My RPP resource pack".to_string())
                .interact_text()?
        };

        // Select template
        let template = if let Some(ref t) = self.template {
            match t.as_str() {
                "minimal" => Template::Minimal,
                "basic" => Template::Basic,
                "full" => Template::Full,
                _ => anyhow::bail!("Unknown template: {}. Use minimal, basic, or full.", t),
            }
        } else if self.yes {
            Template::Minimal
        } else {
            let templates = vec![
                ("minimal", "Minimal - Just structure and config"),
                ("basic", "Basic - With one example plugin"),
                ("full", "Full - Complete demo pack with multiple plugins"),
            ];

            let selection = Select::with_theme(&ColorfulTheme::default())
                .with_prompt("Choose a template")
                .items(&templates.iter().map(|(_, desc)| *desc).collect::<Vec<_>>())
                .default(0)
                .interact()?;

            match selection {
                0 => Template::Minimal,
                1 => Template::Basic,
                2 => Template::Full,
                _ => Template::Minimal,
            }
        };

        tracing::info!("Initializing RPP project: {}", name);
        tracing::info!("Template: {:?}", template);

        // Create directory structure
        self.create_structure(target_dir, &name, &description, template)?;

        // Generate LuaLS definitions
        let lua_defs_path = target_dir.join(".rpp/lua");
        tracing::info!("Generating LuaLS definitions...");
        crate::lua_defs::generate_lua_definitions(&lua_defs_path)?;

        println!("\n✓ Project initialized successfully!");
        println!("\nNext steps:");
        println!("  1. cd {}", target_dir.display());
        println!("  2. rpp build      # Build your resource pack");
        println!("  3. rpp dev        # Start dev server with hot reload");

        Ok(())
    }

    fn create_structure(
        &self,
        target_dir: &PathBuf,
        _name: &str,
        description: &str,
        template: Template,
    ) -> Result<()> {
        // Create rpp.jsonc
        self.create_config(target_dir, description)?;

        // Create .luarc.json
        self.create_luarc(target_dir)?;

        // Create source directory
        let src_dir = target_dir.join("src");
        fs::create_dir_all(&src_dir)?;

        // Create pack.mcmeta
        self.create_pack_mcmeta(&src_dir, description)?;

        // Create plugins directory
        let plugins_dir = target_dir.join("plugins");
        fs::create_dir_all(&plugins_dir)?;

        // Add template-specific files
        match template {
            Template::Minimal => {
                // Minimal has no extra files
            }
            Template::Basic => {
                self.create_basic_template(target_dir)?;
            }
            Template::Full => {
                self.create_full_template(target_dir)?;
            }
        }

        // Create .gitignore
        self.create_gitignore(target_dir)?;

        Ok(())
    }

    fn create_config(&self, target_dir: &PathBuf, _description: &str) -> Result<()> {
        let config = format!(
            r#"{{
  // Source directory containing pack contents
  "sourceDir": "src",

  // Dev server configuration
  "devServer": {{
    "host": "127.0.0.1",
    "port": 8080,
    "hotReload": true
  }},

  // Plugin repositories (for future use)
  "pluginRepositories": []
}}
"#
        );

        fs::write(target_dir.join("rpp.jsonc"), config)?;
        Ok(())
    }

    fn create_luarc(&self, target_dir: &PathBuf) -> Result<()> {
        let luarc = r#"{
  "runtime.version": "LuaJIT",
  "workspace.library": [".rpp/lua"],
  "diagnostics.globals": ["json", "hash", "log"]
}
"#;

        fs::write(target_dir.join(".luarc.json"), luarc)?;
        Ok(())
    }

    fn create_pack_mcmeta(&self, src_dir: &PathBuf, description: &str) -> Result<()> {
        let pack_mcmeta = format!(
            r#"{{
  "pack": {{
    "pack_format": 48,
    "description": "{}"
  }}
}}
"#,
            description
        );

        fs::write(src_dir.join("pack.mcmeta"), pack_mcmeta)?;
        Ok(())
    }

    fn create_gitignore(&self, target_dir: &PathBuf) -> Result<()> {
        let gitignore = r#"# RPP build artifacts
.rpp/build/
.rpp/cache/

# Keep LuaLS definitions
!.rpp/lua/
"#;

        fs::write(target_dir.join(".gitignore"), gitignore)?;
        Ok(())
    }

    fn create_basic_template(&self, target_dir: &PathBuf) -> Result<()> {
        // Create example plugin
        let example_plugin = r#"return {
  name = "example",
  version = "1.0.0",
  patterns = { "**/*.json" },
  priority = 100,

  process = function(ctx, input)
    ctx.log.info("Processing: " .. input.path)

    -- Example: minify JSON
    local data = ctx.json.decode(input.content)
    local minified = ctx.json.encode_compact(data)

    return {
      action = "continue",
      content = minified
    }
  end
}
"#;

        fs::write(target_dir.join("plugins/example.lua"), example_plugin)?;

        // Create README
        let readme = r#"# RPP Project

A resource pack built with RPP (Resource Pack Processor).

## Structure

- `src/` - Source files for your resource pack
- `plugins/` - Lua plugins for processing files
- `.rpp/` - Build artifacts and cache

## Commands

- `rpp build` - Build the resource pack
- `rpp dev` - Start development server with hot reload

## Plugins

This project includes an example plugin that minifies JSON files.
Edit `plugins/example.lua` to customize or add your own plugins.
"#;

        fs::write(target_dir.join("README.md"), readme)?;

        Ok(())
    }

    fn create_full_template(&self, target_dir: &PathBuf) -> Result<()> {
        // First create basic template
        self.create_basic_template(target_dir)?;

        // Add hash renamer plugin
        let hash_renamer = r#"return {
  name = "hash_renamer",
  version = "1.0.0",
  patterns = { "**/*.png", "**/*.jpg" },
  priority = 50,

  process = function(ctx, input)
    -- Generate hash of content
    local hash = ctx.hash.sha256(input.content)
    local short_hash = string.sub(hash, 1, 8)

    -- Get file extension
    local ext = input.path:match("%.([^%.]+)$") or ""

    -- New filename with hash
    local new_name = short_hash .. "." .. ext

    ctx.log.info("Renaming " .. input.path .. " -> " .. new_name)

    return {
      action = "continue",
      content = input.content,
      path = new_name
    }
  end
}
"#;

        fs::write(target_dir.join("plugins/hash_renamer.lua"), hash_renamer)?;

        // Add sample asset
        let src_dir = target_dir.join("src");
        fs::create_dir_all(src_dir.join("assets/minecraft/textures"))?;

        // Create a simple test JSON file
        let test_json = r#"{
  "test": "This will be minified",
  "nested": {
    "value": 42
  }
}
"#;
        fs::write(
            src_dir.join("assets/minecraft/test.json"),
            test_json,
        )?;

        // Update README with more details
        let readme = r#"# RPP Project

A complete resource pack built with RPP (Resource Pack Processor).

## Structure

- `src/` - Source files for your resource pack
  - `pack.mcmeta` - Pack metadata
  - `assets/` - Resource pack assets
- `plugins/` - Lua plugins for processing files
  - `example.lua` - JSON minifier
  - `hash_renamer.lua` - Content-based file renaming
- `.rpp/` - Build artifacts and cache
  - `build/` - Output directory
  - `cache/` - Build cache
  - `lua/` - LuaLS type definitions

## Commands

- `rpp build` - Build the resource pack
- `rpp dev` - Start development server with hot reload

## Plugins

### example.lua
Minifies all JSON files for smaller pack size.

### hash_renamer.lua
Renames PNG/JPG files based on their content hash.
Useful for cache busting and deduplication.

## LuaLS Integration

This project includes type definitions for the RPP Lua API in `.rpp/lua/`.
Open the project in VSCode with the Lua extension for autocomplete.

## Learn More

Edit the plugins to customize processing or create your own!
The RPP API provides:
- `ctx.json` - JSON encoding/decoding
- `ctx.hash` - Hashing functions (xxhash3, sha256, md5)
- `ctx.log` - Logging (debug, info, warn, error)
"#;

        fs::write(target_dir.join("README.md"), readme)?;

        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
enum Template {
    Minimal,
    Basic,
    Full,
}
