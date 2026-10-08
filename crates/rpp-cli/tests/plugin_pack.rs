//! Tests for `rpp plugin pack`.

mod common;

use std::collections::BTreeMap;
use std::io::Read;

use flate2::read::GzDecoder;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use common::packed_plugin::{pack, plugin, PLUGIN_MANIFEST};
use common::{run, stderr, write};

fn entries(archive: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    for entry in tar::Archive::new(GzDecoder::new(archive))
        .entries()
        .unwrap()
    {
        let mut entry = entry.unwrap();
        let path = entry.path().unwrap().to_string_lossy().into_owned();
        let mut contents = Vec::new();
        entry.read_to_end(&mut contents).unwrap();
        files.insert(path, contents);
    }
    files
}

fn hex_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[test]
fn pack_writes_deterministic_archive_and_sha() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    plugin(root);

    let first = pack(root, "out-a");
    let second = pack(root, "out-b");

    let file = root.join("out-a/packed-1.2.3.rpp.tgz");
    let bytes = std::fs::read(&file).unwrap();
    assert_eq!(
        bytes,
        std::fs::read(root.join("out-b/packed-1.2.3.rpp.tgz")).unwrap()
    );
    assert_eq!(first["sha256"], second["sha256"]);
    assert_eq!(first["sha256"], hex_sha256(&bytes));
    assert_eq!(
        std::fs::read_to_string(root.join("out-a/packed-1.2.3.rpp.tgz.sha256")).unwrap(),
        format!("{}  packed-1.2.3.rpp.tgz\n", hex_sha256(&bytes))
    );
    assert_eq!(first["name"], "packed");
    assert_eq!(first["version"], "1.2.3");
    assert_eq!(first["rpp"], ">=0.1");
    assert_eq!(first["description"], "A packed plugin");
    assert_eq!(first["file"], "packed-1.2.3.rpp.tgz");
    assert!(first.get("path").is_none());

    let mut archive = tar::Archive::new(GzDecoder::new(bytes.as_slice()));
    let mut names = Vec::new();
    for entry in archive.entries().unwrap() {
        let entry = entry.unwrap();
        let header = entry.header();
        assert_eq!(
            (
                header.mtime().unwrap(),
                header.uid().unwrap(),
                header.gid().unwrap()
            ),
            (0, 0, 0)
        );
        assert_eq!(header.mode().unwrap(), 0o644);
        names.push(entry.path().unwrap().to_string_lossy().into_owned());
    }
    assert!(names.is_sorted(), "{names:?}");
}

#[test]
fn pack_requires_rpp_range() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    plugin(root);
    write(
        root,
        "rpp.json",
        PLUGIN_MANIFEST.replace("\"rpp\": \">=0.1\",", ""),
    );

    let out = run(root, &["plugin", "pack"]);

    assert!(!out.status.success());
    assert!(stderr(&out).contains("`rpp`"), "{}", stderr(&out));
    assert!(!root.join("packed-1.2.3.rpp.tgz").exists());
}

#[test]
fn pack_rewrites_manifest_entries() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    plugin(root);
    write(
        root,
        "rpp.json",
        PLUGIN_MANIFEST.replace(
            r#""config": "src/config.ts","#,
            r#""config": "src/config.ts", "exports": { "./raw": "src/raw.ts" },"#,
        ),
    );
    write(
        root,
        "src/raw.ts",
        "import { definePlugin } from \"rpp\";\nexport const raw: typeof definePlugin = definePlugin;\n",
    );
    pack(root, "out");

    let files = entries(&std::fs::read(root.join("out/packed-1.2.3.rpp.tgz")).unwrap());

    let manifest: Value = serde_json::from_slice(&files["rpp.json"]).unwrap();
    assert_eq!(
        manifest,
        json!({
            "name": "packed",
            "version": "1.2.3",
            "description": "A packed plugin",
            "rpp": ">=0.1",
            "entry": "dist/plugin.js",
            "config": "dist/config.js",
            "exports": { "./raw": "dist/exports/raw.js" },
            "components": { "tool": "tool.wasm" },
        })
    );
    assert!(files.contains_key("dist/exports/raw.js"));
    assert!(files.contains_key("dist/exports/raw.d.ts"));
}

#[test]
fn pack_rejects_config_importing_the_plugin_sdk() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    plugin(root);
    write(
        root,
        "src/config.ts",
        "import { definePlugin } from \"rpp\";\nexport const config: typeof definePlugin = definePlugin;\n",
    );

    let out = run(root, &["plugin", "pack", "--out", "out"]);

    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("packed `dist/config.js` is not self-contained"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn pack_includes_components_and_declarations() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    plugin(root);
    pack(root, "out");

    let files = entries(&std::fs::read(root.join("out/packed-1.2.3.rpp.tgz")).unwrap());

    let names: Vec<&str> = files.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        [
            "dist/config.d.ts",
            "dist/config.js",
            "dist/config.js.map",
            "dist/plugin.js",
            "dist/plugin.js.map",
            "rpp.json",
            "tool.wasm",
            "types/src/config.d.ts",
        ]
    );
    assert_eq!(files["tool.wasm"], b"(component)");
    let stub = String::from_utf8(files["dist/config.d.ts"].clone()).unwrap();
    assert!(stub.contains("../types/src/config.js"), "{stub}");
    let plugin_js = String::from_utf8(files["dist/plugin.js"].clone()).unwrap();
    assert!(plugin_js.contains("text + '!'") || plugin_js.contains("text + \"!\""));
}
