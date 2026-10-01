//! `rpp plugin pack`: bundle a plugin into a distributable archive.

use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Subcommand;

use crate::ui;

mod pack;

pub use pack::Packed;

/// Plugin subcommands.
#[derive(Debug, Subcommand)]
pub enum PluginCommand {
    /// Bundle a plugin into a self-contained `<name>-<version>.rpp.tgz` archive.
    Pack {
        /// The plugin directory (default: the current directory).
        dir: Option<PathBuf>,
        /// Directory for the archive and its `.sha256` file (default: the plugin directory).
        #[arg(long)]
        out: Option<PathBuf>,
        /// Print the result as JSON on stdout.
        #[arg(long)]
        json: bool,
    },
}

/// Run a plugin subcommand from the current directory.
pub fn run(dir: &Path, command: PluginCommand) -> Result<()> {
    match command {
        PluginCommand::Pack {
            dir: plugin_dir,
            out,
            json,
        } => run_pack(dir, plugin_dir.as_deref(), out.as_deref(), json),
    }
}

fn run_pack(dir: &Path, plugin: Option<&Path>, out: Option<&Path>, json: bool) -> Result<()> {
    let plugin = plugin.map_or_else(|| dir.to_path_buf(), |p| dir.join(p));
    let out = out.map_or_else(|| plugin.clone(), |o| dir.join(o));
    let packed = pack::pack(&plugin, &out)?;
    if json {
        println!("{}", serde_json::to_string(&packed)?);
    } else {
        ui::success(format!(
            "Packed {} {} into {} (sha256 {})",
            packed.name,
            packed.version,
            packed.path.display(),
            packed.sha256
        ));
    }
    Ok(())
}
