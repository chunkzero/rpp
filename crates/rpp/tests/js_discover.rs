//! TypeScript plugin entry discovery: `discover` patterns, namespaces and authoring sources.

#![cfg(feature = "js")]

use std::path::Path;

use rpp::config::{Config, PluginConfig, PluginPermissions, SecurityMode};
use rpp::js::{JsPluginFactory, JsPluginSpec};
use rpp::model::{GeneratorHost, PackFile, PluginFactory, ProcessOutcome};
use tempfile::TempDir;

const PLUGIN: &str = r##"
import { definePlugin } from "#rpp";
export default definePlugin({
  generate(ctx) {
    const found = ctx.discovered("windows").map((d) => ({
      path: d.path,
      namespace: d.namespace,
      title: d.module.title,
    }));
    ctx.emit("found.json", JSON.stringify(found));
  },
});
"##;

struct Project {
    dir: TempDir,
}

/// Load the plugin at `dir` into a project rooted at `root` whose source is `src`.
fn load_plugin(root: &Path, dir: &Path) -> rpp::Result<JsPluginFactory> {
    let mut config = Config::new("test-pack");
    config.pack.pack_format = Some(34);
    JsPluginFactory::load(JsPluginSpec {
        dir,
        project_root: root,
        config: &config,
        plugin: &PluginConfig {
            package: "shop-ui".into(),
            options: serde_json::json!({}),
            security: SecurityMode::Sandboxed,
            permissions: PluginPermissions::default(),
            outputs: Default::default(),
        },
        #[cfg(feature = "wasm")]
        components: Default::default(),
    })
}

impl Project {
    fn new(plugin: &str) -> Self {
        let project = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        project.write(
            "plugin/rpp.json",
            r#"{"name":"shop-ui","version":"1.0.0","config":"src/config.ts",
                "discover":{"windows":"*/window/**/window.ts"}}"#,
        );
        std::fs::create_dir(project.dir.path().join("src")).unwrap();
        project.write("plugin/src/plugin.ts", plugin);
        project.write(
            "plugin/src/config.ts",
            "export const label = (t: string) => t;\n",
        );
        project
    }

    fn write(&self, rel: &str, contents: &str) {
        let path = self.dir.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    fn load(&self) -> rpp::Result<JsPluginFactory> {
        load_plugin(self.dir.path(), &self.dir.path().join("plugin"))
    }

    fn found(&self) -> rpp::Result<serde_json::Value> {
        let mut host = Emitted(None);
        self.load()?.instantiate()?.generate(&mut host)?;
        Ok(serde_json::from_slice(&host.0.unwrap()).unwrap())
    }
}

struct Emitted(Option<Vec<u8>>);

impl GeneratorHost for Emitted {
    fn list_files(&mut self, _: Option<&str>) -> Vec<String> {
        Vec::new()
    }
    fn list_source_files(&mut self, _: Option<&str>) -> Vec<String> {
        Vec::new()
    }
    fn read_file(&mut self, _: &str) -> Option<Vec<u8>> {
        None
    }
    fn read_source(&mut self, _: &str) -> Option<Vec<u8>> {
        None
    }
    fn emit(&mut self, _: &str, contents: Vec<u8>) {
        self.0 = Some(contents);
    }
    fn remove(&mut self, _: &str) {}
    fn emit_output(&mut self, _: &str, _: &str, _: Vec<u8>) {}
}

fn window(title: &str) -> String {
    format!("export const title = {title:?};\n")
}

#[test]
fn new_matching_file_is_discovered_without_config_change() {
    let project = Project::new(PLUGIN);
    project.write("src/shop/window/main/window.ts", &window("Main"));
    assert_eq!(project.found().unwrap().as_array().unwrap().len(), 1);

    project.write("src/bank/window/window.ts", &window("Bank"));
    let found = project.found().unwrap();
    let paths: Vec<_> = found
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        paths,
        ["bank/window/window.ts", "shop/window/main/window.ts"]
    );
}

#[test]
fn exposes_path_and_namespace() {
    let project = Project::new(PLUGIN);
    project.write("src/shop/window/main/window.ts", &window("Main"));
    assert_eq!(
        project.found().unwrap(),
        serde_json::json!([
            { "path": "shop/window/main/window.ts", "namespace": "shop", "title": "Main" }
        ])
    );

    project.write("src/Bad Name/window/window.ts", &window("Bad"));
    let error = project.found().unwrap_err().to_string();
    assert!(error.contains("Bad Name/window/window.ts"), "{error}");
}

#[test]
fn helpers_bundled_through_imports_and_marked_authoring() {
    let project = Project::new(PLUGIN);
    project.write(
        "src/shop/window/window.ts",
        "import { label } from \"../../lib/label.ts\";\nexport const title = label(\"Shop\");\n",
    );
    project.write(
        "src/lib/label.ts",
        "export const label = (t: string) => `[${t}]`;\n",
    );
    project.write("src/shop/textures/a.png", "png");
    assert_eq!(project.found().unwrap()[0]["title"], "[Shop]");

    let factory = project.load().unwrap();
    assert!(factory.is_authoring_source("shop/window/window.ts"));
    assert!(factory.is_authoring_source("lib/label.ts"));
    assert!(!factory.is_authoring_source("shop/textures/a.png"));
}

