//! Shared test helpers for building plugin packages and projects on disk.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
#[cfg(feature = "lua")]
use std::sync::Arc;

#[cfg(feature = "lua")]
use rpp::lua::{LuaPluginFactory, LuaPluginLimits, PackInfo, RuntimeAccess};
#[cfg(feature = "lua")]
use rpp::model::PluginFactory;

use tempfile::TempDir;

/// A scratch directory for a plugin package.
#[cfg(feature = "lua")]
pub struct PluginDir {
    pub dir: TempDir,
}

#[cfg(feature = "lua")]
impl PluginDir {
    /// Create a Lua plugin package with the given id and entry source.
    pub fn lua(id: &str, entry: &str) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("plugin.toml"),
            format!("[plugin]\nid = \"{id}\"\nversion = \"1.0.0\"\n"),
        )
        .unwrap();
        std::fs::write(dir.path().join("init.lua"), entry).unwrap();
        PluginDir { dir }
    }

    /// Write an additional module file inside the plugin package.
    pub fn with_module(self, rel: &str, source: &str) -> Self {
        let path = self.dir.path().join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, source).unwrap();
        self
    }

    /// Declare `overrides` globs in the plugin manifest.
    pub fn with_overrides(self, globs: &[&str]) -> Self {
        let id = self.id_from_manifest();
        let list = globs
            .iter()
            .map(|g| format!("{g:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        std::fs::write(
            self.dir.path().join("plugin.toml"),
            format!("[plugin]\nid = \"{id}\"\nversion = \"1.0.0\"\noverrides = [{list}]\n"),
        )
        .unwrap();
        self
    }

    fn id_from_manifest(&self) -> String {
        let text = std::fs::read_to_string(self.dir.path().join("plugin.toml")).unwrap();
        text.lines()
            .find_map(|line| line.strip_prefix("id = \""))
            .and_then(|rest| rest.strip_suffix('"'))
            .expect("manifest id")
            .to_string()
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Load this plugin into a factory with the given options TOML.
    pub fn factory(&self, options_toml: &str) -> LuaPluginFactory {
        let options: toml::Value = toml::from_str(options_toml).unwrap();
        load_plugin(self.dir.path(), options).expect("load plugin")
    }

    pub fn factory_arc(&self, options_toml: &str) -> Arc<dyn PluginFactory> {
        Arc::new(self.factory(options_toml))
    }
}

/// A scratch project (src dir) for end-to-end engine tests.
pub struct Project {
    pub dir: TempDir,
}

impl Project {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        Project { dir }
    }

    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    pub fn src(&self) -> PathBuf {
        self.dir.path().join("src")
    }

    /// Write a source file (relative to `src/`).
    pub fn write_src(&self, rel: &str, contents: &str) {
        let path = self.src().join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    /// Read an output file (relative to `dist/`).
    pub fn read_out(&self, rel: &str) -> Option<String> {
        std::fs::read_to_string(self.dir.path().join("dist").join(rel)).ok()
    }

    /// Whether an output file exists.
    pub fn out_exists(&self, rel: &str) -> bool {
        self.dir.path().join("dist").join(rel).exists()
    }

    /// A default config with `pack.name = "test-pack"`.
    pub fn config(&self) -> rpp::config::Config {
        rpp::config::Config::parse("[pack]\nname = \"test-pack\"\n", "rpp.toml").unwrap()
    }
}

/// Load a sandboxed Lua plugin with default limits and test pack metadata.
#[cfg(feature = "lua")]
pub fn load_plugin(dir: &Path, options: toml::Value) -> rpp::Result<LuaPluginFactory> {
    LuaPluginFactory::load(
        dir,
        options,
        PackInfo {
            name: "test-pack".into(),
            description: None,
            format: Some(34),
        },
        LuaPluginLimits::default(),
        RuntimeAccess::sandboxed(".".into()),
    )
}
