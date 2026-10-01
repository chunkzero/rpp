use std::time::Duration;

use rpp_js::{
    Bundle, Call, Cancellation, Clock, Engine, Error, Host, HostReply, Limits, LogLevel, Runtime,
};
use serde_json::{json, Value};

struct NoHost;

impl Host for NoHost {
    fn call(&mut self, name: &str, _: Value, _: Option<Vec<u8>>) -> Result<HostReply, String> {
        Err(format!("unexpected host call {name}"))
    }
}

fn bundle(code: &str) -> Bundle {
    Bundle {
        code: code.to_string(),
        source_map: r#"{"version":3,"sources":[],"names":[],"mappings":""}"#.to_string(),
        inputs: Vec::new(),
    }
}

fn limits(heap_mib: usize, time: Duration) -> Limits {
    Limits {
        heap_bytes: heap_mib * 1024 * 1024,
        time,
    }
}

fn load(engine: &Engine, code: &str) -> Runtime {
    try_load(engine, code, Limits::default()).unwrap()
}

fn try_load(engine: &Engine, code: &str, limits: Limits) -> Result<Runtime, Error> {
    let cancellation = Cancellation::new();
    engine
        .load(
            "plugin.js",
            &bundle(code),
            limits,
            Clock::default(),
            &cancellation,
        )
        .map(|(runtime, _)| runtime)
}

fn call_with(
    runtime: &mut Runtime,
    engine: &Engine,
    export: &str,
    args: Value,
    bytes: Option<Vec<u8>>,
    host: &mut dyn Host,
) -> Result<rpp_js::Output, Error> {
    let call = Call {
        export,
        args,
        bytes,
        clock: Clock::default(),
    };
    runtime.call(engine, call, host, &Cancellation::new())
}

fn call(runtime: &mut Runtime, engine: &Engine, export: &str, args: Value) -> Result<Value, Error> {
    call_with(runtime, engine, export, args, None, &mut NoHost).map(|output| output.value)
}

#[test]
fn calls_export_with_args_and_bytes() {
    let engine = Engine::new().unwrap();
    let mut runtime = load(
        &engine,
        "export const sum = (args, bytes) => ({ total: args.a + args.b, length: bytes.length });
         export const reverse = (_, bytes) => bytes.slice().reverse();
         export const none = (_, bytes) => bytes === undefined;",
    );
    let output = call_with(
        &mut runtime,
        &engine,
        "sum",
        json!({"a": 1, "b": 2}),
        Some(vec![9, 8, 7]),
        &mut NoHost,
    )
    .unwrap();
    assert_eq!(output.value, json!({"total": 3, "length": 3}));
    let output = call_with(
        &mut runtime,
        &engine,
        "reverse",
        Value::Null,
        Some(vec![1, 2, 3]),
        &mut NoHost,
    )
    .unwrap();
    assert_eq!(output.value, Value::Null);
    assert_eq!(output.bytes, Some(vec![3, 2, 1]));
    assert_eq!(
        call(&mut runtime, &engine, "none", Value::Null).unwrap(),
        json!(true)
    );
}

#[test]
fn module_state_persists_between_calls() {
    let engine = Engine::new().unwrap();
    let mut runtime = load(&engine, "let n = 0; export const next = () => ++n;");
    assert_eq!(
        call(&mut runtime, &engine, "next", Value::Null).unwrap(),
        json!(1)
    );
    assert_eq!(
        call(&mut runtime, &engine, "next", Value::Null).unwrap(),
        json!(2)
    );
}

#[test]
fn runtimes_do_not_share_globals() {
    let engine = Engine::new().unwrap();
    let code = "export const set = (v) => { globalThis.shared = v; };
                export const get = () => globalThis.shared ?? null;";
    let mut first = load(&engine, code);
    let mut second = load(&engine, code);
    call(&mut first, &engine, "set", json!("first")).unwrap();
    assert_eq!(
        call(&mut second, &engine, "get", Value::Null).unwrap(),
        Value::Null
    );
    call(&mut second, &engine, "set", json!("second")).unwrap();
    assert_eq!(
        call(&mut first, &engine, "get", Value::Null).unwrap(),
        json!("first")
    );
    assert_eq!(
        call(&mut second, &engine, "get", Value::Null).unwrap(),
        json!("second")
    );
}

#[test]
fn async_export_settles() {
    let engine = Engine::new().unwrap();
    let mut runtime = load(
        &engine,
        "const late = await Promise.resolve(40);
         export const run = async (x) => { await Promise.resolve(); return late + x; };",
    );
    assert_eq!(
        call(&mut runtime, &engine, "run", json!(2)).unwrap(),
        json!(42)
    );
}

