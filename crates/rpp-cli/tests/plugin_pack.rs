//! Tests for `rpp plugin pack` and installing the packed archive from a registry.

mod common;

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use flate2::read::GzDecoder;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn write(root: &Path, rel: &str, contents: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn run(root: &Path, args: &[&str]) -> std::process::Output {
    common::command(root).args(args).output().expect("run rpp")
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

const PLUGIN_MANIFEST: &str = r#"{
  "name": "packed",
  "version": "1.2.3",
  "description": "A packed plugin",
  "rpp": ">=0.1",
  "entry": "src/plugin.ts",
  "config": "src/config.ts",
  "components": { "tool": "tool.wasm" },
  "dependencies": { "dep": "^1" }
}
"#;

/// A plugin whose processor appends `dep`'s mark, or throws from `src/helper.ts` for
/// the text `explode`.
fn plugin(root: &Path) {
    write(root, "rpp.json", PLUGIN_MANIFEST);
    write(root, "tool.wasm", "(component)");
    write(
        root,
        "node_modules/dep/package.json",
        r#"{"name":"dep","version":"1.0.0","type":"module","exports":"./index.js"}"#,
    );
    write(
        root,
        "node_modules/dep/index.js",
        "export const mark = (text) => text + '!';\n",
    );
    write(
        root,
        "src/helper.ts",
        "export function check(text: string): void {\n  if (text === 'explode') {\n    throw new Error('boom from helper');\n  }\n}\n",
    );
    write(
        root,
        "src/config.ts",
        r##"import { definePluginConfig, type Access, type PluginEntry } from "#rpp/config";

type Options = { text: string };
const config: (options: Options, access?: Access) => PluginEntry = definePluginConfig<Options>(
  "packed",
);
export default config;
"##,
    );
    write(
        root,
        "src/plugin.ts",
        r##"import { definePlugin } from "#rpp";
import { mark } from "dep";
import { check } from "./helper";

export default definePlugin<{ text: string }>({
  processors: {
    mark: {
      files: ["*.txt"],
      run(_ctx, file) {
        check(file.text);
        file.text = mark(file.text);
      },
    },
  },
});
"##,
    );
}

fn pack(root: &Path, out: &str) -> Value {
    let result = run(root, &["plugin", "pack", "--out", out, "--json"]);
    assert!(result.status.success(), "{}", stderr(&result));
    serde_json::from_slice(&result.stdout).unwrap()
}

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
    assert!(first["file"]
        .as_str()
        .unwrap()
        .ends_with("out-a/packed-1.2.3.rpp.tgz"));

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
        &PLUGIN_MANIFEST.replace("\"rpp\": \">=0.1\",", ""),
    );

    let out = run(root, &["plugin", "pack"]);

    assert!(!out.status.success());
    assert!(stderr(&out).contains("`rpp`"), "{}", stderr(&out));
    assert!(!root.join("packed-1.2.3.rpp.tgz").exists());
}

#[test]
fn pack_rewrites_manifest_entry_and_config() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    plugin(root);
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
            "components": { "tool": "tool.wasm" },
        })
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

/// A static HTTP server on a loopback port.
struct Server {
    base: String,
    routes: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let routes = Arc::new(Mutex::new(HashMap::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let handle = {
            let (routes, stop) = (Arc::clone(&routes), Arc::clone(&stop));
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => respond(stream, &routes),
                        Err(_) => std::thread::sleep(std::time::Duration::from_millis(5)),
                    }
                }
            })
        };
        Server {
            base,
            routes,
            stop,
            handle: Some(handle),
        }
    }

    fn serve(&self, path: &str, body: Vec<u8>) {
        self.routes.lock().unwrap().insert(path.to_string(), body);
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            handle.join().unwrap();
        }
    }
}

fn respond(mut stream: TcpStream, routes: &Mutex<HashMap<String, Vec<u8>>>) {
    stream.set_nonblocking(false).unwrap();
    let mut head = Vec::new();
    let mut chunk = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => head.extend_from_slice(&chunk[..n]),
        }
    }
    let head = String::from_utf8_lossy(&head);
    let path = head.split_whitespace().nth(1).unwrap_or("");
    let body = routes.lock().unwrap().get(path).cloned();
    let (status, body) = match body {
        Some(body) => ("200 OK", body),
        None => ("404 Not Found", Vec::new()),
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(&body);
}

/// Packs the plugin, serves it from a registry, and returns a project depending on it.
fn consumer(dir: &Path, server: &Server, source: &str) {
    let plugin_dir = dir.join("plugin");
    plugin(&plugin_dir);
    let packed = pack(&plugin_dir, "out");
    let archive = std::fs::read(plugin_dir.join("out/packed-1.2.3.rpp.tgz")).unwrap();
    server.serve("/archives/packed-1.2.3.rpp.tgz", archive);
    server.serve(
        "/plugins/packed.json",
        serde_json::to_vec(&json!({
            "name": "packed",
            "repository": "https://example.com/packed",
            "description": "A packed plugin",
            "versions": [{
                "version": "1.2.3",
                "url": format!("{}/archives/packed-1.2.3.rpp.tgz", server.base),
                "sha256": packed["sha256"],
                "rpp": ">=0.1",
            }],
        }))
        .unwrap(),
    );

    let project = dir.join("project");
    write(
        &project,
        "rpp.json",
        r#"{ "dependencies": { "packed": "^1.2.3" } }"#,
    );
    write(
        &project,
        "rpp.config.ts",
        r##"import { defineConfig } from "#rpp/config";
import packed from "#plugins/packed";

export default defineConfig({
  pack: { name: "p" },
  plugins: [packed({ text: "x" })],
});
"##,
    );
    write(&project, "src/a.txt", source);
}

#[test]
fn packed_plugin_installs_from_registry() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start();
    consumer(dir.path(), &server, "hello");
    let project = dir.path().join("project");

    let out = common::command(&project)
        .env("RPP_REGISTRY", &server.base)
        .args(["build", "--no-squash"])
        .output()
        .unwrap();

    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        std::fs::read_to_string(project.join("dist/a.txt")).unwrap(),
        "hello!"
    );
}

#[test]
fn packed_plugin_error_maps_to_original_source() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::start();
    consumer(dir.path(), &server, "explode");
    let project = dir.path().join("project");

    let out = common::command(&project)
        .env("RPP_REGISTRY", &server.base)
        .args(["build", "--no-squash"])
        .output()
        .unwrap();

    assert!(!out.status.success());
    let message = stderr(&out);
    assert!(message.contains("boom from helper"), "{message}");
    assert!(message.contains("src/helper.ts:3:"), "{message}");
    assert!(!message.contains("dist/"), "{message}");
}
