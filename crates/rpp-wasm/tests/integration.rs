//! Integration tests for the rpp-wasm component host.
//!
//! These tests build two guest crates (the `grayscale-wasm` example and the
//! `edge-plugin` fixture) for `wasm32-wasip2`. If that target is not installed
//! and cannot be added, the affected tests **skip** (with an `eprintln!`)
//! rather than fail, so the suite stays green on CI runners without the target.

use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{Mutex, OnceLock},
    time::Duration,
};

use rpp_wasm::{CompiledPlugin, Error, HostCallbacks, Limits, LogLevel, ProcessResult, WasmEngine};

/// `true` if the `wasm32-wasip2` target is available (installing it once if
/// needed). Memoized; the install attempt happens at most once per test run.
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

/// Build a guest crate at `crate_dir` for `wasm32-wasip2` (release) into a
/// **separate** target dir under that crate, avoiding the shared workspace
/// target-dir lock. Returns the path to the produced `.wasm` component.
///
/// Builds are serialized across tests via a mutex so two tests don't invoke
/// cargo on the same crate concurrently.
fn build_guest(crate_dir: &Path, wasm_stem: &str) -> PathBuf {
    static BUILD_LOCK: Mutex<()> = Mutex::new(());
    let _guard = BUILD_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let target_dir = crate_dir.join("target-test");
    let status = Command::new("cargo")
        .current_dir(crate_dir)
        .env("CARGO_TARGET_DIR", &target_dir)
        .args(["build", "--release", "--target", "wasm32-wasip2"])
        .status()
        .expect("failed to spawn cargo");
    assert!(status.success(), "guest build failed in {crate_dir:?}");

    let wasm = target_dir
        .join("wasm32-wasip2")
        .join("release")
        .join(format!("{wasm_stem}.wasm"));
    assert!(wasm.is_file(), "expected component at {wasm:?}");
    wasm
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn grayscale_dir() -> PathBuf {
    manifest_dir().join("../../examples/plugins/grayscale-wasm")
}

fn edge_dir() -> PathBuf {
    manifest_dir().join("tests/fixtures/edge-plugin")
}

/// A recording host: captures logs and emits/removes, and serves a small
/// virtual filesystem for read/list.
#[derive(Default)]
struct RecordingHost {
    logs: Vec<(LogLevel, String)>,
    emitted: Vec<(String, Vec<u8>)>,
    removed: Vec<String>,
    files: Vec<(String, Vec<u8>)>,
    sources: Vec<(String, Vec<u8>)>,
}

impl HostCallbacks for RecordingHost {
    fn log(&mut self, level: LogLevel, message: &str) {
        self.logs.push((level, message.to_string()));
    }
    fn list_files(&mut self, _pattern: Option<&str>) -> Vec<String> {
        self.files.iter().map(|(p, _)| p.clone()).collect()
    }
    fn read_file(&mut self, path: &str) -> Option<Vec<u8>> {
        self.files
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, c)| c.clone())
    }
    fn read_source(&mut self, path: &str) -> Option<Vec<u8>> {
        self.sources
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, c)| c.clone())
    }
    fn emit_file(&mut self, path: &str, contents: Vec<u8>) {
        self.emitted.push((path.to_string(), contents));
    }
    fn remove_file(&mut self, path: &str) {
        self.removed.push(path.to_string());
    }
}

/// A host that shares its recorded state behind an `Arc<Mutex<…>>` so the test
/// can inspect what the guest did after the instance is consumed.
#[derive(Clone, Default)]
struct SharedHost(std::sync::Arc<Mutex<RecordingHost>>);

impl HostCallbacks for SharedHost {
    fn log(&mut self, level: LogLevel, message: &str) {
        self.0.lock().unwrap().log(level, message);
    }
    fn list_files(&mut self, pattern: Option<&str>) -> Vec<String> {
        self.0.lock().unwrap().list_files(pattern)
    }
    fn read_file(&mut self, path: &str) -> Option<Vec<u8>> {
        self.0.lock().unwrap().read_file(path)
    }
    fn read_source(&mut self, path: &str) -> Option<Vec<u8>> {
        self.0.lock().unwrap().read_source(path)
    }
    fn emit_file(&mut self, path: &str, contents: Vec<u8>) {
        self.0.lock().unwrap().emit_file(path, contents)
    }
    fn remove_file(&mut self, path: &str) {
        self.0.lock().unwrap().remove_file(path)
    }
}

