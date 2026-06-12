//! Integration tests for the `wasm` feature adapter
//! ([`rpp::wasm::WasmPluginFactory`]).
//!
//! These build the `edge-plugin` test fixture (shared with `rpp-wasm`'s own
//! tests) for `wasm32-wasip2` into a separate target dir, then drive it through
//! the core [`PluginFactory`]/[`PluginInstance`] traits. If the target is not
//! installed and cannot be added, every test **skips** rather than fails so the
//! suite stays green on runners without the toolchain.

#![cfg(feature = "wasm")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use rpp::manifest::PluginManifest;
use rpp::model::{GeneratorHost, PackFile, PluginFactory, ProcessOutcome};
use rpp::wasm::WasmPluginFactory;
use rpp_wasm::WasmEngine;

/// `true` if `wasm32-wasip2` is available (installing it once if needed).
fn wasip2_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        if target_installed() {
            return true;
        }
        eprintln!("wasm32-wasip2 not installed; attempting `rustup target add`...");
        let added = Command::new("rustup")
            .args(["target", "add", "wasm32-wasip2"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        added && target_installed()
    })
}

fn target_installed() -> bool {
    Command::new("rustc")
        .args(["--print", "target-libdir", "--target", "wasm32-wasip2"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn edge_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rpp-wasm/tests/fixtures/edge-plugin")
}

/// Build the edge-plugin guest into its own `target-test` dir and return the
/// produced `.wasm` path. Serialized across tests via a mutex.
fn build_edge() -> PathBuf {
    static BUILD_LOCK: Mutex<()> = Mutex::new(());
    let _guard = BUILD_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let crate_dir = edge_dir();
    let target_dir = crate_dir.join("target-test");
    let status = Command::new("cargo")
        .current_dir(&crate_dir)
        .env("CARGO_TARGET_DIR", &target_dir)
        .args(["build", "--release", "--target", "wasm32-wasip2"])
        .status()
        .expect("failed to spawn cargo");
    assert!(status.success(), "guest build failed in {crate_dir:?}");

    let wasm = target_dir
        .join("wasm32-wasip2")
        .join("release")
        .join("edge_plugin.wasm");
    assert!(wasm.is_file(), "expected component at {wasm:?}");
    wasm
}

/// Build a plugin package directory in `dir`: copy `wasm` to `plugin.wasm` and
/// write a `plugin.toml` referencing it.
fn make_package(dir: &Path, wasm: &Path) {
    std::fs::copy(wasm, dir.join("plugin.wasm")).expect("copy wasm");
    std::fs::write(
        dir.join("plugin.toml"),
        "[plugin]\nid = \"edge\"\nversion = \"0.1.0\"\nruntime = \"wasm\"\nmodule = \"plugin.wasm\"\n",
    )
    .expect("write plugin.toml");
}

fn load_factory(dir: &Path, engine: &WasmEngine, options: toml::Value) -> WasmPluginFactory {
    let manifest = PluginManifest::load(dir).expect("load manifest");
    WasmPluginFactory::load(engine, dir, &manifest, options).expect("load factory")
}

/// A simple in-memory [`GeneratorHost`] for driving the generate bridge.
#[derive(Default)]
struct TestHost {
    files: BTreeMap<String, Vec<u8>>,
    sources: BTreeMap<String, Vec<u8>>,
    emitted: Vec<(String, Vec<u8>)>,
    removed: Vec<String>,
    listed: Vec<Option<String>>,
}

impl GeneratorHost for TestHost {
    fn list_files(&mut self, glob: Option<&str>) -> Vec<String> {
        self.listed.push(glob.map(str::to_string));
        self.files.keys().cloned().collect()
    }
    fn read_file(&mut self, path: &str) -> Option<Vec<u8>> {
        self.files.get(path).cloned()
    }
    fn read_source(&mut self, path: &str) -> Option<Vec<u8>> {
        self.sources.get(path).cloned()
    }
    fn emit(&mut self, path: &str, contents: Vec<u8>) {
        self.emitted.push((path.to_string(), contents));
    }
    fn remove(&mut self, path: &str) {
        self.removed.push(path.to_string());
    }
}

#[test]
fn factory_exposes_info() {
    if !wasip2_available() {
        eprintln!("SKIP factory_exposes_info: wasm32-wasip2 unavailable");
        return;
    }
    let wasm = build_edge();
    let dir = tempfile::tempdir().unwrap();
    make_package(dir.path(), &wasm);

    let engine = WasmEngine::new().expect("engine");
    let factory = load_factory(dir.path(), &engine, empty_options());

    assert_eq!(factory.id(), "edge");
    assert_eq!(factory.version(), "0.1.0");
    assert!(factory.has_generator());
    let names: Vec<&str> = factory
        .processors()
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    assert!(names.contains(&"keep"));
    assert!(names.contains(&"drop"));
}

#[test]
fn process_outcomes_map_correctly() {
    if !wasip2_available() {
        eprintln!("SKIP process_outcomes_map_correctly: wasm32-wasip2 unavailable");
        return;
    }
    let wasm = build_edge();
    let dir = tempfile::tempdir().unwrap();
    make_package(dir.path(), &wasm);

    let engine = WasmEngine::new().expect("engine");
    let factory = load_factory(dir.path(), &engine, empty_options());
    let mut inst = factory.instantiate().expect("instantiate");

    let mut file = PackFile::new("f.txt", b"hi".to_vec());
    assert_eq!(
        inst.process("keep", &mut file).unwrap(),
        ProcessOutcome::Unchanged
    );
    assert_eq!(
        inst.process("drop", &mut file).unwrap(),
        ProcessOutcome::Dropped
    );

    // A guest error is surfaced as a Processor error with attribution.
    let err = inst.process("fail", &mut file).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("edge"), "{msg}");
    assert!(msg.contains("fail"), "{msg}");
    assert!(msg.contains("boom"), "{msg}");
}

#[test]
fn generate_bridge_services_host_callbacks() {
    if !wasip2_available() {
        eprintln!("SKIP generate_bridge_services_host_callbacks: wasm32-wasip2 unavailable");
        return;
    }
    let wasm = build_edge();
    let dir = tempfile::tempdir().unwrap();
    make_package(dir.path(), &wasm);

    let engine = WasmEngine::new().expect("engine");
    let factory = load_factory(dir.path(), &engine, empty_options());
    let mut inst = factory.instantiate().expect("instantiate");

    let mut host = TestHost::default();
    host.files.insert("a.json".into(), b"{}".to_vec());

    inst.generate(&mut host).expect("generate");

    // The guest emits two files through the channel bridge.
    let names: Vec<&str> = host.emitted.iter().map(|(p, _)| p.as_str()).collect();
    assert!(names.contains(&"gen_a.txt"), "{names:?}");
    assert!(names.contains(&"gen_b.txt"), "{names:?}");
    // The guest's `list_files(Some("**/*.json"))` round-tripped to the host.
    assert_eq!(host.listed, vec![Some("**/*.json".to_string())]);
}

#[test]
fn cache_key_depends_on_options() {
    if !wasip2_available() {
        eprintln!("SKIP cache_key_depends_on_options: wasm32-wasip2 unavailable");
        return;
    }
    let wasm = build_edge();
    let dir = tempfile::tempdir().unwrap();
    make_package(dir.path(), &wasm);

    let engine = WasmEngine::new().expect("engine");

    let mut a = toml::map::Map::new();
    a.insert("flag".into(), toml::Value::Boolean(true));
    let f1 = load_factory(dir.path(), &engine, toml::Value::Table(a));

    let f2 = load_factory(dir.path(), &engine, empty_options());

    // Same module + manifest, different options => different cache key.
    assert_ne!(f1.cache_key(), f2.cache_key());

    // Identical inputs => stable key.
    let f3 = load_factory(dir.path(), &engine, empty_options());
    assert_eq!(f2.cache_key(), f3.cache_key());
}

fn empty_options() -> toml::Value {
    toml::Value::Table(toml::map::Map::new())
}
