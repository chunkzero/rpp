//! WASM components for JavaScript plugins: the `component.load` and `component.call`
//! host calls. Values cross in the [`wire`] encoding (see `sdk/components.ts`).

mod wire;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use rpp_js::HostReply;
use rpp_wasm::{
    Error as WasmError, Function, Permissions, Preopen, Value as WasmValue, WasmInstance,
};
use serde::Deserialize;
use serde_json::{json, Value};

use self::wire::{from_wire, function_json, to_wire};
use super::access::RuntimeAccess;

static NEXT_JOB: AtomicU64 = AtomicU64::new(0);

struct Slot {
    name: String,
    /// `None` once a call has trapped or timed out.
    instance: Option<WasmInstance>,
}

/// The component instances loaded during one JavaScript job.
pub(super) struct Components {
    job: u64,
    slots: Vec<Slot>,
}

#[derive(Deserialize)]
struct LoadArgs {
    name: String,
}

#[derive(Deserialize)]
struct CallArgs {
    handle: String,
    path: String,
    args: Vec<Value>,
}

impl Components {
    pub(super) fn new() -> Self {
        Self {
            job: NEXT_JOB.fetch_add(1, Ordering::Relaxed),
            slots: Vec::new(),
        }
    }

    pub(super) fn load(
        &mut self,
        access: &RuntimeAccess,
        deadline: Instant,
        value: Value,
    ) -> Result<HostReply, String> {
        let args: LoadArgs = parse("component.load", value)?;
        let component = access
            .components
            .get(&args.name)
            .ok_or_else(|| format!("unknown component `{}`", args.name))?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        let instance = match component.instantiate_with_deadline(permissions(access), remaining) {
            Ok(instance) => instance,
            Err(error @ WasmError::Timeout(_)) => return Ok(failure("timeout", &error)),
            Err(error) => {
                return Err(format!(
                    "component `{}` failed to instantiate: {error}",
                    args.name
                ));
            }
        };
        let functions: Vec<Value> = component
            .schema()
            .functions
            .iter()
            .map(function_json)
            .collect();
        let handle = format!("{}:{}", self.job, self.slots.len());
        self.slots.push(Slot {
            name: args.name,
            instance: Some(instance),
        });
        Ok(HostReply {
            value: json!({ "handle": handle, "functions": functions }),
            bytes: None,
        })
    }

    pub(super) fn call(
        &mut self,
        access: &RuntimeAccess,
        deadline: Instant,
        value: Value,
        bytes: Option<Vec<u8>>,
    ) -> Result<HostReply, String> {
        let args: CallArgs = parse("component.call", value)?;
        let slot = self.slot(&args.handle)?;
        let function = export(access, &slot.name, &args)?;
        let params = params(function, &args.args, &bytes.unwrap_or_default())?;

        let instance = slot
            .instance
            .as_mut()
            .ok_or("component handle poisoned by an earlier trap or timeout")?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        let results = match instance.call_with_deadline(&args.path, &params, remaining) {
            Ok(results) => results,
            Err(error @ WasmError::Timeout(_)) => {
                slot.instance = None;
                return Ok(failure("timeout", &error));
            }
            Err(error @ WasmError::Trap(_)) => {
                slot.instance = None;
                return Ok(failure("trap", &error));
            }
            Err(error) => return Err(error.to_string()),
        };
        results_reply(function, &args.path, results)
    }

    fn slot(&mut self, handle: &str) -> Result<&mut Slot, String> {
        let released = || "component handle released".to_string();
        let (job, index) = handle.split_once(':').ok_or_else(released)?;
        if job.parse::<u64>().ok() != Some(self.job) {
            return Err(released());
        }
        let index: usize = index.parse().map_err(|_| released())?;
        self.slots.get_mut(index).ok_or_else(released)
    }
}

fn parse<T: serde::de::DeserializeOwned>(name: &str, value: Value) -> Result<T, String> {
    serde_json::from_value(value)
        .map_err(|error| format!("invalid arguments for `{name}`: {error}"))
}

/// The reply for a load or call that timed out or trapped.
fn failure(kind: &str, error: &WasmError) -> HostReply {
    HostReply {
        value: json!({ "failure": { "kind": kind, "message": error.to_string() } }),
        bytes: None,
    }
}

/// The export `args.path` of `component`, checked against the number of arguments.
fn export<'a>(
    access: &'a RuntimeAccess,
    component: &str,
    args: &CallArgs,
) -> Result<&'a Function, String> {
    let function = access
        .components
        .get(component)
        .and_then(|component| {
            component
                .schema()
                .functions
                .iter()
                .find(|function| function.path == args.path)
        })
        .ok_or_else(|| format!("component `{component}` has no export `{}`", args.path))?;
    if args.args.len() != function.params.len() {
        return Err(format!(
            "export `{}` expects {} argument(s), got {}",
            args.path,
            function.params.len(),
            args.args.len()
        ));
    }
    Ok(function)
}

fn params(function: &Function, args: &[Value], bytes: &[u8]) -> Result<Vec<WasmValue>, String> {
    args.iter()
        .zip(&function.params)
        .map(|(value, (name, ty))| {
            from_wire(value, ty, bytes)
                .map_err(|message| format!("invalid parameter `{name}`: {message}"))
        })
        .collect()
}

/// The `{ results }` reply, with byte lists attached.
fn results_reply(
    function: &Function,
    path: &str,
    results: Vec<WasmValue>,
) -> Result<HostReply, String> {
    if results.len() != function.results.len() {
        return Err(format!(
            "export `{path}` returned {} value(s), but its schema declares {}",
            results.len(),
            function.results.len()
        ));
    }
    let mut out = Vec::new();
    let results = results
        .into_iter()
        .zip(&function.results)
        .enumerate()
        .map(|(index, (value, ty))| {
            to_wire(value, ty, &mut out)
                .map_err(|message| format!("invalid result {}: {message}", index + 1))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(HostReply {
        value: json!({ "results": results }),
        bytes: Some(out),
    })
}

/// The WASI capabilities the plugin's permissions grant its components.
fn permissions(access: &RuntimeAccess) -> Permissions {
    let granted = &access.permissions;
    let env = granted
        .environment
        .iter()
        .filter_map(|name| std::env::var(name).ok().map(|value| (name.clone(), value)))
        .collect();
    let preopen = |path: &PathBuf, writable| Preopen {
        host: access.project_root.join(path),
        guest: path.to_string_lossy().replace('\\', "/"),
        writable,
    };
    let reads = granted.read.iter().map(|path| preopen(path, false));
    let writes = granted.write.iter().map(|path| preopen(path, true));
    let preopens = reads.chain(writes).collect();
    Permissions {
        clocks: granted.clocks,
        random: granted.random,
        stdio: granted.stdio,
        network: granted.network,
        environment: env,
        preopens,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_handle_from_previous_job_rejected() {
        let access = RuntimeAccess::sandboxed(PathBuf::from("."));
        let previous = Components::new();
        let mut current = Components::new();
        let call = json!({ "handle": format!("{}:0", previous.job), "path": "f", "args": [] });
        let error = current
            .call(&access, Instant::now(), call, None)
            .unwrap_err();
        assert_eq!(error, "component handle released");
    }
}
