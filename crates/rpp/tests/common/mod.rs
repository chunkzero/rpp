//! Shared test helpers for building plugin packages and projects on disk.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rpp::lua::{LuaPluginFactory, LuaPluginLimits, PackInfo, RuntimeAccess};
use rpp::model::PluginFactory;

use tempfile::TempDir;

/// A scratch directory for a plugin package.
pub struct PluginDir {
    pub dir: TempDir,
}

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
