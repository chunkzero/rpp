use crate::dev_server::{DevServer, ServerConfig};
use clap::Args;
use rpp::build::BuildEngine;
use rpp::RppConfig;
use std::path::PathBuf;

#[derive(Args)]
pub struct ServeCommand {
    /// Source directory (defaults to config or "src")
    #[arg(short, long)]
    pub source: Option<PathBuf>,

    /// Output directory (defaults to .rpp/build)
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Cache directory (defaults to .rpp/cache)
    #[arg(short, long)]
    pub cache: Option<PathBuf>,

    /// Host to bind to
    #[arg(long)]
    pub host: Option<String>,

    /// Port to listen on
    #[arg(short, long)]
    pub port: Option<u16>,

    /// Enable plugin hot reload
    #[arg(long)]
    pub hot_reload: Option<bool>,
}

impl ServeCommand {
    pub async fn run(&self) -> anyhow::Result<()> {
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

        let engine = builder.build()?;

        // Determine server configuration: CLI args > config > defaults
        let host = self.host.clone().or_else(|| {
            config.as_ref().map(|c| c.dev_server.host.clone())
        }).unwrap_or_else(|| "127.0.0.1".to_string());

        let port = self.port.or_else(|| {
            config.as_ref().map(|c| c.dev_server.port)
        }).unwrap_or(8080);

        let hot_reload = self.hot_reload.or_else(|| {
            config.as_ref().map(|c| c.dev_server.hot_reload)
        }).unwrap_or(true);

        let output_dir = self.output.clone().unwrap_or_else(|| PathBuf::from(".rpp/build"));

        let server_config = ServerConfig {
            host,
            port,
            source_dir: source.clone(),
            output_dir,
            plugin_dir: Some(PathBuf::from("plugins")),
            hot_reload,
        };

        let mut server = DevServer::new(engine, server_config);
        server.run().await
    }
}
