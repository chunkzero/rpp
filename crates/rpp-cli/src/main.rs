use std::{path::Path, time::Instant};

use crate::cli::Cli;
use clap::Parser;

mod cli;
mod plugin;
mod server;

pub const DEFAULT_CONFIG_PATH: &str = "./rpp.jsonc";

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let mut lua = rpp::lua::RppLua::new();
    let p1 = lua.load_plugin(Path::new("./plugin")).unwrap();
    let p2 = lua.load_plugin(Path::new("./plugin2")).unwrap();

    let start_time = Instant::now(); // Record the starting time

    p1.init().unwrap();
    p2.init().unwrap();

    let elapsed_time = start_time.elapsed(); // Calculate the elapsed duration

    println!("Elapsed time: {:?}", elapsed_time);

    Ok(())

    /*
    let cli = Cli::parse();
    cli.command.run()
    */
}
