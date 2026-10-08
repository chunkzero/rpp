mod common;

use std::path::Path;

use rpp_js::{bundle, BundlePackage, BundleRequest};
use tempfile::TempDir;

use common::{bundle_error, request, source_list, write};

fn package_request(root: &Path, pkg: &Path) -> BundleRequest {
    let mut req = request(root, "main.ts");
    req.packages.insert(
        "plugin:window".to_string(),
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
        "import { win } from 'plugin:window';\nexport const out: string = win();\n",
    );
    write(
        &pkg,
        "index.ts",
        "import { host } from 'rpp';\nimport { name } from './lib/name';\nexport const win = (): string => host() + name;\n",
    );
    write(
        &pkg,
        "lib/name.ts",
        "export const name: string = 'window';\n",
    );
    let mut req = package_request(&root, &pkg);
    req.virtual_modules.insert(
        "rpp".to_string(),
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
        "import { win } from 'plugin:window';\nexport const out = win;\n",
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
        "import { win } from 'plugin:window';\nexport const out = win;\n",
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
            "main.ts",
            "plugin:window/index.ts",
            "plugin:window/lib/n.ts"
        ]
    );
}

fn mapped_library(root: &Path) -> BundleRequest {
    write(root, "src/lib.ts", "export const lib = 1;\n");
    write(
        root,
        "dist/lib.js",
        "export const lib = 1;\n//# sourceMappingURL=lib.js.map\n",
    );
    write(
        root,
        "dist/lib.js.map",
        r#"{"version":3,"sources":["../src/lib.ts"],"names":[],"mappings":"AAAA"}"#,
    );
    write(root, "main.ts", "export { lib } from './dist/lib.js';\n");
    request(root, "main.ts")
}

#[test]
fn bundle_chains_input_source_maps() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let req = mapped_library(&root);

    let bundle = bundle(&req).unwrap();

    let sources = source_list(&bundle.source_map);
    assert!(sources.contains(&"src/lib.ts".to_string()), "{sources:?}");
    assert!(!sources.contains(&"dist/lib.js".to_string()), "{sources:?}");
}

#[test]
fn bundle_records_source_map_inputs() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let req = mapped_library(&root);

    let bundle = bundle(&req).unwrap();

    assert!(bundle.inputs.contains(&root.join("dist/lib.js.map")));
}

#[test]
fn package_subpaths_share_modules() {
    let root_dir = TempDir::new().unwrap();
    let pkg_dir = TempDir::new().unwrap();
    let root = root_dir.path().canonicalize().unwrap();
    let pkg = pkg_dir.path().canonicalize().unwrap();
    write(
        &root,
        "main.ts",
        "import { a } from 'plugin:window';\nimport { b } from 'plugin:window/raw';\nexport const same: boolean = a === b;\n",
    );
    write(&pkg, "state.ts", "export const state = {};\n");
    write(&pkg, "index.ts", "export { state as a } from './state';\n");
    write(&pkg, "raw.ts", "export { state as b } from './state';\n");
    let mut req = package_request(&root, &pkg);
    req.packages.insert(
        "plugin:window/raw".to_string(),
        BundlePackage {
            dir: pkg.clone(),
            entry: "raw.ts".to_string(),
        },
    );

    let bundle = bundle(&req).unwrap();

    assert_eq!(
        bundle.code.matches("const state").count(),
        1,
        "{}",
        bundle.code
    );
}
