//! `rpp check`: generate definitions, then type-check TypeScript with `tsc`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::commands::codegen::find_root;
use crate::{codegen, ui};

const TYPESCRIPT_VERSION: &str = "7.0.2";

/// The compiler: `RPP_TSC`, else the one bundled beside the executable, else `tsc` on PATH.
fn compiler() -> OsString {
    if let Some(path) = std::env::var_os("RPP_TSC") {
        return path;
    }
    bundled_compiler().map_or_else(|| OsString::from("tsc"), PathBuf::into_os_string)
}

fn bundled_compiler() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let path = exe
        .parent()?
        .join("toolchain/typescript")
        .join(TYPESCRIPT_VERSION)
        .join("tsc");
    path.is_file().then_some(path)
}

/// Run the check command from `dir`.
pub fn run(dir: &Path) -> Result<()> {
    let root = find_root(dir)?;
    ui::intro("Type-check TypeScript");
    codegen::write(&root)?;

    let compiler = compiler();
    let status = Command::new(&compiler)
        .arg("-p")
        .arg(root.join("tsconfig.json"))
        .args(["--noEmit", "--pretty"])
        .current_dir(&root)
        .status();
    let status = match status {
        Ok(status) => status,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => bail!(
            "TypeScript compiler `{}` not found; reinstall rpp to restore its bundled compiler, \
             install TypeScript 7+ so `tsc` is on PATH, or set RPP_TSC to the compiler path",
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
