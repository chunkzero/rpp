use clap::Args;
use rpp::build::BuildEngine;
use rpp::plugin::LuaProcessor;
use rpp::RppConfig;
use std::path::PathBuf;

#[derive(Args)]
pub struct BuildCommand {
    /// Source directory (defaults to config or "src")
    #[arg(short, long)]
    pub source: Option<PathBuf>,

    /// Output directory (defaults to .rpp/build)
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Cache directory (defaults to .rpp/cache)
    #[arg(short, long)]
    pub cache: Option<PathBuf>,

    /// Clean before build
    #[arg(long)]
    pub clean: bool,

    /// Number of worker threads
    #[arg(short = 'j', long)]
    pub jobs: Option<usize>,
}

impl BuildCommand {
    pub fn run(&self) -> anyhow::Result<()> {
        // Load config if exists
        let config = RppConfig::load_from_current_dir()?;

        // Determine source directory: CLI arg > config > default "src"
        let source = if let Some(ref s) = self.source {
            s.clone()
        } else if let Some(ref cfg) = config {
            PathBuf::from(&cfg.source_dir)
        } else {
            PathBuf::from("src")
        };

        let source = source.canonicalize()?;

        // Build the engine with appropriate defaults
        let mut builder = BuildEngine::builder().source_dir(&source);

        // Output directory: CLI arg > default (.rpp/build)
        if let Some(ref output) = self.output {
            builder = builder.output_dir(output);
        }

        // Cache directory: CLI arg > default (.rpp/cache)
        if let Some(ref cache) = self.cache {
            builder = builder.cache_dir(cache);
        }

        if let Some(jobs) = self.jobs {
            builder = builder.num_workers(jobs);
        }

        let mut engine = builder.build()?;

        // Generate LuaLS definitions if they don't exist
        let lua_defs_path = PathBuf::from(".rpp/lua");
        if !lua_defs_path.exists() {
            tracing::info!("Generating LuaLS definitions...");
            crate::lua_defs::generate_lua_definitions(&lua_defs_path)?;
        }

        // Load Lua plugins from plugins/ directory in pack root (not source dir)
        let plugin_dir = PathBuf::from("plugins");
        if plugin_dir.exists() {
            tracing::info!("Loading plugins from {}...", plugin_dir.display());
            load_lua_plugins(&mut engine, &plugin_dir)?;
        }

        if self.clean {
            tracing::info!("Cleaning output directory...");
            engine.clean()?;
        }

        tracing::info!("Building {}...", source.display());
        let result = engine.build()?;

        tracing::info!(
            "Build complete: {} processed, {} cached, {} generated, {} cancelled ({:.2?})",
            result.files_processed,
            result.files_cached,
            result.files_generated,
            result.files_cancelled,
            result.duration
        );

        Ok(())
    }
}

fn load_lua_plugins(engine: &mut BuildEngine, plugin_dir: &PathBuf) -> anyhow::Result<()> {
    use std::fs;

    for entry in fs::read_dir(plugin_dir)? {
        let entry = entry?;
        let path = entry.path();

        // Only load .lua files
        if path.extension().and_then(|s| s.to_str()) != Some("lua") {
            continue;
        }

        let filename = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");

        tracing::info!("  Loading plugin: {}", filename);

        let source = fs::read_to_string(&path)?;

        // Parse plugin metadata from Lua source
        let (name, version, patterns, priority) = parse_plugin_metadata(&source)?;

        tracing::info!(
            "    {} v{} (priority: {}, patterns: {:?})",
            name,
            version,
            priority,
            patterns
        );

        // Create LuaProcessor with parsed metadata
        let processor = LuaProcessor::new(name.clone(), version, patterns, priority, source);

        engine.register_processor(processor)?;
        tracing::info!("    ✓ Registered: {}", name);
    }

    Ok(())
}

fn parse_plugin_metadata(source: &str) -> anyhow::Result<(String, String, Vec<String>, i32)> {
    // Use sandboxed Lua runtime for parsing metadata
    let runtime = rpp::lua::LuaRuntime::new()?;

    // Create a safe parsing function using the sandboxed Lua instance
    let (name, version, patterns, priority) = runtime.with_lua(|lua| {
        let plugin_table: mlua::Table = lua.load(source).eval()?;

        let name: String = plugin_table.get("name")?;
        let version: String = plugin_table.get("version")?;
        let patterns: Vec<String> = plugin_table.get("patterns")?;
        let priority: i32 = plugin_table.get("priority").unwrap_or(100);

        Ok::<_, mlua::Error>((name, version, patterns, priority))
    })?;

    Ok((name, version, patterns, priority))
}
