use rpp_js::{
    bundle, BundleRequest, Call, Cancellation, Clock, Engine, Error, Host, HostReply, Limits,
};
use serde_json::{json, Value};

struct NoHost;

impl Host for NoHost {
    fn call(&mut self, _: &str, _: Value, _: Option<Vec<u8>>) -> Result<HostReply, String> {
        Err("no host".into())
    }
}

fn plugin(files: &[(&str, &str)]) -> (tempfile::TempDir, rpp_js::Bundle) {
    let dir = tempfile::tempdir().unwrap();
    for (path, source) in files {
        let path = dir.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    let bundle = bundle(&BundleRequest {
        root: dir.path().to_path_buf(),
        entry: "src/plugin.ts".into(),
        ..Default::default()
    })
    .unwrap();
    (dir, bundle)
}

fn stack_of(error: Error) -> String {
    match error {
        Error::JavaScript { stack, .. } => stack,
        other => panic!("expected a JavaScript error, got {other:?}"),
    }
}

#[test]
fn call_errors_point_at_original_sources() {
    let (_dir, bundle) = plugin(&[
        (
            "src/plugin.ts",
            "import { fail } from \"./helper.ts\";\n\nexport function run(_: unknown): void {\n  fail(\"boom\");\n}\n",
        ),
        (
            "src/helper.ts",
            "interface Unused { a: number }\n\nexport function fail(message: string): never {\n  throw new Error(message);\n}\n",
        ),
    ]);
    let engine = Engine::new().unwrap();
    let cancel = Cancellation::new();
    let (mut runtime, _) = engine
        .load(
            "plugin",
            &bundle,
            Limits::default(),
            Clock::default(),
            &cancel,
        )
        .unwrap();
    let call = Call {
        export: "run",
        args: json!(null),
        bytes: None,
        clock: Clock::default(),
    };
    let stack = stack_of(
        runtime
            .call(&engine, call, &mut NoHost, &cancel)
            .unwrap_err(),
    );
    assert!(stack.contains("src/helper.ts:4:"), "{stack}");
    assert!(stack.contains("src/plugin.ts:4:"), "{stack}");
}

#[test]
fn evaluation_errors_point_at_original_sources() {
    let (_dir, bundle) = plugin(&[(
        "src/plugin.ts",
        "type Id = string;\n\nfunction init(id: Id): void {\n  throw new TypeError(id);\n}\n\ninit(\"bad\");\nexport const ok = 1;\n",
    )]);
    let engine = Engine::new().unwrap();
    let cancel = Cancellation::new();
    let error = engine
        .load(
            "plugin",
            &bundle,
            Limits::default(),
            Clock::default(),
            &cancel,
        )
        .err()
        .expect("evaluation should throw");
    let stack = stack_of(error);
    assert!(stack.contains("src/plugin.ts:4:"), "{stack}");
}