#[test]
fn never_settling_promise_errors() {
    let engine = Engine::new().unwrap();
    let mut runtime = load(&engine, "export const run = () => new Promise(() => {});");
    let error = call(&mut runtime, &engine, "run", Value::Null).unwrap_err();
    assert!(matches!(error, Error::JavaScript { .. }), "{error:?}");
    assert!(!runtime.is_terminated());
}

struct Echo;

impl Host for Echo {
    fn call(
        &mut self,
        name: &str,
        value: Value,
        bytes: Option<Vec<u8>>,
    ) -> Result<HostReply, String> {
        if name == "fail" {
            return Err("nope".into());
        }
        let mut bytes = bytes.unwrap_or_default();
        bytes.push(0);
        Ok(HostReply {
            value: json!({"name": name, "value": value}),
            bytes: Some(bytes),
        })
    }
}

#[test]
fn host_call_round_trips_value_and_bytes() {
    let engine = Engine::new().unwrap();
    let mut runtime = load(
        &engine,
        "export const run = () => {
           const reply = __rpp.call('echo', { n: 1 }, new Uint8Array([5, 6]));
           return { value: reply.value, bytes: Array.from(reply.bytes) };
         };
         export const bare = () => __rpp.call('echo', null).bytes.length;
         export const enumerable = () => Object.keys(globalThis).includes('__rpp');",
    );
    let output = call_with(&mut runtime, &engine, "run", Value::Null, None, &mut Echo).unwrap();
    assert_eq!(
        output.value,
        json!({"value": {"name": "echo", "value": {"n": 1}}, "bytes": [5, 6, 0]})
    );
    let output = call_with(&mut runtime, &engine, "bare", Value::Null, None, &mut Echo).unwrap();
    assert_eq!(output.value, json!(1));
    let output = call_with(
        &mut runtime,
        &engine,
        "enumerable",
        Value::Null,
        None,
        &mut Echo,
    );
    assert_eq!(output.unwrap().value, json!(false));
}

