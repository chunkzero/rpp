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
    static PROCESS: OnceLock<Option<BuiltFixture>> = OnceLock::new();
    let fixture = match stem {
        "math_component" => &MATH,
        "process_component" => &PROCESS,
        _ => panic!("unknown component fixture `{stem}`"),
    };
    fixture
        .get_or_init(|| compile_fixture(crate_dir, stem))
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
    let engine = WasmEngine::new().unwrap();
    let component = engine.load(&fixture.wasm).unwrap();
    assert!(component.schema().functions.iter().any(|f| f.path == "add"));

    let mut instance = component.instantiate(Permissions::default()).unwrap();
    let results = instance
        .call("add", &[Value::S32(2), Value::S32(40)])
        .unwrap();
    assert_eq!(results, vec![Value::S32(42)]);
}

#[test]
fn process_import_requires_permission() {
    let Some(fixture) = build_fixture(&fixture("process-component"), "process_component") else {
        return;
    };
    let engine = WasmEngine::new().unwrap();
    let component = engine.load(&fixture.wasm).unwrap();

    match component.instantiate(Permissions::default()) {
        Err(Error::DeniedCapability(name)) => assert!(name.contains("rpp:host/process"), "{name}"),
        Ok(_) => panic!("expected denied process import, got Ok"),
        Err(other) => panic!("expected denied process import, got {other:?}"),
    }
}

#[test]
fn process_import_runs_when_granted() {
    let Some(fixture) = build_fixture(&fixture("process-component"), "process_component") else {
        return;
    };
    let engine = WasmEngine::new().unwrap();
    let component = engine.load(&fixture.wasm).unwrap();

    let mut instance = component
        .instantiate(Permissions {
            processes: vec!["/bin/echo".into()],
            ..Permissions::default()
        })
        .unwrap();
    let results = instance
        .call("run-echo", &[Value::String("/bin/echo".into())])
        .unwrap();
    assert_eq!(
        results,
        vec![Value::Result(Ok(Some(Box::new(Value::String(
            "hi".into()
        )))))]
    );
}

#[test]
fn timeout_interrupts_guest() {
    let Some(fixture) = build_fixture(&fixture("math-component"), "math_component") else {
        return;
    };
    let engine = WasmEngine::with_limits(Limits {
        deadline: Duration::from_millis(100),
        ..Limits::default()
    })
    .unwrap();
    let component = engine.load(&fixture.wasm).unwrap();
    let mut instance = component.instantiate(Permissions::default()).unwrap();
    match instance.call("spin", &[]) {
        Err(Error::Timeout(_)) => {}
        other => panic!("expected timeout, got {other:?}"),
    }
}