/// Build a tiny 2x2 RGB PNG in memory.
fn tiny_png() -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut buf, 2, 2);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("png header");
        // Four colored pixels (red, green, blue, white).
        let data: [u8; 12] = [
            255, 0, 0, // red
            0, 255, 0, // green
            0, 0, 255, // blue
            255, 255, 255, // white
        ];
        writer.write_image_data(&data).expect("png data");
    }
    buf
}

/// Decode a PNG and return (width, height, color-type).
fn png_info(bytes: &[u8]) -> (u32, u32, png::ColorType) {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let reader = decoder.read_info().expect("decode png");
    let info = reader.info();
    (info.width, info.height, info.color_type)
}

fn load(engine: &WasmEngine, wasm: &Path) -> CompiledPlugin {
    engine.load(wasm).expect("load component")
}

#[test]
fn grayscale_happy_path() {
    if !wasip2_available() {
        eprintln!("SKIP grayscale_happy_path: wasm32-wasip2 unavailable");
        return;
    }
    let wasm = build_guest(&grayscale_dir(), "grayscale_wasm");
    let engine = WasmEngine::new().expect("engine");
    let plugin = load(&engine, &wasm);

    // PluginInfo assertions.
    let info = plugin.info();
    assert_eq!(info.id, "grayscale");
    assert_eq!(info.version, "0.1.0");
    assert!(info.has_generator);
    assert_eq!(info.processors.len(), 1);
    let p = &info.processors[0];
    assert_eq!(p.name, "grayscale");
    assert_eq!(p.patterns, vec!["**/*.gray.png".to_string()]);
    assert_eq!(p.priority, 0);

    let mut inst = plugin
        .instantiate("{}", RecordingHost::default())
        .expect("instantiate");

    let png = tiny_png();
    let (_, _, ct) = png_info(&png);
    assert_eq!(ct, png::ColorType::Rgb, "fixture starts as RGB");

    let result = inst
        .process("grayscale", "assets/logo.gray.png", &png)
        .expect("process");

    match result {
        ProcessResult::Modified { path, contents } => {
            // Renamed: .gray stripped.
            assert_eq!(path, "assets/logo.png");
            // Re-encoded as grayscale.
            let (w, h, ct) = png_info(&contents);
            assert_eq!((w, h), (2, 2));
            assert_eq!(ct, png::ColorType::Grayscale, "output is grayscale");
        }
        other => panic!("expected Modified, got {other:?}"),
    }

    // Generator emits a report.
    let host = SharedHost::default();
    let mut inst2 = plugin
        .instantiate("{}", host.clone())
        .expect("instantiate 2");
    inst2
        .process("grayscale", "a.gray.png", &png)
        .expect("process a");
    inst2.generate().expect("generate");
    let recorded = host.0.lock().unwrap();
    let report = recorded
        .emitted
        .iter()
        .find(|(p, _)| p == "grayscale_report.json")
        .expect("report emitted");
    let text = String::from_utf8_lossy(&report.1);
    assert!(text.contains("\"processed\":1"), "report: {text}");
}

fn edge_plugin(engine: &WasmEngine) -> CompiledPlugin {
    let wasm = build_guest(&edge_dir(), "edge_plugin");
    load(engine, &wasm)
}

#[test]
fn edge_unchanged_dropped_and_error() {
    if !wasip2_available() {
        eprintln!("SKIP edge_unchanged_dropped_and_error: wasm32-wasip2 unavailable");
        return;
    }
    let engine = WasmEngine::new().expect("engine");
    let plugin = edge_plugin(&engine);
    assert_eq!(plugin.info().id, "edge");

    let mut inst = plugin
        .instantiate("{}", RecordingHost::default())
        .expect("instantiate");

    assert_eq!(
        inst.process("keep", "f.txt", b"hi").expect("keep"),
        ProcessResult::Unchanged
    );
    assert_eq!(
        inst.process("drop", "f.txt", b"hi").expect("drop"),
        ProcessResult::Dropped
    );
    match inst.process("fail", "f.txt", b"hi") {
        Err(Error::GuestError(msg)) => assert_eq!(msg, "boom"),
        other => panic!("expected GuestError(boom), got {other:?}"),
    }
}

