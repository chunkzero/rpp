//! User-level plugin manifest and lockfile storage.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use rpp::config::PluginConfig;
use rpp_fetch::Lockfile;

pub const USER_MANIFEST_FILE: &str = "plugins.toml";
pub const USER_LOCK_FILE: &str = "plugins.lock";

/// The user-level plugin store under `~/.rpp`.
#[derive(Default)]
pub struct UserPlugins {
    pub root: PathBuf,
    pub plugins: Vec<PluginConfig>,
    /// For directory installs, the directory each entry (keyed by its
    /// `source`) was copied from, so `plugin update --global` can refresh it.
    pub origins: BTreeMap<String, PathBuf>,
}

impl UserPlugins {
    /// Load the user plugin manifest. A missing manifest is empty.
    pub fn load() -> Result<Self> {
        let root = user_root()?;
        let path = root.join(USER_MANIFEST_FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(error).with_context(|| format!("reading {}", path.display()));
            }
        };
        let (plugins, origins) =
            parse_manifest(&text).with_context(|| format!("parsing {}", path.display()))?;
        Ok(Self {
            root,
            plugins,
            origins,
        })
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

/// Parse `plugins.toml`: `[[plugin]]` entries are project plugin entries plus
/// an optional `origin` key recorded by directory installs.
fn parse_manifest(text: &str) -> Result<(Vec<PluginConfig>, BTreeMap<String, PathBuf>)> {
    let mut table: toml::Table = toml::from_str(text)?;
    let entries = match table.remove("plugin") {
        Some(toml::Value::Array(entries)) => entries,
        Some(_) => anyhow::bail!("`plugin` must be an array of tables"),
        None => Vec::new(),
    };
    if let Some(unknown) = table.keys().next() {
        anyhow::bail!("unknown key `{unknown}`");
    }
    let mut plugins = Vec::with_capacity(entries.len());
    let mut origins = BTreeMap::new();
    for entry in entries {
        let toml::Value::Table(mut entry) = entry else {
            anyhow::bail!("`[[plugin]]` entries must be tables");
        };
        let origin = match entry.remove("origin") {
            Some(toml::Value::String(origin)) => Some(PathBuf::from(origin)),
            Some(_) => anyhow::bail!("plugin `origin` must be a string"),
            None => None,
        };
        let plugin: PluginConfig = toml::Value::Table(entry).try_into()?;
        if let (Some(origin), Some(source)) = (origin, plugin.source.as_deref()) {
            origins.insert(source.to_string(), origin);
        }
        plugins.push(plugin);
    }
    Ok((plugins, origins))
}

fn user_root() -> Result<PathBuf> {
    if let Some(root) = std::env::var_os("RPP_HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(root));
    }
    dirs::home_dir()
        .map(|home| home.join(".rpp"))
        .ok_or_else(|| anyhow!("could not determine the user home directory; set RPP_HOME"))
}

/// Recursively copy a directory package into the user plugin store, replacing
/// any previous install of the same plugin.
pub fn copy_plugin_dir(source: &Path, destination: &Path) -> Result<()> {
    let parent = destination
        .parent()
        .context("global plugin destination has no parent")?;
    std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;

    let staging = tempfile::Builder::new()
        .prefix(".install-")
        .tempdir_in(parent)
        .with_context(|| format!("creating install staging directory in {}", parent.display()))?;
    copy_dir_contents(source, staging.path())?;

    let previous = destination.exists().then(|| {
        tempfile::Builder::new()
            .prefix(".replaced-")
            .tempdir_in(parent)
    });
    let previous = match previous {
        Some(previous) => {
            let previous = previous
                .with_context(|| format!("creating replacement staging in {}", parent.display()))?;
            let old = previous.path().join("old");
            std::fs::rename(destination, &old)
                .with_context(|| format!("moving aside {}", destination.display()))?;
            Some(previous)
        }
        None => None,
    };
    install_staged(staging.path(), destination, previous)
}

fn install_staged(
    staging: &Path,
    destination: &Path,
    mut previous: Option<tempfile::TempDir>,
) -> Result<()> {
    if let Err(install_error) = std::fs::rename(staging, destination) {
        if let Some(old) = previous
            .as_ref()
            .map(|previous| previous.path().join("old"))
        {
            if let Err(rollback_error) = std::fs::rename(&old, destination) {
                let backup = previous.take().expect("previous install exists").keep();
                return Err(install_error).with_context(|| {
                    format!(
                        "installing {}; restoring the previous install also failed ({rollback_error}); backup retained at {}",
                        destination.display(),
                        backup.join("old").display()
                    )
                });
            }
        }
        return Err(install_error)
            .with_context(|| format!("installing plugin directory {}", destination.display()));
    }
    drop(previous);
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

        // Reinstalling replaces the previous copy.
        std::fs::write(source.path().join("plugin.toml"), "[plugin]\n# v2\n").unwrap();
        copy_plugin_dir(source.path(), &destination).unwrap();
        assert!(std::fs::read_to_string(destination.join("plugin.toml"))
            .unwrap()
            .contains("v2"));
    }

    #[test]
    fn failed_install_restores_previous_package() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("plugin");
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(destination.join("plugin.toml"), "old").unwrap();

        let previous = tempfile::Builder::new()
            .prefix(".replaced-")
            .tempdir_in(root.path())
            .unwrap();
        std::fs::rename(&destination, previous.path().join("old")).unwrap();

        assert!(
            install_staged(&root.path().join("missing"), &destination, Some(previous)).is_err()
        );
        assert_eq!(
            std::fs::read_to_string(destination.join("plugin.toml")).unwrap(),
            "old"
        );
    }

    #[test]
    fn manifest_origin_is_split_from_plugin_entries() {
        let (plugins, origins) = parse_manifest(
            "[[plugin]]\nsource = \"path:plugins/a\"\norigin = \"/src/a\"\n\n[[plugin]]\nsource = \"github:o/r\"\n",
        )
        .unwrap();
        assert_eq!(plugins.len(), 2);
        assert_eq!(
            origins.get("path:plugins/a"),
            Some(&PathBuf::from("/src/a"))
        );
        assert!(parse_manifest("[[plugin]]\nsource = \"path:x\"\nbogus = 1\n").is_err());
    }
}
