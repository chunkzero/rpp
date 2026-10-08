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

fn request(root: &Path, plugin: &str, config: Option<&str>) -> PackRequest {
    PackRequest {
        root: root.to_path_buf(),
        plugin: plugin.to_string(),
        config: config.map(str::to_string),
        self_specifier: None,
        jsx_import_source: None,
        ..Default::default()
    }
}

fn packed(root: &Path, plugin: &str, config: Option<&str>) -> PackOutput {
    pack(&request(root, plugin, config)).unwrap()
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

    let output = packed(root, "src/plugin.ts", None);

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

    let output = packed(root, "plugin.ts", None);

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

    let output = packed(&root, "plugin.ts", None);

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
        "import { log } from 'rpp';\nimport { read } from 'rpp:fs';\nexport const run = (): void => { log(read()); };\n",
    );

    let output = packed(root, "plugin.ts", None);

    let code = &output.files["dist/plugin.js"];
    assert!(code.contains("from \"rpp\""), "{code}");
    assert!(code.contains("from \"rpp:fs\""), "{code}");
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

    let output = packed(root, "src/plugin.ts", Some("src/config.ts"));

    let chunks: Vec<_> = output
        .files
        .iter()
        .filter(|(name, _)| name.starts_with("dist/chunk-") && name.ends_with(".js"))
        .collect();
    assert_eq!(chunks.len(), 1, "{:?}", output.files.keys());
    assert!(chunks[0].1.contains("shared-marker"));
    assert_eq!(output.plugin, "dist/plugin.js");
    assert_eq!(output.config.as_deref(), Some("dist/config.js"));
    assert!(output.files[&output.plugin].contains("./chunk-"));
    assert!(output.files["dist/config.js"].contains("./chunk-"));
    assert!(!output.files[&output.plugin].contains("shared-marker"));
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

    let mut req = request(root, "src/plugin.ts", Some("src/config.ts"));
    req.self_specifier = Some("plugin:self".to_string());
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
fn pack_declares_tsx_modules_as_tsc_resolves_them() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    write(
        root,
        "src/options.tsx",
        "export interface Options { size: number }\n",
    );
    write(root, "src/size.ts", "export type Size = number;\n");
    write(
        root,
        "src/other.tsx",
        "export interface Other { n: number }\n",
    );
    write(
        root,
        "src/other/index.ts",
        "export interface Index { n: number }\n",
    );
    write(
        root,
        "src/config.ts",
        "import type { Options } from './options.jsx';\n\
         import type { Size } from './size.jsx';\n\
         import type { Other } from './other';\n\
         export default function config(options: Options & Other): Size { return options.size; }\n",
    );
    write(root, "src/plugin.ts", "export const run = () => 1;\n");
    let output = packed(root, "src/plugin.ts", Some("src/config.ts"));
    let keys: Vec<_> = output.declarations.keys().map(String::as_str).collect();
    // `.jsx` names a `.tsx` or `.ts` file, and `./other` prefers the file over the directory index.
    assert_eq!(
        keys,
        [
            "types/src/config.d.ts",
            "types/src/options.d.ts",
            "types/src/other.d.ts",
            "types/src/size.d.ts"
        ]
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

    let result = pack(&request(root, "src/plugin.ts", Some("src/config.ts")));

    match result {
        Err(Error::Bundle(message)) => assert!(message.contains("src/config.ts"), "{message}"),
        other => panic!("expected a bundle error, got {other:?}"),
    }
}

#[test]
fn pack_requires_a_plugin_entry() {
    let dir = TempDir::new().unwrap();
    let result = pack(&request(dir.path(), "", None));
    assert!(matches!(result, Err(Error::Invalid(_))));
}

#[test]
fn pack_declares_type_only_dependencies_transitively() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    write(root, "src/plugin.ts", "export const run = () => 1;\n");
    write(
        root,
        "src/config.ts",
        "import type { Options } from './types.js';\nexport default function config(options: Options): number { return options.size; }\n",
    );
    write(
        root,
        "src/types.ts",
        "export type { Size } from './nested/size';\nexport interface Options { size: number }\n",
    );
    write(root, "src/nested/size.ts", "export type Size = number;\n");

    let output = packed(root, "src/plugin.ts", Some("src/config.ts"));

    let keys: Vec<_> = output.declarations.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "types/src/config.d.ts",
            "types/src/nested/size.d.ts",
            "types/src/types.d.ts"
        ]
    );
}

#[test]
fn pack_stub_reexports_default_declared_by_specifier() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    write(root, "src/plugin.ts", "export const run = () => 1;\n");
    write(
        root,
        "src/config.mts",
        "const config: () => number = () => 1;\nexport { config as default };\n",
    );

    let output = packed(root, "src/plugin.ts", Some("src/config.mts"));

    assert!(output.declarations.contains_key("types/src/config.d.mts"));
    let stub = &output.files["dist/config.d.ts"];
    assert!(
        stub.contains("export { default } from \"../types/src/config.mjs\";"),
        "{stub}"
    );
}

#[test]
fn pack_bundles_exports_beside_the_config() {
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
        "export const run = (): number => 1;\n",
    );
    write(
        root,
        "src/config.ts",
        "import { raw } from 'plugin:window/raw';\nexport const name: string = raw();\n",
    );
    write(
        root,
        "src/raw.ts",
        "import { shared } from './shared';\nexport const raw = (): string => shared();\n",
    );
    let mut request = request(root, "src/plugin.ts", Some("src/config.ts"));
    request.self_specifier = Some("plugin:window".to_string());
    request
        .exports
        .insert("raw".to_string(), "src/raw.ts".to_string());

    let output = pack(&request).unwrap();

    assert_eq!(output.exports["raw"], "dist/exports/raw.js");
    assert!(output.files["dist/exports/raw.js"].contains("../chunk-"));
    assert!(output.files["dist/config.js"].contains("./chunk-"));
    assert_eq!(
        output.files["dist/exports/raw.d.ts"],
        "export * from \"../../types/src/raw.js\";\n"
    );
    assert!(output.declarations.contains_key("types/src/raw.d.ts"));
}
