mod common;

use std::time::Duration;

use rpp_js::{Call, Cancellation, Clock, Engine, Error, Limits};
use serde_json::{json, Value};

use common::{call, load, try_load, NoHost};

fn limits(heap_mib: usize, time: Duration) -> Limits {
    Limits {
        heap_bytes: heap_mib * 1024 * 1024,
        time,
    }
}

#[test]
fn infinite_loop_hits_deadline_and_runtime_stays_terminated() {
    let engine = Engine::new().unwrap();
    let code = "export const spin = () => { for (;;) {} }; export const ok = () => 1;";
    let mut runtime = try_load(&engine, code, limits(32, Duration::from_millis(200))).unwrap();
    let error = call(&mut runtime, &engine, "spin", Value::Null).unwrap_err();
    assert!(matches!(error, Error::Deadline), "{error:?}");
    assert!(runtime.is_terminated());
    let error = call(&mut runtime, &engine, "ok", Value::Null).unwrap_err();
    assert!(matches!(error, Error::Terminated), "{error:?}");
    let mut fresh = load(&engine, code);
    assert_eq!(
        call(&mut fresh, &engine, "ok", Value::Null).unwrap(),
        json!(1)
    );
}

#[test]
fn heap_limit_terminates() {
    let engine = Engine::new().unwrap();
    let mut runtime = try_load(
        &engine,
        "export const fill = () => { const all = []; for (;;) all.push(new Array(1_000_000).fill(1)); };",
        limits(16, Duration::from_secs(20)),
    )
    .unwrap();
    let error = call(&mut runtime, &engine, "fill", Value::Null).unwrap_err();
    assert!(matches!(error, Error::Heap), "{error:?}");
    assert!(runtime.is_terminated());
    let mut runtime = try_load(
        &engine,
        "export const buffers = () => { const all = []; for (;;) all.push(new Uint8Array(8_000_000)); };",
        limits(16, Duration::from_secs(20)),
    )
    .unwrap();
    let error = call(&mut runtime, &engine, "buffers", Value::Null).unwrap_err();
    assert!(matches!(error, Error::Heap), "{error:?}");
    let small = try_load(&engine, "", limits(8, Duration::from_secs(1)))
        .err()
        .unwrap();
    assert!(matches!(small, Error::Invalid(_)));
}

#[test]
fn cancellation_from_another_thread_terminates() {
    let engine = Engine::new().unwrap();
    let mut runtime = try_load(
        &engine,
        "export const spin = () => { for (;;) {} };",
        limits(32, Duration::from_secs(30)),
    )
    .unwrap();
    let cancellation = Cancellation::new();
    let remote = cancellation.clone();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        remote.cancel();
    });
    let call = Call {
        export: "spin",
        args: Value::Null,
        bytes: None,
        clock: Clock::default(),
    };
    let error = runtime
        .call(&engine, call, &mut NoHost, &cancellation)
        .unwrap_err();
    thread.join().unwrap();
    assert!(matches!(error, Error::Cancelled), "{error:?}");
    assert!(runtime.is_terminated());
}

#[test]
fn hostile_prepare_stack_trace_cannot_outlive_deadline() {
    let engine = Engine::new().unwrap();
    let code = "export const spin = () => {
        Error.prepareStackTrace = () => { for (;;) {} };
        for (;;) {}
    };";
    let mut runtime = try_load(&engine, code, limits(32, Duration::from_millis(200))).unwrap();
    let started = std::time::Instant::now();
    let error = call(&mut runtime, &engine, "spin", Value::Null).unwrap_err();
    assert!(matches!(error, Error::Deadline), "{error:?}");
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn discarded_buffers_beyond_budget_do_not_terminate() {
    let engine = Engine::new().unwrap();
    let code = "export const churn = () => {
        let total = 0;
        for (let i = 0; i < 40; i++) total += new Uint8Array(4_000_000).length;
        return total;
    };";
    let mut runtime = try_load(&engine, code, limits(16, Duration::from_secs(20))).unwrap();
    assert_eq!(
        call(&mut runtime, &engine, "churn", Value::Null).unwrap(),
        json!(160_000_000)
    );
    assert!(!runtime.is_terminated());
}
