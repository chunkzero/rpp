use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use rpp_js::{pack, Error, PackOutput, PackRequest};
use tempfile::TempDir;

fn write(root: &Path, name: &str, contents: &str) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn package(root: &Path, name: &str, manifest: &str, files: &[(&str, &str)]) {
    let dir = format!("node_modules/{name}");
    write(root, &format!("{dir}/package.json"), manifest);
    for (file, contents) in files {
        write(root, &format!("{dir}/{file}"), contents);
    }
}

fn request(root: &Path, entries: &[(&str, &str)]) -> PackRequest {
    PackRequest {
        root: root.to_path_buf(),
        entries: entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        self_specifier: None,
    }
}

fn packed(root: &Path, entries: &[(&str, &str)]) -> PackOutput {
    pack(&request(root, entries)).unwrap()
}

#[test]
fn pack_inlines_node_modules_dependency() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    package(
        root,
        "dep",
        r#"{"name":"dep","version":"1.0.0","type":"module","exports":"./index.js"}"#,
        &[("index.js", "export const dep = () => 'from-dep-marker';\n")],
    );
    write(
        root,
        "src/plugin.ts",
        "import { dep } from 'dep';\nexport const run = (): string => dep();\n",
    );

    let output = packed(root, &[("plugin", "src/plugin.ts")]);

    let code = &output.files["dist/plugin.js"];
    assert!(code.contains("from-dep-marker"), "{code}");
    assert!(!code.contains("from \"dep\""), "{code}");
    assert!(
        code.ends_with("//# sourceMappingURL=plugin.js.map\n"),
        "{code}"
    );
    let map: serde_json::Value = serde_json::from_str(&output.files["dist/plugin.js.map"]).unwrap();
    let sources: Vec<&str> = map["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    assert!(sources.contains(&"../src/plugin.ts"), "{sources:?}");
    assert!(
        sources.contains(&"../node_modules/dep/index.js"),
        "{sources:?}"
    );
}

#[test]
fn pack_resolves_main_field_only_package() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    package(
        root,
        "legacy",
        r#"{"name":"legacy","version":"1.0.0","main":"lib/main.js"}"#,
        &[("lib/main.js", "export const legacy = 'legacy-marker';\n")],
    );
    write(
        root,
        "plugin.ts",
        "import { legacy } from 'legacy';\nexport const out: string = legacy;\n",
    );

    let output = packed(root, &[("plugin", "plugin.ts")]);

    assert!(output.files["dist/plugin.js"].contains("legacy-marker"));
}

#[test]
fn pack_allows_hoisted_node_modules() {
    let dir = TempDir::new().unwrap();
    let workspace = dir.path();
    package(
        workspace,
        "hoisted",
        r#"{"name":"hoisted","version":"1.0.0","exports":"./index.js"}"#,
        &[("index.js", "export const hoisted = 'hoisted-marker';\n")],
    );
    let root = workspace.join("packages/plugin");
    write(
        &root,
        "plugin.ts",
        "import { hoisted } from 'hoisted';\nexport const out: string = hoisted;\n",
    );

    let output = packed(&root, &[("plugin", "plugin.ts")]);

    assert!(output.files["dist/plugin.js"].contains("hoisted-marker"));
    let map: serde_json::Value = serde_json::from_str(&output.files["dist/plugin.js.map"]).unwrap();
    assert!(
        map["sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s == "../node_modules/hoisted/index.js"),
        "{map}"
    );
}

#[test]
fn pack_keeps_rpp_external() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    write(
        root,
        "plugin.ts",
        "import { log } from '#rpp';\nimport { read } from '#rpp/fs';\nexport const run = (): void => { log(read()); };\n",
    );

    let output = packed(root, &[("plugin", "plugin.ts")]);

    let code = &output.files["dist/plugin.js"];
    assert!(code.contains("from \"#rpp\""), "{code}");
    assert!(code.contains("from \"#rpp/fs\""), "{code}");
}

#[test]
fn pack_shares_chunk_between_plugin_and_config() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    write(
        root,
        "src/shared.ts",
        "export const shared = (): string => 'shared-marker';\n",
    );
    write(
        root,
        "src/plugin.ts",
        "import { shared } from './shared';\nexport const run = (): string => shared();\n",
    );
    write(
        root,
        "src/config.ts",
        "import { shared } from './shared';\nexport const name: string = shared();\n",
    );

    let output = packed(
        root,
        &[("plugin", "src/plugin.ts"), ("config", "src/config.ts")],
    );

    let chunks: Vec<_> = output
        .files
        .iter()
        .filter(|(name, _)| name.starts_with("dist/chunk-") && name.ends_with(".js"))
        .collect();
    assert_eq!(chunks.len(), 1, "{:?}", output.files.keys());
    assert!(chunks[0].1.contains("shared-marker"));
    assert!(output.files["dist/plugin.js"].contains("./chunk-"));
    assert!(output.files["dist/config.js"].contains("./chunk-"));
    assert!(!output.files["dist/plugin.js"].contains("shared-marker"));
}

#[test]
fn pack_emits_isolated_declarations() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    package(
        root,
        "dep",
        r#"{"name":"dep","version":"1.0.0","main":"index.js"}"#,
        &[("index.js", "export const dep = 1;\n")],
    );
    write(
        root,
        "src/options.ts",
        "export interface Options { size: number }\nexport const defaults: Options = { size: 1 };\n",
    );
    write(
        root,
        "src/config.ts",
        "import { dep } from 'dep';\nimport { defaults, type Options } from './options';\nexport default function config(options: Options = defaults): number { return options.size + dep; }\n",
    );
    write(root, "src/plugin.ts", "export const run = () => 1;\n");

    let mut req = request(
        root,
        &[("plugin", "src/plugin.ts"), ("config", "src/config.ts")],
    );
    req.self_specifier = Some("#plugins/self".to_string());
    let output = pack(&req).unwrap();

    let keys: Vec<_> = output.declarations.keys().map(String::as_str).collect();
    assert_eq!(keys, ["types/src/config.d.ts", "types/src/options.d.ts"]);
    assert!(output.declarations["types/src/config.d.ts"].contains("export default function config"));
    assert!(output.declarations["types/src/options.d.ts"].contains("interface Options"));
    let stub = &output.files["dist/config.d.ts"];
    assert!(
        stub.contains("export * from \"../types/src/config.js\";"),
        "{stub}"
    );
    assert!(
        stub.contains("export { default } from \"../types/src/config.js\";"),
        "{stub}"
    );
}

#[test]
fn pack_reports_isolated_declaration_errors() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    write(root, "src/plugin.ts", "export const run = () => 1;\n");
    write(
        root,
        "src/config.ts",
        "const compute = (): number => 1;\nexport const size = compute() + 1;\n",
    );

    let result = pack(&request(
        root,
        &[("plugin", "src/plugin.ts"), ("config", "src/config.ts")],
    ));

    match result {
        Err(Error::Bundle(message)) => assert!(message.contains("src/config.ts"), "{message}"),
        other => panic!("expected a bundle error, got {other:?}"),
    }
}

#[test]
fn pack_requires_a_plugin_entry() {
    let dir = TempDir::new().unwrap();
    let entries: BTreeMap<String, String> = BTreeMap::new();
    let result = pack(&PackRequest {
        root: dir.path().to_path_buf(),
        entries,
        self_specifier: None,
    });
    assert!(matches!(result, Err(Error::Bundle(_))));
}
