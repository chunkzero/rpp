use crate::dev_server::{DevServer, ServerConfig};
use clap::Args;
use rpp::build::BuildEngine;
use std::path::PathBuf;

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
