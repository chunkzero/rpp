//! Evaluating `rpp.config.ts`.

#![cfg(feature = "js")]

use std::collections::BTreeMap;
use std::path::Path;

use rpp::js::{evaluate_config, ConfigPackage, JsPluginLimits};
use rpp::Error;

fn write_file(root: &Path, rel: &str, contents: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn evaluate(
    root: &Path,
    packages: &BTreeMap<String, ConfigPackage>,
) -> rpp::Result<rpp::js::EvaluatedConfig> {
    evaluate_config(root, packages, JsPluginLimits::default())
}

fn config_error(result: rpp::Result<rpp::js::EvaluatedConfig>) -> String {
    match result.unwrap_err() {
        Error::Config { path, message } => {
            assert!(path.ends_with("rpp.config.ts"), "{path:?}");
            message
        }
        other => panic!("expected a config error, got {other}"),
    }
}

fn demo_package(root: &Path, validate: &str) -> BTreeMap<String, ConfigPackage> {
    let dir = root.join("deps/demo");
    write_file(&dir, "rpp.json", "{}");
    write_file(
        &dir,
        "src/config.ts",
        &r##"import { definePluginConfig } from "#rpp/config";
export default definePluginConfig<{ level?: number }>("demo", {
  normalize: (options) => ({ level: 1, ...options }),
  validate: (options) => VALIDATE,
});
"##
        .replace("VALIDATE", validate),
    );
    BTreeMap::from([(
        "demo".to_string(),
        ConfigPackage {
            dir,
            config: Some("src/config.ts".into()),
        },
    )])
}

#[test]
fn evaluates_minimal_config() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "rpp.config.ts",
        r##"import { defineConfig } from "#rpp/config";
export default defineConfig({ pack: { name: "mini", packFormat: 34 } });
"##,
    );
    let evaluated = evaluate(dir.path(), &BTreeMap::new()).unwrap();
    assert_eq!(evaluated.config.pack.name, "mini");
    assert!(evaluated.config.plugins.is_empty());
    assert!(evaluated
        .inputs
        .iter()
        .any(|p| p.ends_with("rpp.config.ts")));
}

#[test]
fn plugin_config_factory_types_options() {
    let dir = tempfile::tempdir().unwrap();
    let packages = demo_package(dir.path(), "undefined");
    write_file(
        dir.path(),
        "rpp.config.ts",
        r##"import { defineConfig } from "#rpp/config";
import demo from "#plugins/demo";
export default defineConfig({
  pack: { name: "p" },
  plugins: [demo({}), demo({ level: 5 })],
});
"##,
    );
    let evaluated = evaluate(dir.path(), &packages).unwrap();
    let plugins = &evaluated.config.plugins;
    assert_eq!(plugins.len(), 2);
    assert_eq!(plugins[0].package, "demo");
    assert_eq!(
        plugins[0].options.get("level").unwrap().as_integer(),
        Some(1)
    );
    assert_eq!(
        plugins[1].options.get("level").unwrap().as_integer(),
        Some(5)
    );
    assert!(evaluated
        .inputs
        .iter()
        .any(|p| p.ends_with("src/config.ts")));
}

#[test]
fn factory_validation_errors_name_plugin() {
    let dir = tempfile::tempdir().unwrap();
    let packages = demo_package(dir.path(), r#"["level must be positive"]"#);
    write_file(
        dir.path(),
        "rpp.config.ts",
        r##"import demo from "#plugins/demo";
export default { pack: { name: "p" }, plugins: [demo({ level: -1 })] };
"##,
    );
    let message = config_error(evaluate(dir.path(), &packages));
    assert!(message.contains("level must be positive"), "{message}");
    assert!(message.contains("demo"), "{message}");
}

#[test]
fn config_can_import_local_helpers() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "config/helper.ts",
        "export const name: string = \"helped\";\n",
    );
    write_file(
        dir.path(),
        "rpp.config.ts",
        "import { name } from \"./config/helper.ts\";\nexport default { pack: { name } };\n",
    );
    let evaluated = evaluate(dir.path(), &BTreeMap::new()).unwrap();
    assert_eq!(evaluated.config.pack.name, "helped");
    assert!(evaluated
        .inputs
        .iter()
        .any(|p| p.ends_with("config/helper.ts")));
}

#[test]
fn host_functions_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "rpp.config.ts",
        r#"declare const __rpp: { call(name: string, value: unknown): unknown };
export default {
  get pack() {
    return { name: String(__rpp.call("files", {})) };
  },
};
"#,
    );
    let message = config_error(evaluate(dir.path(), &BTreeMap::new()));
    assert!(
        message.contains("host functions are unavailable in rpp.config.ts"),
        "{message}"
    );
}

#[test]
fn missing_default_export_errors() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        dir.path(),
        "rpp.config.ts",
        "export const pack = { name: \"x\" };\n",
    );
    config_error(evaluate(dir.path(), &BTreeMap::new()));
}