#[test]
fn host_callbacks_suppressed_outside_generate() {
    if !wasip2_available() {
        eprintln!("SKIP host_callbacks_suppressed_outside_generate: wasm32-wasip2 unavailable");
        return;
    }
    let engine = WasmEngine::new().expect("engine");
    let plugin = edge_plugin(&engine);

    let host = SharedHost::default();
    // Pre-populate the virtual fs; the guest must still see nothing during process.
    host.0
        .lock()
        .unwrap()
        .files
        .push(("present.txt".into(), b"data".to_vec()));

    let mut inst = plugin.instantiate("{}", host.clone()).expect("instantiate");
    // `probe` reports what the host returned during the process phase.
    match inst.process("probe", "x", b"") {
        Err(Error::GuestError(msg)) => {
            assert_eq!(msg, "probe: listed=0 read=false src=false", "{msg}");
        }
        other => panic!("expected probe GuestError, got {other:?}"),
    }
    // emit/remove during process must have been suppressed.
    let recorded = host.0.lock().unwrap();
    assert!(
        recorded.emitted.is_empty(),
        "emit suppressed outside generate"
    );
    assert!(
        recorded.removed.is_empty(),
        "remove suppressed outside generate"
    );
    // log is always allowed.
    assert!(recorded.logs.iter().any(|(_, m)| m == "probe ran"));
}

#[test]
fn generate_emits_files() {
    if !wasip2_available() {
        eprintln!("SKIP generate_emits_files: wasm32-wasip2 unavailable");
        return;
    }
    let engine = WasmEngine::new().expect("engine");
    let plugin = edge_plugin(&engine);

    let host = SharedHost::default();
    let mut inst = plugin.instantiate("{}", host.clone()).expect("instantiate");
    inst.generate().expect("generate");

    let recorded = host.0.lock().unwrap();
    let names: Vec<&str> = recorded.emitted.iter().map(|(p, _)| p.as_str()).collect();
    assert!(names.contains(&"gen_a.txt"), "{names:?}");
    assert!(names.contains(&"gen_b.txt"), "{names:?}");
}

#[test]
fn timeout_via_infinite_loop() {
    if !wasip2_available() {
        eprintln!("SKIP timeout_via_infinite_loop: wasm32-wasip2 unavailable");
        return;
    }
    // Short deadline so the test is fast.
    let engine = WasmEngine::with_limits(Limits {
        deadline: Duration::from_millis(200),
        ..Limits::default()
    })
    .expect("engine");
    let plugin = edge_plugin(&engine);
    let mut inst = plugin
        .instantiate("{}", RecordingHost::default())
        .expect("instantiate");

    match inst.process("spin", "x", b"") {
        Err(Error::Timeout(_)) => {}
        other => panic!("expected Timeout, got {other:?}"),
    }
}

#[test]
fn memory_limit_traps() {
    if !wasip2_available() {
        eprintln!("SKIP memory_limit_traps: wasm32-wasip2 unavailable");
        return;
    }
    // Tiny memory cap so the allocation bomb is denied.
    let engine = WasmEngine::with_limits(Limits {
        memory_bytes: 16 * 1024 * 1024,
        ..Limits::default()
    })
    .expect("engine");
    let plugin = edge_plugin(&engine);
    let mut inst = plugin
        .instantiate("{}", RecordingHost::default())
        .expect("instantiate");

    // The guest tries to allocate 1 TiB; growth past the cap must fail. The
    // guest reports either a trap (memory.grow trap via StoreLimits) or a
    // handled allocation failure string.
    match inst.process("bomb", "x", b"") {
        Err(Error::Trap(_)) | Err(Error::GuestError(_)) => {}
        other => panic!("expected trap or guest error from memory bomb, got {other:?}"),
    }
}

#[test]
fn rejects_invalid_component_path() {
    let engine = WasmEngine::new().expect("engine");
    let missing = manifest_dir().join("tests/does-not-exist.wasm");
    match engine.load(&missing) {
        Err(Error::Io { .. }) => {}
        Err(other) => panic!("expected Io error, got {other:?}"),
        Ok(_) => panic!("expected Io error, got Ok"),
    }
}
