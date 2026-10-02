//! Host calls reachable from plugin JavaScript.

use std::time::{Duration, Instant};

use rpp_js::{Host, HostReply};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

use super::access::RuntimeAccess;
#[cfg(feature = "wasm")]
use super::component::Components;
use super::hash;
use super::process::{self, ProcessRequest};
use super::toml_json::{json_to_toml, toml_to_json};
use crate::model::GeneratorHost;
use crate::util::glob;
use crate::util::path::validate_relative;

/// What the plugin is running, which decides the host calls it may make.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
    /// Bundle evaluation, `describe` and `init`.
    Load,
    Processor,
    Generator,
    Hook,
}

/// The host for one JavaScript call.
pub(super) struct JsHost<'a> {
    access: &'a RuntimeAccess,
    phase: Phase,
    deadline: Instant,
    generator: Option<&'a mut dyn GeneratorHost>,
    /// The final path and drop flag reported by the `file` call.
    pub(super) file: Option<(String, bool)>,
    #[cfg(feature = "wasm")]
    components: Components,
}

#[derive(Deserialize)]
struct FileArgs {
    path: String,
    dropped: bool,
}

#[derive(Deserialize)]
struct GlobArgs {
    glob: Option<String>,
}

#[derive(Deserialize)]
struct PathArgs {
    path: String,
}

#[derive(Deserialize)]
struct OutputArgs {
    root: String,
    path: String,
}

#[derive(Deserialize)]
struct TomlParseArgs {
    text: String,
}

#[derive(Deserialize)]
struct TomlStringifyArgs {
    value: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Algorithm {
    Xxh3,
    Sha256,
    Md5,
    Crc32,
}

#[derive(Deserialize)]
struct HashArgs {
    algorithm: Algorithm,
}

#[derive(Deserialize)]
struct MatchArgs {
    pattern: String,
    path: String,
}

#[derive(Deserialize)]
struct RunArgs {
    program: String,
    #[serde(default)]
    args: Vec<String>,
    cwd: Option<String>,
    #[serde(default)]
    env: std::collections::BTreeMap<String, String>,
    timeout_ms: Option<u64>,
}

fn parse<T: DeserializeOwned>(name: &str, value: Value) -> Result<T, String> {
    serde_json::from_value(value)
        .map_err(|error| format!("invalid arguments for `{name}`: {error}"))
}

fn relative(path: &str) -> Result<(), String> {
    validate_relative(path)
}

fn reply(value: Value) -> HostReply {
    HostReply { value, bytes: None }
}

fn null() -> Result<HostReply, String> {
    Ok(reply(Value::Null))
}

impl<'a> JsHost<'a> {
    pub(super) fn new(
        access: &'a RuntimeAccess,
        phase: Phase,
        deadline: Instant,
        generator: Option<&'a mut dyn GeneratorHost>,
    ) -> Self {
        Self {
            access,
            phase,
            deadline,
            generator,
            file: None,
            #[cfg(feature = "wasm")]
            components: Components::new(),
        }
    }

    fn generator(&mut self, name: &str) -> Result<&mut (dyn GeneratorHost + 'a), String> {
        self.generator
            .as_deref_mut()
            .ok_or_else(|| format!("`{name}` is only available in the generator"))
    }

    fn run_process(&self, args: RunArgs, stdin: Vec<u8>) -> Result<HostReply, String> {
        let access = self.access;
        if !access.allows_process() {
            return Err("process execution requires trusted process permissions".into());
        }
        if !matches!(self.phase, Phase::Generator | Phase::Hook) {
            return Err("process execution is only available in generators and hooks".into());
        }
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("execution time limit exhausted before the process started".into());
        }
        let timeout = args
            .timeout_ms
            .map_or(remaining, |ms| Duration::from_millis(ms).min(remaining));
        let cwd = match &args.cwd {
            Some(cwd) => {
                relative(cwd)?;
                Some(access.project_root.join(cwd))
            }
            None => None,
        };
        let output = process::run(
            access,
            ProcessRequest {
                program: args.program,
                args: args.args,
                cwd,
                environment: args.env.into_iter().collect(),
                stdin,
                timeout: Some(timeout),
            },
        )?;
        Ok(HostReply {
            value: json!({
                "status": output.status,
                "stderr": String::from_utf8_lossy(&output.stderr),
            }),
            bytes: Some(output.stdout),
        })
    }
}

