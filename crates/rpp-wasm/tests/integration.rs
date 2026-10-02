//! Integration tests for dynamic component libraries.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

use rpp_wasm::{Error, Limits, Permissions, Value, WasmEngine};

fn target_installed() -> bool {
    let Ok(output) = Command::new("rustc").args(["--print", "sysroot"]).output() else {
        return false;
    };
    Path::new(String::from_utf8_lossy(&output.stdout).trim())
        .join("lib/rustlib/wasm32-wasip2/lib")
        .is_dir()
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

struct BuiltFixture {
    _target: tempfile::TempDir,
    wasm: PathBuf,
}

fn build_fixture(crate_dir: &Path, stem: &str) -> Option<&'static BuiltFixture> {
    static MATH: OnceLock<Option<BuiltFixture>> = OnceLock::new();
    MATH.get_or_init(|| compile_fixture(crate_dir, stem))
        .as_ref()
}

fn compile_fixture(crate_dir: &Path, stem: &str) -> Option<BuiltFixture> {
    if !target_installed() {
        eprintln!("SKIP: wasm32-wasip2 unavailable");
        return None;
    }
    let target = tempfile::tempdir().expect("temporary fixture target");
    let status = Command::new("cargo")
        .current_dir(crate_dir)
        .env("CARGO_TARGET_DIR", target.path())
        .args([
            "build",
            "--locked",
            "--release",
            "--target",
            "wasm32-wasip2",
        ])
        .status()
        .expect("spawn cargo");
    assert!(status.success(), "guest build failed in {crate_dir:?}");
    Some(BuiltFixture {
        wasm: target
            .path()
            .join("wasm32-wasip2")
            .join("release")
            .join(format!("{stem}.wasm")),
        _target: target,
    })
}

#[test]
fn schema_and_dynamic_call() {
    let Some(fixture) = build_fixture(&fixture("math-component"), "math_component") else {
        return;
    };
    let engine = WasmEngine::new(Limits::default(), None).unwrap();
    let component = engine.load(&fixture.wasm).unwrap();
    assert!(component.schema().functions.iter().any(|f| f.path == "add"));

    let mut instance = component.instantiate(Permissions::default()).unwrap();
    let results = instance
        .call("add", &[Value::S32(2), Value::S32(40)])
        .unwrap();
    assert_eq!(results, vec![Value::S32(42)]);
}

#[test]
fn timeout_interrupts_guest() {
    let Some(fixture) = build_fixture(&fixture("math-component"), "math_component") else {
        return;
    };
    let engine = WasmEngine::new(
        Limits {
            deadline: Duration::from_millis(100),
            ..Limits::default()
        },
        None,
    )
    .unwrap();
    let component = engine.load(&fixture.wasm).unwrap();
    let mut instance = component.instantiate(Permissions::default()).unwrap();
    match instance.call("spin", &[]) {
        Err(Error::Timeout(_)) => {}
        other => panic!("expected timeout, got {other:?}"),
    }
}

#[test]
fn call_with_deadline_caps_timeout() {
    let Some(fixture) = build_fixture(&fixture("math-component"), "math_component") else {
        return;
    };
    let engine = WasmEngine::new(Limits::default(), None).unwrap();
    let component = engine.load(&fixture.wasm).unwrap();
    let mut instance = component.instantiate(Permissions::default()).unwrap();
    let limit = Duration::from_millis(100);
    match instance.call_with_deadline("spin", &[], limit) {
        Err(Error::Timeout(reported)) => assert_eq!(reported, limit),
        other => panic!("expected timeout, got {other:?}"),
    }
}

#[test]
fn sleeping_returns_immediately_on_fixed_clock() {
    let Some(fixture) = build_fixture(&fixture("math-component"), "math_component") else {
        return;
    };
    let engine = WasmEngine::new(Limits::default(), None).unwrap();
    let component = engine.load(&fixture.wasm).unwrap();
    let mut instance = component.instantiate(Permissions::default()).unwrap();
    let started = std::time::Instant::now();
    let results = instance.call("sleep-hour", &[]).unwrap();
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(results, vec![Value::U64(0)]);
}
