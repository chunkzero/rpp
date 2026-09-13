use std::path::Path;
use std::process::{Command, Stdio};

pub fn command(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rpp"));
    command
        .current_dir(root)
        .env("RPP_HOME", root.join(".test-rpp-home"))
        .env("RPP_CACHE_DIR", root.join(".test-rpp-cache"))
        .stdin(Stdio::null());
    command
}
