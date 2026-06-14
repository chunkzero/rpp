//! User-level plugin manifest and lockfile storage.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use rpp::config::PluginConfig;
use rpp_fetch::Lockfile;
use serde::Deserialize;

pub const USER_MANIFEST_FILE: &str = "plugins.toml";
pub const USER_LOCK_FILE: &str = "plugins.lock";

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct UserPluginManifest {
    #[serde(default, rename = "plugin")]
    plugins: Vec<PluginConfig>,
}

/// The user-level plugin store under `~/.rpp`.
pub struct UserPlugins {
    pub root: PathBuf,
    pub plugins: Vec<PluginConfig>,
}

impl UserPlugins {
    /// Load the user plugin manifest. A missing manifest is empty.
    pub fn load() -> Result<Self> {
        let root = user_root()?;
        let path = root.join(USER_MANIFEST_FILE);
        let plugins = match std::fs::read_to_string(&path) {
            Ok(text) => {
                toml::from_str::<UserPluginManifest>(&text)
                    .with_context(|| format!("parsing {}", path.display()))?
                    .plugins
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => {
                return Err(error).with_context(|| format!("reading {}", path.display()));
            }
        };
        Ok(Self { root, plugins })
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.root.join(USER_MANIFEST_FILE)
    }

    pub fn lock_path(&self) -> PathBuf {
        self.root.join(USER_LOCK_FILE)
    }

    pub fn plugin_dir(&self, id: &str) -> PathBuf {
        self.root.join("plugins").join(id)
    }

    pub fn lockfile(&self) -> Result<Lockfile> {
        Lockfile::load(&self.lock_path()).map_err(Into::into)
    }

    pub fn manifest_text(&self) -> Result<String> {
        let path = self.manifest_path();
        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn ensure_root(&self) -> Result<()> {
        std::fs::create_dir_all(&self.root)
            .with_context(|| format!("creating {}", self.root.display()))
    }
}

fn user_root() -> Result<PathBuf> {
    if let Some(root) = std::env::var_os("RPP_HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(root));
    }
    dirs::home_dir()
        .map(|home| home.join(".rpp"))
        .ok_or_else(|| anyhow!("could not determine the user home directory; set RPP_HOME"))
}

/// Recursively copy an installed directory package into the user plugin store.
pub fn copy_plugin_dir(source: &Path, destination: &Path) -> Result<()> {
    if destination.exists() {
        anyhow::bail!(
            "global plugin directory {} already exists",
            destination.display()
        );
    }
    let parent = destination
        .parent()
        .context("global plugin destination has no parent")?;
    std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;

    let staging = tempfile::Builder::new()
        .prefix(".install-")
        .tempdir_in(parent)
        .with_context(|| format!("creating install staging directory in {}", parent.display()))?;
    copy_dir_contents(source, staging.path())?;
    std::fs::rename(staging.keep(), destination)
        .with_context(|| format!("installing plugin directory {}", destination.display()))?;
    Ok(())
}

fn copy_dir_contents(source: &Path, destination: &Path) -> Result<()> {
    for entry in
        std::fs::read_dir(source).with_context(|| format!("reading {}", source.display()))?
    {
        let entry = entry.with_context(|| format!("reading {}", source.display()))?;
        let name = entry.file_name();
        if matches!(name.to_str(), Some(".git" | ".rpp" | "target")) {
            continue;
        }

        let from = entry.path();
        let to = destination.join(&name);
        let file_type = entry
            .file_type()
            .with_context(|| format!("reading file type for {}", from.display()))?;
        if file_type.is_symlink() {
            anyhow::bail!(
                "directory plugin contains unsupported symlink {}",
                from.display()
            );
        }
        if file_type.is_dir() {
            std::fs::create_dir(&to).with_context(|| format!("creating {}", to.display()))?;
            copy_dir_contents(&from, &to)?;
        } else if file_type.is_file() {
            std::fs::copy(&from, &to)
                .with_context(|| format!("copying {} to {}", from.display(), to.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_package_without_build_artifacts() {
        let source = tempfile::tempdir().unwrap();
        let destination_root = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("plugin.toml"), "[plugin]\n").unwrap();
        std::fs::create_dir(source.path().join("target")).unwrap();
        std::fs::write(source.path().join("target/large"), "ignored").unwrap();

        let destination = destination_root.path().join("plugin");
        copy_plugin_dir(source.path(), &destination).unwrap();

        assert!(destination.join("plugin.toml").is_file());
        assert!(!destination.join("target").exists());
    }
}
