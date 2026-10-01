use std::fs;
use std::path::Path;

use rpp_js::{bundle, BundlePackage, BundleRequest, Error};
use tempfile::TempDir;

fn write(root: &Path, name: &str, contents: &str) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn request(root: &Path, entry: &str) -> BundleRequest {
    BundleRequest {
        root: root.to_path_buf(),
        entry: entry.to_string(),
        ..Default::default()
    }
}

fn bundle_error(request: &BundleRequest) -> String {
    match bundle(request) {
        Err(Error::Bundle(message)) => message,
        other => panic!("expected a bundle error, got {other:?}"),
    }
}

fn source_list(source_map: &str) -> Vec<String> {
    let json: serde_json::Value = serde_json::from_str(source_map).unwrap();
    json["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn bundles_typescript_with_relative_imports() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    write(&root, "helper.ts", "export interface Options { n: number }\nexport const double = (n: number): number => n * 2;\n");
    write(
        &root,
        "main.ts",
        "import { double } from './helper';\nimport type { Options } from './helper';\nexport function run(options: Options): number { return double(options.n); }\n",
    );

    let bundle = bundle(&request(&root, "main.ts")).unwrap();

    assert!(bundle.code.contains("export"), "{}", bundle.code);
    assert!(bundle.code.contains("run"), "{}", bundle.code);
    assert!(!bundle.code.contains("interface"), "{}", bundle.code);
    assert!(!bundle.code.contains("import "), "{}", bundle.code);
    assert_eq!(
        bundle.inputs,
        vec![root.join("helper.ts"), root.join("main.ts")]
    );
}

#[test]
fn virtual_entry_imports_virtual_and_real_modules() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    write(
        &root,
        "src/real.ts",
        "export function real(): string { return 'real'; }\n",
    );
    let mut req = request(&root, "rpp:entry");
    req.virtual_modules.insert(
        "rpp:entry".to_string(),
        "import { host } from '#rpp';\nimport { real } from './src/real';\nexport const out: string = host() + real();\n".to_string(),
    );
    req.virtual_modules.insert(
        "#rpp".to_string(),
        "export function host(): string { return 'host'; }\n".to_string(),
    );

    let bundle = bundle(&req).unwrap();

    assert!(bundle.code.contains("out"), "{}", bundle.code);
    assert!(bundle.code.contains("'host'") || bundle.code.contains("\"host\""));
    assert_eq!(bundle.inputs, vec![root.join("src/real.ts")]);
    let sources = source_list(&bundle.source_map);
    assert!(sources.contains(&"#rpp".to_string()), "{sources:?}");
    assert!(sources.contains(&"rpp:entry".to_string()), "{sources:?}");
    assert!(sources.contains(&"src/real.ts".to_string()), "{sources:?}");
}

#[test]
fn resolves_package_imports_field() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    write(
        &root,
        "package.json",
        r##"{"imports": {"#lib": "./lib.ts"}}"##,
    );
    write(
        &root,
        "lib.ts",
        "export function lib(): number { return 42; }\n",
    );
    write(
        &root,
        "main.ts",
        "import { lib } from '#lib';\nexport const value = lib();\n",
    );

    let bundle = bundle(&request(&root, "main.ts")).unwrap();

    assert!(bundle.code.contains("42"), "{}", bundle.code);
    assert!(bundle.inputs.contains(&root.join("lib.ts")));
}

#[test]
fn rejects_node_builtins() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    write(
        &root,
        "main.ts",
        "import fs from 'node:fs';\nimport path from 'path';\nexport const x = [fs, path];\n",
    );

    let message = bundle_error(&request(&root, "main.ts"));

    assert!(message.contains("`node:fs`"), "{message}");
    assert!(message.contains("main.ts"), "{message}");
    assert!(message.contains("`path`"), "{message}");
}

#[test]
fn rejects_files_outside_root() {
    let dir = TempDir::new().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let root = base.join("root");
    write(&base, "outside.ts", "export const outside = 1;\n");
    write(
        &root,
        "main.ts",
        "import { outside } from '../outside';\nexport const x = outside;\n",
    );

    let message = bundle_error(&request(&root, "main.ts"));
    assert!(message.contains("../outside"), "{message}");
    assert!(message.contains("outside the root"), "{message}");

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(base.join("outside.ts"), root.join("link.ts")).unwrap();
        write(
            &root,
            "main.ts",
            "import { outside } from './link';\nexport const x = outside;\n",
        );

        let message = bundle_error(&request(&root, "main.ts"));
        assert!(message.contains("./link"), "{message}");
        assert!(message.contains("outside the root"), "{message}");
    }
}

#[test]
fn reports_syntax_errors_with_location() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    write(&root, "main.ts", "export const ok = 1;\nexport const = ;\n");

    let message = bundle_error(&request(&root, "main.ts"));

    assert!(message.contains("main.ts:2"), "{message}");
}

#[test]
fn source_map_lists_relative_sources() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    write(
        &root,
        "src/util/math.ts",
        "export function one(): number { return Math.random(); }\n",
    );
    write(
        &root,
        "src/main.ts",
        "import { one } from './util/math';\nexport const two = one() + one();\n",
    );

    let bundle = bundle(&request(&root, "src/main.ts")).unwrap();

    let mut sources = source_list(&bundle.source_map);
    sources.sort();
    assert_eq!(sources, vec!["src/main.ts", "src/util/math.ts"]);
}

