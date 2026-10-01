//! TypeScript plugin entry discovery: `discover` patterns, namespaces and authoring sources.

#![cfg(feature = "js")]

use rpp::host::{PackInfo, RuntimeAccess};
use rpp::js::{JsPluginFactory, JsPluginLimits};
use rpp::model::{GeneratorHost, PluginFactory};
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
        JsPluginFactory::load(
            self.dir.path().join("plugin"),
            toml::Value::Table(Default::default()),
            PackInfo {
                name: "test-pack".into(),
                description: None,
                format: Some(34),
            },
            JsPluginLimits::default(),
            RuntimeAccess::sandboxed(".".into()),
            &self.dir.path().join("src"),
        )
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
