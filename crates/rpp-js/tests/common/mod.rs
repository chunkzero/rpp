#![allow(dead_code)]

use std::fs;
use std::path::Path;

use rpp_js::{
    Bundle, BundleRequest, Call, Cancellation, Clock, Engine, Error, Host, HostReply, Limits,
    Runtime,
};
use serde_json::Value;

pub struct NoHost;

impl Host for NoHost {
    fn call(&mut self, name: &str, _: Value, _: Option<Vec<u8>>) -> Result<HostReply, String> {
        Err(format!("unexpected host call {name}"))
    }
}

pub fn plain_bundle(code: &str) -> Bundle {
    Bundle {
        code: code.to_string(),
        source_map: r#"{"version":3,"sources":[],"names":[],"mappings":""}"#.to_string(),
        inputs: Vec::new(),
        input_hashes: Vec::new(),
    }
}

pub fn load(engine: &Engine, code: &str) -> Runtime {
    try_load(engine, code, Limits::default()).unwrap()
}

pub fn try_load(engine: &Engine, code: &str, limits: Limits) -> Result<Runtime, Error> {
    let cancellation = Cancellation::new();
    engine
        .load(
            "plugin.js",
            &plain_bundle(code),
            limits,
            Clock::default(),
            &cancellation,
        )
        .map(|(runtime, _)| runtime)
}

pub fn call_with(
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

pub fn call(
    runtime: &mut Runtime,
    engine: &Engine,
    export: &str,
    args: Value,
) -> Result<Value, Error> {
    call_with(runtime, engine, export, args, None, &mut NoHost).map(|output| output.value)
}

pub fn write(root: &Path, name: &str, contents: &str) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

pub fn request(root: &Path, entry: &str) -> BundleRequest {
    BundleRequest {
        root: root.to_path_buf(),
        entry: entry.to_string(),
        ..Default::default()
    }
}

pub fn bundle_error(request: &BundleRequest) -> String {
    match rpp_js::bundle(request) {
        Err(Error::Bundle(message)) => message,
        other => panic!("expected a bundle error, got {other:?}"),
    }
}

pub fn source_list(source_map: &str) -> Vec<String> {
    let json: serde_json::Value = serde_json::from_str(source_map).unwrap();
    json["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect()
}
