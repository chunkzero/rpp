# Commit 10: CLI Integration and Cleanup

**Goal**: Update CLI to use new build engine, remove old code.

## Files to Modify

- `crates/rpp-cli/src/cli/build.rs`
- `crates/rpp/src/lib.rs`

## Files to Delete

- `crates/rpp/src/compile/` (entire directory)
- `crates/rpp/src/lua/handler.rs`

## Updated `cli/build.rs`

```rust
use std::path::PathBuf;
use clap::Args;
use rpp::build::BuildEngine;

#[derive(Args)]
pub struct BuildCommand {
    /// Source directory
    #[arg(default_value = ".")]
    pub source: PathBuf,

    /// Output directory
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Clean before build
    #[arg(long)]
    pub clean: bool,

    /// Number of worker threads
    #[arg(short = 'j', long)]
    pub jobs: Option<usize>,
}

impl BuildCommand {
    pub fn run(&self) -> anyhow::Result<()> {
        let source = self.source.canonicalize()?;
        let output = self.output.clone().unwrap_or_else(|| source.join("dist"));

        let mut builder = BuildEngine::builder()
            .source_dir(&source)
            .output_dir(&output);

        if let Some(jobs) = self.jobs {
            builder = builder.num_workers(jobs);
        }

        let mut engine = builder.build()?;

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
```

## Add `cli/serve.rs`

```rust
use std::path::PathBuf;
use clap::Args;
use rpp::build::BuildEngine;
use crate::dev_server::{DevServer, ServerConfig};

#[derive(Args)]
pub struct ServeCommand {
    /// Source directory
    #[arg(default_value = ".")]
    pub source: PathBuf,

    /// Output directory
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Host to bind to
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,

    /// Port to listen on
    #[arg(short, long, default_value = "8080")]
    pub port: u16,

    /// Enable plugin hot reload
    #[arg(long)]
    pub hot_reload: bool,
}

impl ServeCommand {
    pub async fn run(&self) -> anyhow::Result<()> {
        let source = self.source.canonicalize()?;
        let output = self.output.clone().unwrap_or_else(|| source.join("dist"));

        let engine = BuildEngine::builder()
            .source_dir(&source)
            .output_dir(&output)
            .build()?;

        let config = ServerConfig {
            host: self.host.clone(),
            port: self.port,
            source_dir: source.clone(),
            output_dir: output,
            plugin_dir: Some(source.join("plugins")),
            hot_reload: self.hot_reload,
        };

        let mut server = DevServer::new(engine, config);
        server.run().await
    }
}
```

## Update `cli/mod.rs`

```rust
mod build;
mod serve;

pub use build::BuildCommand;
pub use serve::ServeCommand;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "rpp")]
#[command(about = "Resource Pack Processor")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Build a resource pack
    Build(BuildCommand),
    /// Start development server with hot reload
    Serve(ServeCommand),
}
```

## Update `main.rs`

```rust
use clap::Parser;
use tracing_subscriber::EnvFilter;

mod cli;
mod dev_server;

use cli::{Cli, Commands};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Build(cmd) => cmd.run(),
        Commands::Serve(cmd) => cmd.run().await,
    }
}
```

## Update `lib.rs` (rpp crate)

```rust
//! RPP - Resource Pack Processor
//!
//! A multi-phase build pipeline with sandboxed Lua plugins.

pub mod build;
pub mod lua;
pub mod plugin;
pub mod sandbox;
pub mod worker;

// Re-export commonly used types
pub use build::{BuildEngine, BuildEngineBuilder, BuildResult, BuildError};
pub use plugin::{
    Plugin, ProcessorPlugin, GeneratorPlugin,
    ProcessResult, ProcessingContext, GeneratorContext,
    PluginRegistry, LuaProcessor,
};
```

## Migration Checklist

### Remove Old Code

```bash
# Remove old compile module
rm -rf crates/rpp/src/compile/

# Remove old Lua handler
rm crates/rpp/src/lua/handler.rs
```

### Update Cargo.toml

Ensure all new dependencies are added:

```toml
# crates/rpp/Cargo.toml
[dependencies]
thiserror = "2.0"
sha2 = "0.10"
md5 = "0.7"
glob = "0.3"
crossbeam-channel = "0.5"
ignore = "0.4"
bincode = "2.0"
mlua = { version = "0.11", features = ["luajit52", "vendored"] }
twox-hash = "2.0"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
toml = "0.9"
tracing = "0.1"

# crates/rpp-cli/Cargo.toml
[dependencies]
rpp = { path = "../rpp" }
clap = { version = "4.5", features = ["derive"] }
tokio = { version = "1.0", features = ["full"] }
anyhow = "1.0"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
viz = { version = "0.10", features = ["sse"] }
notify = "7.0"
mime_guess = "2.0"
futures-util = "0.3"
tokio-stream = "0.1"
```

## Verification

```bash
# Check everything compiles
cargo check --workspace

# Run tests
cargo test --workspace

# Test build command
cargo run -p rpp-cli -- build ./examples/sample_pack

# Test serve command
cargo run -p rpp-cli -- serve ./examples/sample_pack
```

## Example Usage

```bash
# Simple build
rpp build ./my_pack

# Build with custom output
rpp build ./my_pack --output ./dist

# Build with 8 workers
rpp build ./my_pack -j 8

# Clean and rebuild
rpp build ./my_pack --clean

# Start dev server
rpp serve ./my_pack

# Dev server with custom port
rpp serve ./my_pack --port 3000

# Dev server with hot reload
rpp serve ./my_pack --hot-reload
```
