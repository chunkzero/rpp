//! `rpp check`: generate definitions, then type-check TypeScript with `tsc`.

use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::commands::codegen::find_root;
use crate::{codegen, ui};

/// Run the check command from `dir`.
pub fn run(dir: &Path) -> Result<()> {
    let root = find_root(dir)?;
    ui::intro("Type-check TypeScript");
    codegen::write(&root)?;

    let compiler = std::env::var_os("RPP_TSC").unwrap_or_else(|| OsString::from("tsc"));
    let status = Command::new(&compiler)
        .arg("-p")
        .arg(root.join("tsconfig.json"))
        .args(["--noEmit", "--pretty"])
        .current_dir(&root)
        .status();
    let status = match status {
        Ok(status) => status,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => bail!(
            "TypeScript compiler `{}` not found; install TypeScript 7+ so `tsc` is on PATH \
             or set RPP_TSC to the compiler path",
            compiler.to_string_lossy()
        ),
        Err(error) => {
            return Err(error).with_context(|| format!("running `{}`", compiler.to_string_lossy()))
        }
    };
    if !status.success() {
        bail!("type checking failed");
    }
    ui::success("No type errors");
    Ok(())
}
