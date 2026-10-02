//! External PackSquash engine support.

use std::io::Write;
use std::path::Path;
use std::process::Command;

use crate::error::{Error, Result};

/// Run an external PackSquash binary over `pack_dir`, producing `zip_path`.
///
/// PackSquash is driven by an options TOML file passed as its sole CLI argument.
/// If `options_file` is provided it is passed through verbatim (the caller is
/// responsible for its `pack_directory` / `output_file_path` keys). Otherwise a
/// minimal temporary options file is generated setting:
///
/// ```toml
/// pack_directory = "<pack_dir>"
/// output_file_path = "<zip_path>"
/// ```
///
/// # Errors
/// - [`Error::PackSquashNotFound`] if `binary` cannot be spawned (not installed
///   or not on `PATH`).
/// - [`Error::PackSquashFailed`] (with captured stderr) on a non-zero exit.
/// - [`Error::Io`] if the temporary options file cannot be written.
pub fn run_packsquash(
    binary: &str,
    pack_dir: &Path,
    zip_path: &Path,
    options_file: Option<&Path>,
) -> Result<()> {
    // Keeps a generated options file alive for the duration of the call.
    let generated;
    let options_path = match options_file {
        Some(path) => path,
        None => {
            generated = generate_options_file(pack_dir, zip_path)?;
            generated.path()
        }
    };

    let output = Command::new(binary)
        .arg(options_path)
        .output()
        .map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                Error::PackSquashNotFound
            } else {
                Error::io(binary, err)
            }
        })?;

    if output.status.success() {
        Ok(())
    } else {
        Err(Error::PackSquashFailed {
            code: output.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// Write a minimal PackSquash options file to a temp file and return its handle.
fn generate_options_file(pack_dir: &Path, zip_path: &Path) -> Result<tempfile::NamedTempFile> {
    let mut file = tempfile::Builder::new()
        .prefix("rpp-packsquash-")
        .suffix(".toml")
        .tempfile()
        .map_err(|err| Error::io(std::env::temp_dir(), err))?;

    let contents = render_options(pack_dir, zip_path);
    file.write_all(contents.as_bytes())
        .map_err(|err| Error::io(file.path(), err))?;
    file.flush().map_err(|err| Error::io(file.path(), err))?;
    Ok(file)
}

/// Render the minimal options TOML body. TOML basic strings require escaping
/// backslashes and quotes (relevant on Windows paths).
fn render_options(pack_dir: &Path, zip_path: &Path) -> String {
    format!(
        "pack_directory = \"{}\"\noutput_file_path = \"{}\"\n",
        toml_escape(&pack_dir.to_string_lossy()),
        toml_escape(&zip_path.to_string_lossy()),
    )
}

/// Escape a string for use inside a TOML basic string literal.
fn toml_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_binary_errors_clearly() {
        let dir = tempfile::tempdir().unwrap();
        let err = run_packsquash(
            "rpp-definitely-not-a-real-binary",
            dir.path(),
            &dir.path().join("out.zip"),
            None,
        )
        .unwrap_err();
        assert!(matches!(err, Error::PackSquashNotFound));
        assert!(err.to_string().contains("builtin"));
    }

    #[test]
    fn options_file_names_pack_and_output() {
        let body = render_options(Path::new("/tmp/some pack"), Path::new("/tmp/out.zip"));
        assert!(body.contains("pack_directory = \"/tmp/some pack\""));
        assert!(body.contains("output_file_path = \"/tmp/out.zip\""));
    }
}
