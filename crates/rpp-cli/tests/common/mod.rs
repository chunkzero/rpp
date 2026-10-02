//! Helpers shared by the integration test crates. Each crate uses a subset.
#![allow(dead_code)]

pub mod packed_plugin;

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

pub fn command(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rpp"));
    command
        .current_dir(root)
        .env("RPP_HOME", root.join(".test-rpp-home"))
        .env("RPP_CACHE_DIR", root.join(".test-rpp-cache"))
        .stdin(Stdio::null());
    command
}

/// Runs `rpp <args>` in `root`.
pub fn run(root: &Path, args: &[&str]) -> Output {
    command(root).args(args).output().expect("run rpp")
}

/// Runs `rpp build <args>` in `root`.
pub fn build(root: &Path, args: &[&str]) -> Output {
    command(root)
        .arg("build")
        .args(args)
        .output()
        .expect("run rpp build")
}

pub fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Writes `contents` to `root/rel`, creating parent directories.
pub fn write(root: &Path, rel: &str, contents: impl AsRef<[u8]>) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

pub fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The repository's `examples` directory.
pub fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

/// Recursively copies `from` to `to`, skipping build outputs and guest crates.
pub fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if ["target", ".rpp", "dist", "generated", "guest"]
            .iter()
            .any(|skip| name == *skip)
        {
            continue;
        }
        let target = to.join(&name);
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

pub fn wasip2_available() -> bool {
    let Ok(output) = Command::new("rustc").args(["--print", "sysroot"]).output() else {
        return false;
    };
    Path::new(String::from_utf8_lossy(&output.stdout).trim())
        .join("lib/rustlib/wasm32-wasip2/lib")
        .is_dir()
}

/// Builds the `wasm32-wasip2` guest crate in `crate_dir` into `target_dir` and returns the
/// path of `artifact` (a file name such as `guest.wasm`) in its `release` or `debug` output.
/// `extra_args` are appended to the `cargo build` invocation.
pub fn build_wasm_guest(
    crate_dir: &Path,
    target_dir: &Path,
    artifact: &str,
    release: bool,
    extra_args: &[&str],
) -> PathBuf {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut command = Command::new(cargo);
    command
        .args(["build", "--locked", "--target", "wasm32-wasip2"])
        .args(extra_args)
        .env("CARGO_TARGET_DIR", target_dir)
        .current_dir(crate_dir);
    if release {
        command.arg("--release");
    }
    let output = command.output().expect("build wasm guest");
    assert!(
        output.status.success(),
        "guest build failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let profile = if release { "release" } else { "debug" };
    target_dir
        .join("wasm32-wasip2")
        .join(profile)
        .join(artifact)
}
