//! Helpers for loading and running TypeScript plugins from temporary directories.

use std::path::Path;

use rpp::config::{Config, PluginConfig, PluginPermissions, SecurityMode};
use rpp::js::{JsPluginFactory, JsPluginSpec};
use rpp::model::{GeneratorHost, PackFile, PluginInstance, ProcessOutcome};
use tempfile::TempDir;

pub fn write_file(root: &Path, rel: &str, contents: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

/// A plugin directory named `ts-test` whose entry `src/plugin.ts` holds `source`.
pub fn write_plugin(source: &str) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("rpp.json"),
        r#"{ "name": "ts-test", "version": "1.0.0", "entry": "src/plugin.ts" }"#,
    )
    .unwrap();
    write_file(dir.path(), "src/plugin.ts", source);
    dir
}

/// A sandboxed plugin entry with `options` given as JSON text.
pub fn plugin(options: &str) -> PluginConfig {
    PluginConfig {
        package: "ts-test".into(),
        options: serde_json::from_str(options).unwrap(),
        security: SecurityMode::Sandboxed,
        permissions: PluginPermissions::default(),
        outputs: Default::default(),
    }
}

/// Load the plugin at `dir` into a project rooted at `project_root`.
pub fn try_load_in(
    project_root: &Path,
    dir: &Path,
    plugin: &PluginConfig,
) -> rpp::Result<JsPluginFactory> {
    let config = Config::new("test-pack", 34);
    JsPluginFactory::load(JsPluginSpec {
        dir,
        project_root,
        config: &config,
        plugin,
        #[cfg(feature = "wasm")]
        components: Default::default(),
    })
}

pub fn try_load_with(dir: &Path, plugin: &PluginConfig) -> rpp::Result<JsPluginFactory> {
    try_load_in(dir, dir, plugin)
}

pub fn try_load(dir: &Path, options: &str) -> rpp::Result<JsPluginFactory> {
    try_load_with(dir, &plugin(options))
}

pub fn load(dir: &Path, options: &str) -> JsPluginFactory {
    try_load(dir, options).unwrap()
}

pub fn process(
    instance: &mut dyn PluginInstance,
    processor: &str,
    path: &str,
    body: &str,
) -> (PackFile, ProcessOutcome) {
    let mut file = PackFile::new(path, body.as_bytes().to_vec());
    let outcome = instance.process(processor, &mut file).unwrap();
    (file, outcome)
}

pub fn text(file: &PackFile) -> &str {
    std::str::from_utf8(&file.contents).unwrap()
}

/// A generator host backed by in-memory files that records everything a generator does.
#[derive(Default)]
pub struct Recorder {
    pub outputs: Vec<(String, Vec<u8>)>,
    pub sources: Vec<(String, Vec<u8>)>,
    pub emitted: Vec<(String, Vec<u8>)>,
    pub removed: Vec<String>,
    pub root_outputs: Vec<(String, String, Vec<u8>)>,
}

impl GeneratorHost for Recorder {
    fn list_files(&mut self, glob: Option<&str>) -> Vec<String> {
        let all = self.outputs.iter().map(|(path, _)| path.clone());
        match glob {
            Some(glob) => all
                .filter(|path| glob::Pattern::new(glob).unwrap().matches(path))
                .collect(),
            None => all.collect(),
        }
    }

    fn list_source_files(&mut self, _glob: Option<&str>) -> Vec<String> {
        self.sources.iter().map(|(path, _)| path.clone()).collect()
    }

    fn read_file(&mut self, path: &str) -> Option<Vec<u8>> {
        let found = self.outputs.iter().find(|(p, _)| p == path);
        found.map(|(_, bytes)| bytes.clone())
    }

    fn read_source(&mut self, path: &str) -> Option<Vec<u8>> {
        let found = self.sources.iter().find(|(p, _)| p == path);
        found.map(|(_, bytes)| bytes.clone())
    }

    fn emit(&mut self, path: &str, contents: Vec<u8>) {
        self.emitted.push((path.into(), contents));
    }

    fn remove(&mut self, path: &str) {
        self.removed.push(path.into());
    }

    fn emit_output(&mut self, root: &str, path: &str, contents: Vec<u8>) {
        self.root_outputs.push((root.into(), path.into(), contents));
    }
}