impl JsHost<'_> {
    /// Host calls that read or write through the generator host.
    fn generator_call(
        &mut self,
        name: &str,
        value: Value,
        bytes: Option<Vec<u8>>,
    ) -> Result<HostReply, String> {
        match name {
            "files" | "source_files" => {
                let args: GlobArgs = parse(name, value)?;
                let host = self.generator(name)?;
                let glob = args.glob.as_deref();
                let files = if name == "files" {
                    host.list_files(glob)
                } else {
                    host.list_source_files(glob)
                };
                Ok(reply(json!(files)))
            }
            "read" | "read_source" => {
                let args: PathArgs = parse(name, value)?;
                relative(&args.path)?;
                let host = self.generator(name)?;
                let contents = if name == "read" {
                    host.read_file(&args.path)
                } else {
                    host.read_source(&args.path)
                };
                Ok(HostReply {
                    value: json!({ "found": contents.is_some() }),
                    bytes: contents,
                })
            }
            "emit" | "remove" => {
                let args: PathArgs = parse(name, value)?;
                relative(&args.path)?;
                let host = self.generator(name)?;
                if name == "emit" {
                    host.emit(&args.path, bytes.unwrap_or_default());
                } else {
                    host.remove(&args.path);
                }
                null()
            }
            "emit_output" => {
                let args: OutputArgs = parse(name, value)?;
                relative(&args.path)?;
                self.generator(name)?.emit_output(
                    &args.root,
                    &args.path,
                    bytes.unwrap_or_default(),
                );
                null()
            }
            _ => Err(format!("unknown host call `{name}`")),
        }
    }
}

/// Host calls that only transform their arguments.
fn utility_call(name: &str, value: Value, bytes: Option<Vec<u8>>) -> Result<HostReply, String> {
    match name {
        "toml.parse" => {
            let args: TomlParseArgs = parse(name, value)?;
            let parsed: toml::Value =
                toml::from_str(&args.text).map_err(|e| format!("toml parse error: {e}"))?;
            Ok(reply(toml_to_json(&parsed)))
        }
        "toml.stringify" => {
            let args: TomlStringifyArgs = parse(name, value)?;
            let value =
                json_to_toml(&args.value).map_err(|e| format!("toml stringify error: {e}"))?;
            let text = toml::to_string(&value).map_err(|e| format!("toml stringify error: {e}"))?;
            Ok(reply(Value::String(text)))
        }
        "hash" => {
            let args: HashArgs = parse(name, value)?;
            let data = bytes.unwrap_or_default();
            Ok(reply(match args.algorithm {
                Algorithm::Xxh3 => json!(hash::xxh3_hex(&data)),
                Algorithm::Sha256 => json!(hash::sha256_hex(&data)),
                Algorithm::Md5 => json!(hash::md5_hex(&data)),
                Algorithm::Crc32 => json!(hash::crc32(&data)),
            }))
        }
        "glob.match" => {
            let args: MatchArgs = parse(name, value)?;
            let matched = glob::compile(&args.pattern).is_ok_and(|g| g.matches(&args.path));
            Ok(reply(json!(matched)))
        }
        _ => Err(format!("unknown host call `{name}`")),
    }
}

impl Host for JsHost<'_> {
    fn call(
        &mut self,
        name: &str,
        value: Value,
        bytes: Option<Vec<u8>>,
    ) -> Result<HostReply, String> {
        match name {
            "file" => {
                let args: FileArgs = parse(name, value)?;
                relative(&args.path)?;
                self.file = Some((args.path, args.dropped));
                null()
            }
            "files" | "source_files" | "read" | "read_source" | "emit" | "remove"
            | "emit_output" => self.generator_call(name, value, bytes),
            "toml.parse" | "toml.stringify" | "hash" | "glob.match" => {
                utility_call(name, value, bytes)
            }
            "process.run" => {
                let args: RunArgs = parse(name, value)?;
                self.run_process(args, bytes.unwrap_or_default())
            }
            #[cfg(feature = "wasm")]
            "component.load" => self.components.load(self.access, self.deadline, value),
            #[cfg(feature = "wasm")]
            "component.call" => self
                .components
                .call(self.access, self.deadline, value, bytes),
            #[cfg(not(feature = "wasm"))]
            "component.load" | "component.call" => {
                Err("components require rpp built with the `wasm` feature".into())
            }
            _ => Err(format!("unknown host call `{name}`")),
        }
    }
}