#[test]
fn host_unavailable_during_evaluation() {
    let engine = Engine::new().unwrap();
    let error = try_load(&engine, "__rpp.call('echo', null);", Limits::default())
        .err()
        .unwrap();
    match error {
        Error::JavaScript { message, .. } => {
            assert!(message.contains("host functions are unavailable during module evaluation"));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn host_error_becomes_exception() {
    let engine = Engine::new().unwrap();
    let mut runtime = load(
        &engine,
        "export const run = () => {
           try { __rpp.call('fail', null); } catch (e) { return [e instanceof Error, e.message]; }
         };",
    );
    let output = call_with(&mut runtime, &engine, "run", Value::Null, None, &mut Echo).unwrap();
    assert_eq!(output.value, json!([true, "nope"]));
}

#[test]
fn fixed_clock_is_deterministic() {
    let engine = Engine::new().unwrap();
    let code = "export const sample = () => [Date.now(), new Date().getTime(), Math.random(), Math.random()];";
    let clock = Clock::Fixed {
        timestamp_ms: 1_700_000_000_000,
        seed: 7,
    };
    let sample = |seed_clock: Clock| {
        let mut runtime = load(&engine, code);
        let call = Call {
            export: "sample",
            args: Value::Null,
            bytes: None,
            clock: seed_clock,
        };
        runtime
            .call(&engine, call, &mut NoHost, &Cancellation::new())
            .unwrap()
            .value
    };
    let first = sample(clock);
    assert_eq!(first, sample(clock));
    assert_eq!(first[0], json!(1_700_000_000_000_i64));
    assert_eq!(first[1], json!(1_700_000_000_000_i64));
    assert_ne!(
        first,
        sample(Clock::Fixed {
            timestamp_ms: 1_700_000_000_000,
            seed: 8
        })
    );
    let real = sample(Clock::Real);
    assert_ne!(real[0], json!(1_700_000_000_000_i64));
}

#[test]
fn console_output_is_captured() {
    let engine = Engine::new().unwrap();
    let (mut runtime, logs) = engine
        .load(
            "plugin.js",
            &bundle(
                "console.log('loaded');
                 export const run = () => {
                   console.debug('a', 1, { b: 2 });
                   console.warn('w');
                   console.error(1n);
                   console.info('i');
                 };
                 export const flood = () => { for (;;) console.log('x'); };",
            ),
            Limits::default(),
            Clock::default(),
            &Cancellation::new(),
        )
        .unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(
        (logs[0].level, logs[0].message.as_str()),
        (LogLevel::Info, "loaded")
    );
    let output = call_with(&mut runtime, &engine, "run", Value::Null, None, &mut NoHost).unwrap();
    let logs: Vec<_> = output
        .logs
        .iter()
        .map(|log| (log.level, log.message.as_str()))
        .collect();
    assert_eq!(
        logs,
        [
            (LogLevel::Debug, "a 1 {\"b\":2}"),
            (LogLevel::Warn, "w"),
            (LogLevel::Error, "1"),
            (LogLevel::Info, "i"),
        ]
    );
    let error = call(&mut runtime, &engine, "flood", Value::Null).unwrap_err();
    assert!(
        matches!(&error, Error::JavaScript { message, .. } if message.starts_with("RangeError"))
    );
}

#[test]
fn missing_export_errors() {
    let engine = Engine::new().unwrap();
    let mut runtime = load(&engine, "export const value = 1;");
    for export in ["absent", "value"] {
        let error = call(&mut runtime, &engine, export, Value::Null).unwrap_err();
        assert!(
            matches!(&error, Error::MissingExport(name) if name == export),
            "{error:?}"
        );
    }
}

#[test]
fn non_json_result_is_invalid() {
    let engine = Engine::new().unwrap();
    let mut runtime = load(
        &engine,
        "export const fn = () => () => 1;
         export const symbol = () => ({ a: Symbol('x') });
         export const big = () => 1n;
         export const nan = () => [NaN];
         export const cycle = () => { const a = {}; a.a = a; return a; };
         export const fine = () => ({ a: undefined, b: [1, 'x', null] });
         export const boom = () => { throw new TypeError('bad'); };",
    );
    for export in ["fn", "symbol", "big", "nan", "cycle"] {
        let error = call(&mut runtime, &engine, export, Value::Null).unwrap_err();
        assert!(matches!(error, Error::Invalid(_)), "{export}: {error:?}");
    }
    assert_eq!(
        call(&mut runtime, &engine, "fine", Value::Null).unwrap(),
        json!({"b": [1, "x", null]})
    );
    match call(&mut runtime, &engine, "boom", Value::Null).unwrap_err() {
        Error::JavaScript { message, stack } => {
            assert_eq!(message, "TypeError: bad");
            assert!(
                stack.contains("plugin.js") || stack.contains("rpp:"),
                "{stack}"
            );
        }
        other => panic!("{other:?}"),
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
fn forbidden_globals_are_absent() {
    let engine = Engine::new().unwrap();
    let mut runtime = load(
        &engine,
        "export const types = () => Object.fromEntries(
           ['Deno', 'WebAssembly', 'Intl', 'Temporal', 'performance', 'WeakRef',
            'FinalizationRegistry', 'SharedArrayBuffer', 'TextEncoder', 'TextDecoder', 'URL']
             .map((name) => [name, typeof globalThis[name]]));",
    );
    let types = call(&mut runtime, &engine, "types", Value::Null).unwrap();
    for name in [
        "Deno",
        "WebAssembly",
        "Intl",
        "Temporal",
        "performance",
        "WeakRef",
    ] {
        assert_eq!(types[name], "undefined", "{name}");
    }
    assert_eq!(types["TextEncoder"], "function");
    assert_eq!(types["URL"], "function");
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

#[test]
fn dates_are_utc() {
    let engine = Engine::new().unwrap();
    let code = "export const dates = () => [
        new Date(0).getHours(),
        new Date(0).getTimezoneOffset(),
        new Date(2020, 0, 1).getTime(),
        Date.parse('2020-01-01T00:00:00'),
        new Date('2020-01-01T00:00:00').getTime(),
        new Date(2020, 0, 1, 5).toString(),
        new Date({ toString: () => '2020-01-01T00:00:00' }).getTime(),
        Number.isNaN(Date.parse('2020/01/01')),
        typeof structuredClone,
    ];";
    let mut runtime = load(&engine, code);
    let value = call(&mut runtime, &engine, "dates", Value::Null).unwrap();
    let utc = 1_577_836_800_000_i64;
    assert_eq!(
        value,
        json!([
            0,
            0,
            utc,
            utc,
            utc,
            "Wed Jan 01 2020 05:00:00 GMT+0000 (UTC)",
            utc,
            true,
            "undefined"
        ])
    );
}