#[test]
fn output_is_deterministic() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    write(&root, "a.ts", "export const a = 1;\n");
    write(&root, "b.ts", "export const b = 2;\n");
    write(&root, "main.ts", "import { a } from './a';\nimport { b } from './b';\nexport const sum = a + b;\nexport const lazy = () => import('./a');\n");

    let first = bundle(&request(&root, "main.ts")).unwrap();
    for _ in 0..3 {
        assert_eq!(bundle(&request(&root, "main.ts")).unwrap(), first);
    }
}

#[test]
fn inputs_exclude_bundler_runtime_helpers() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    write(&root, "legacy.cjs", "module.exports = { value: 1 };\n");
    write(
        &root,
        "main.ts",
        "import legacy from './legacy.cjs';\nexport const run = () => legacy.value;\n",
    );

    let bundle = bundle(&request(&root, "main.ts")).unwrap();

    assert_eq!(
        bundle.inputs,
        vec![root.join("legacy.cjs"), root.join("main.ts")]
    );
}

#[test]
fn inputs_include_tree_shaken_modules() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    write(
        &root,
        "unused.ts",
        "export const marker = \"shaken-away\";\n",
    );
    write(
        &root,
        "main.ts",
        "import './unused';\nexport const run = () => 1;\n",
    );

    let bundle = bundle(&request(&root, "main.ts")).unwrap();

    assert!(!bundle.code.contains("shaken-away"), "{}", bundle.code);
    assert_eq!(
        bundle.inputs,
        vec![root.join("main.ts"), root.join("unused.ts")]
    );
}

#[test]
fn package_json_mapping_edits_change_inputs() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    write(&root, "a.ts", "export const v = 'a';\n");
    write(&root, "b.ts", "export const v = 'b';\n");
    write(
        &root,
        "main.ts",
        "import { v } from '#dep';\nexport const out = v;\n",
    );
    write(&root, "package.json", r##"{"imports":{"#dep":"./a.ts"}}"##);
    let first = bundle(&request(&root, "main.ts")).unwrap();
    assert!(first.inputs.contains(&root.join("package.json")));

    write(&root, "package.json", r##"{"imports":{"#dep":"./b.ts"}}"##);
    let second = bundle(&request(&root, "main.ts")).unwrap();

    assert_ne!(first.input_hashes, second.input_hashes);
}

fn package_request(root: &Path, pkg: &Path) -> BundleRequest {
    let mut req = request(root, "main.ts");
    req.packages.insert(
        "#plugins/window".to_string(),
        BundlePackage {
            dir: pkg.to_path_buf(),
            entry: "index.ts".to_string(),
        },
    );
    req
}

#[test]
fn package_specifier_resolves_outside_root() {
    let root_dir = TempDir::new().unwrap();
    let pkg_dir = TempDir::new().unwrap();
    let root = root_dir.path().canonicalize().unwrap();
    let pkg = pkg_dir.path().canonicalize().unwrap();
    write(
        &root,
        "main.ts",
        "import { win } from '#plugins/window';\nexport const out: string = win();\n",
    );
    write(
        &pkg,
        "index.ts",
        "import { host } from '#rpp';\nimport { name } from './lib/name';\nexport const win = (): string => host() + name;\n",
    );
    write(
        &pkg,
        "lib/name.ts",
        "export const name: string = 'window';\n",
    );
    let mut req = package_request(&root, &pkg);
    req.virtual_modules.insert(
        "#rpp".to_string(),
        "export function host(): string { return 'host'; }\n".to_string(),
    );

    let bundle = bundle(&req).unwrap();

    assert!(bundle.code.contains("window"), "{}", bundle.code);
    let mut expected = vec![
        pkg.join("index.ts"),
        pkg.join("lib/name.ts"),
        root.join("main.ts"),
    ];
    expected.sort();
    assert_eq!(bundle.inputs, expected);
}

#[test]
fn package_files_cannot_escape_their_dir() {
    let base = TempDir::new().unwrap();
    let base = base.path().canonicalize().unwrap();
    let root = base.join("root");
    let pkg = base.join("pkg");
    write(&base, "secret.ts", "export const secret = 1;\n");
    write(
        &root,
        "main.ts",
        "import { win } from '#plugins/window';\nexport const out = win;\n",
    );
    write(
        &pkg,
        "index.ts",
        "import { secret } from '../secret';\nexport const win = secret;\n",
    );

    let message = bundle_error(&package_request(&root, &pkg));

    assert!(message.contains("../secret"), "{message}");
    assert!(message.contains("outside the root"), "{message}");
}

#[test]
fn package_sources_are_labelled_by_specifier() {
    let root_dir = TempDir::new().unwrap();
    let pkg_dir = TempDir::new().unwrap();
    let root = root_dir.path().canonicalize().unwrap();
    let pkg = pkg_dir.path().canonicalize().unwrap();
    write(
        &root,
        "main.ts",
        "import { win } from '#plugins/window';\nexport const out = win;\n",
    );
    write(
        &pkg,
        "index.ts",
        "import { n } from './lib/n';\nexport const win = n() + n();\n",
    );
    write(
        &pkg,
        "lib/n.ts",
        "export const n = (): number => Math.random();\n",
    );

    let bundle = bundle(&package_request(&root, &pkg)).unwrap();

    let mut sources = source_list(&bundle.source_map);
    sources.sort();
    assert_eq!(
        sources,
        vec![
            "#plugins/window/index.ts",
            "#plugins/window/lib/n.ts",
            "main.ts"
        ]
    );
}