#[test]
fn plugin_inside_source_dir_is_rejected() {
    let project = Project::new(PLUGIN);
    project.write(
        "src/plugins/p/rpp.json",
        r#"{"name":"inner","version":"1.0.0","config":"src/config.ts","discover":{"w":"*.ts"}}"#,
    );
    project.write("src/plugins/p/src/plugin.ts", PLUGIN);
    project.write("src/plugins/p/src/config.ts", "export const x = 1;\n");
    let error = load_plugin(
        project.dir.path(),
        &project.dir.path().join("src/plugins/p"),
    )
    .err()
    .expect("load fails")
    .to_string();
    assert!(
        error.contains("inside the pack source directory"),
        "{error}"
    );
}

#[test]
fn typescript_sources_are_authoring_when_discovering() {
    let project = Project::new(PLUGIN);
    let factory = project.load().unwrap();
    assert!(factory.is_authoring_source("lib/types.ts"));
    assert!(factory.is_authoring_source("a/b.mts"));
    assert!(factory.is_authoring_source("a/b.cts"));
    assert!(!factory.is_authoring_source("a/b.json"));
}

#[test]
fn definitions_import_plugin_api_via_plugins_specifier() {
    let project = Project::new(PLUGIN);
    project.write(
        "src/shop/window/window.ts",
        "import { label } from \"#plugins/shop-ui\";\nexport const title = label(\"Api\");\n",
    );
    assert_eq!(project.found().unwrap()[0]["title"], "Api");
}

#[test]
fn undeclared_discover_name_throws() {
    let project = Project::new(
        "import { definePlugin } from \"#rpp\";\n\
         export default definePlugin({ generate(ctx) { ctx.discovered(\"doors\"); } });\n",
    );
    let error = project.found().unwrap_err().to_string();
    assert!(
        error.contains("TypeError") && error.contains("doors"),
        "{error}"
    );
}

#[test]
fn confined_to_source_dir_while_plugin_keeps_its_own_imports() {
    let project = Project::new(
        "import { definePlugin } from \"#rpp\";\nimport { note } from \"./helper.ts\";\n\
         export default definePlugin({ generate(ctx) { ctx.emit(\"found.json\", JSON.stringify([note])); } });\n",
    );
    project.write("plugin/src/helper.ts", "export const note = \"helped\";\n");
    assert_eq!(project.found().unwrap(), serde_json::json!(["helped"]));

    project.write("secret.ts", "export const title = \"secret\";\n");
    project.write(
        "src/shop/window/window.ts",
        "export { title } from \"../../../secret.ts\";\n",
    );
    assert!(project.load().is_err());
}

fn keys(project: &Project) -> (u64, u64) {
    let factory = project.load().unwrap();
    (factory.processor_key(), factory.cache_key())
}

#[test]
fn definition_edit_changes_generator_key_only() {
    let project = Project::new(PLUGIN);
    project.write("src/shop/window/window.ts", &window("Main"));
    let (processor, generator) = keys(&project);
    assert_eq!(keys(&project), (processor, generator));

    project.write("src/shop/window/window.ts", &window("Changed"));
    let (new_processor, new_generator) = keys(&project);
    assert_eq!(new_processor, processor);
    assert_ne!(new_generator, generator);
}

#[test]
fn adding_and_removing_entry_changes_generator_key_only() {
    let project = Project::new(PLUGIN);
    project.write("src/shop/window/window.ts", &window("Main"));
    let (processor, generator) = keys(&project);

    project.write("src/bank/window/window.ts", &window("Bank"));
    let (added_processor, added_generator) = keys(&project);
    assert_eq!(added_processor, processor);
    assert_ne!(added_generator, generator);

    std::fs::remove_file(project.dir.path().join("src/bank/window/window.ts")).unwrap();
    assert_eq!(keys(&project), (processor, generator));
}

#[test]
fn helper_edit_changes_generator_key_only() {
    let project = Project::new(PLUGIN);
    project.write(
        "src/shop/window/window.ts",
        "import { label } from \"../../lib/label.ts\";\nexport const title = label(\"Shop\");\n",
    );
    project.write(
        "src/lib/label.ts",
        "export const label = (t: string) => t;\n",
    );
    let (processor, generator) = keys(&project);

    project.write(
        "src/lib/label.ts",
        "export const label = (t: string) => `[${t}]`;\n",
    );
    let (new_processor, new_generator) = keys(&project);
    assert_eq!(new_processor, processor);
    assert_ne!(new_generator, generator);
}

#[test]
fn processor_cannot_read_discovered() {
    let project = Project::new(
        "import { definePlugin } from \"#rpp\";\n\
         export default definePlugin({ processors: { p: { files: \"**/*\", \
         run(ctx: any) { ctx.discovered(\"windows\"); } } } });\n",
    );
    let mut instance = project.load().unwrap().instantiate().unwrap();
    let mut file = PackFile::new("a.txt", b"x".to_vec());
    let result: rpp::Result<ProcessOutcome> = instance.process("p", &mut file);
    let error = result.unwrap_err().to_string();
    assert!(
        error.contains("only available in generate/onStart/onFinish"),
        "{error}"
    );
}

#[test]
fn second_load_reuses_cached_bundle() {
    let project = Project::new(PLUGIN);
    project.write("src/shop/window/window.ts", &window("Main"));
    let cache = project.dir.path().join(".rpp/cache");
    let first = keys(&project);

    let entries = || std::fs::read_dir(cache.join("bundles")).unwrap().count();
    assert_eq!(entries(), 1);
    let stored = std::fs::read_dir(cache.join("bundles"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let modified = std::fs::metadata(&stored).unwrap().modified().unwrap();

    assert_eq!(keys(&project), first);
    assert_eq!(
        std::fs::metadata(&stored).unwrap().modified().unwrap(),
        modified
    );

    project.write("src/shop/window/window.ts", &window("Changed"));
    assert_ne!(keys(&project), first);
    assert_eq!(entries(), 1);
}
