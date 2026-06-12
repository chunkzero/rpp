//! Path normalization helpers.
//!
//! All paths flowing through the pipeline are relative and use forward slashes.

use std::path::Path;

/// Validate a portable, relative path used inside a resource pack.
pub(crate) fn validate_relative(path: &str) -> std::result::Result<(), String> {
    if path.is_empty() {
        return Err("path must not be empty".into());
    }
    if path.contains(['\\', '\0', ':']) {
        return Err(format!("path `{path}` contains a forbidden character"));
    }
    if path.starts_with('/') || path.ends_with('/') || path.contains("//") {
        return Err(format!("path `{path}` must be a normalized relative path"));
    }
    if path
        .split('/')
        .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(format!(
            "path `{path}` must not contain `.` or `..` components"
        ));
    }
    Ok(())
}

/// Convert an OS path (relative to a root) into a forward-slash string.
pub(crate) fn to_forward_slash(path: &Path) -> String {
    path.components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::validate_relative;

    #[test]
    fn validates_pack_paths() {
        assert!(validate_relative("assets/minecraft/a.json").is_ok());
        assert!(validate_relative(".rppignore").is_ok());
        for invalid in [
            "",
            "/tmp/file",
            "../file",
            "a/../file",
            "a//file",
            "a\\file",
            "C:/file",
        ] {
            assert!(validate_relative(invalid).is_err(), "{invalid}");
        }
    }
}
