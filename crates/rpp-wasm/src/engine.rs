//! Shared wasmtime engine and compiled component handles.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread::JoinHandle;
use std::time::Duration;

use sha2::{Digest, Sha256};
use wasmtime::component::types::{ComponentItem, Type};
use wasmtime::component::{Component, Linker, Val};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::p2;

use crate::instance::map_timeout;
use crate::store::StoreData;
use crate::types::{Function, Limits, Permissions, Schema, ValueType};
use crate::{Error, Result, WasmInstance};

const EPOCH_TICK: Duration = Duration::from_millis(50);

struct EpochTicker {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl EpochTicker {
    fn spawn(engine: Engine) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let handle = std::thread::Builder::new()
            .name("rpp-wasm-epoch".into())
            .spawn(move || {
                while !stop_thread.load(Ordering::Relaxed) {
                    std::thread::sleep(EPOCH_TICK);
                    engine.increment_epoch();
                }
            })?;
        Ok(Self {
            stop,
            handle: Some(handle),
        })
    }
}

impl Drop for EpochTicker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Limits {
    pub(crate) fn epoch_ticks(&self) -> u64 {
        let nanos = self.deadline.as_nanos().max(1);
        nanos.div_ceil(EPOCH_TICK.as_nanos().max(1)).max(1) as u64
    }
}

/// Shared engine used to compile component libraries.
#[derive(Clone)]
pub struct WasmEngine {
    engine: Engine,
    limits: Limits,
    _ticker: Arc<EpochTicker>,
    components: Arc<Mutex<HashMap<[u8; 32], Component>>>,
}

impl WasmEngine {
    /// Construct an engine with default limits.
    pub fn new() -> Result<Self> {
        Self::build(Limits::default(), None)
    }

    /// Construct an engine with custom limits.
    pub fn with_limits(limits: Limits) -> Result<Self> {
        Self::build(limits, None)
    }

    /// Construct an engine using a persistent Wasmtime compilation cache.
    pub fn with_cache_dir(cache_dir: impl AsRef<Path>) -> Result<Self> {
        Self::build(Limits::default(), Some(cache_dir.as_ref()))
    }

    /// Construct an engine with custom limits and a persistent compilation cache.
    pub fn with_limits_and_cache(limits: Limits, cache_dir: impl AsRef<Path>) -> Result<Self> {
        Self::build(limits, Some(cache_dir.as_ref()))
    }

    fn build(limits: Limits, cache_dir: Option<&Path>) -> Result<Self> {
        let mut config = Config::new();
        config.wasm_component_model(true);
        config.epoch_interruption(true);
        if let Some(cache_dir) = cache_dir {
            let cache_dir = if cache_dir.is_absolute() {
                cache_dir.to_path_buf()
            } else {
                std::env::current_dir()
                    .map_err(|source| Error::Io {
                        path: PathBuf::from("."),
                        source,
                    })?
                    .join(cache_dir)
            };
            let mut cache_config = wasmtime::CacheConfig::new();
            cache_config.with_directory(cache_dir);
            let cache = wasmtime::Cache::new(cache_config).map_err(Error::Engine)?;
            config.cache(Some(cache));
        }
        let engine = Engine::new(&config).map_err(Error::Engine)?;
        let ticker = EpochTicker::spawn(engine.clone()).map_err(Error::EpochTicker)?;
        Ok(Self {
            engine,
            limits,
            _ticker: Arc::new(ticker),
            components: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Compile a component and inspect its imports and exported functions.
    pub fn load(&self, wasm_path: &Path) -> Result<CompiledComponent> {
        let bytes = std::fs::read(wasm_path).map_err(|source| Error::Io {
            path: wasm_path.to_path_buf(),
            source,
        })?;
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        let component = {
            let mut components = self
                .components
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(component) = components.get(&digest) {
                component.clone()
            } else {
                let component =
                    Component::new(&self.engine, &bytes).map_err(|source| Error::Compile {
                        path: wasm_path.to_path_buf(),
                        source,
                    })?;
                components.insert(digest, component.clone());
                component
            }
        };
        let schema = schema(&self.engine, &component);
        Ok(CompiledComponent {
            engine: self.clone(),
            component,
            schema,
            path: wasm_path.to_path_buf(),
            digest,
        })
    }

    pub(crate) fn new_store(&self, permissions: Permissions) -> Result<Store<StoreData>> {
        let data = StoreData::new(permissions, self.limits.memory_bytes)?;
        let mut store = Store::new(&self.engine, data);
        store.limiter(|data| &mut data.limits);
        store.epoch_deadline_trap();
        store.set_epoch_deadline(self.limits.epoch_ticks());
        Ok(store)
    }
}

/// A compiled, reusable component library.
#[derive(Clone)]
pub struct CompiledComponent {
    engine: WasmEngine,
    component: Component,
    schema: Schema,
    path: PathBuf,
    digest: [u8; 32],
}

impl CompiledComponent {
    /// Component binary path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// SHA-256 digest of the component binary used for cache identity.
    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }

    /// Discovered imports and exported function signatures.
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// Instantiate with an explicit capability set.
    pub fn instantiate(&self, permissions: Permissions) -> Result<WasmInstance> {
        validate_imports(&self.schema.imports, &permissions)?;
        let mut linker = Linker::new(&self.engine.engine);
        p2::add_to_linker_sync(&mut linker).map_err(Error::Engine)?;
        add_process_host(&mut linker)?;

        let mut store = self.engine.new_store(permissions)?;
        let instance = linker
            .instantiate(&mut store, &self.component)
            .map_err(|error| map_timeout(error, self.engine.limits.deadline))?;
        Ok(WasmInstance {
            store,
            instance,
            epoch_ticks: self.engine.limits.epoch_ticks(),
            deadline: self.engine.limits.deadline,
        })
    }
}

fn schema(engine: &Engine, component: &Component) -> Schema {
    let ty = component.component_type();
    let imports = ty
        .imports(engine)
        .map(|(name, _)| name.to_string())
        .collect();
    let mut functions = Vec::new();
    for (name, item) in ty.exports(engine) {
        collect_functions(engine, name, item, &mut functions);
    }
    functions.sort_by(|a, b| a.path.cmp(&b.path));
    Schema { imports, functions }
}

fn collect_functions(engine: &Engine, path: &str, item: ComponentItem, output: &mut Vec<Function>) {
    match item {
        ComponentItem::ComponentFunc(function) => output.push(Function {
            path: path.to_string(),
            params: function
                .params()
                .map(|(name, ty)| (name.to_string(), value_type(ty)))
                .collect(),
            results: function.results().map(value_type).collect(),
        }),
        ComponentItem::ComponentInstance(instance) => {
            for (name, item) in instance.exports(engine) {
                collect_functions(engine, &format!("{path}#{name}"), item, output);
            }
        }
        _ => {}
    }
}

fn value_type(ty: Type) -> ValueType {
    match ty {
        Type::Bool => ValueType::Bool,
        Type::S8 => ValueType::S8,
        Type::U8 => ValueType::U8,
        Type::S16 => ValueType::S16,
        Type::U16 => ValueType::U16,
        Type::S32 => ValueType::S32,
        Type::U32 => ValueType::U32,
        Type::S64 => ValueType::S64,
        Type::U64 => ValueType::U64,
        Type::Float32 => ValueType::Float32,
        Type::Float64 => ValueType::Float64,
        Type::Char => ValueType::Char,
        Type::String => ValueType::String,
        Type::List(list) => ValueType::List(Box::new(value_type(list.ty()))),
        Type::Record(record) => ValueType::Record(
            record
                .fields()
                .map(|field| (field.name.to_string(), value_type(field.ty)))
                .collect(),
        ),
        Type::Tuple(tuple) => ValueType::Tuple(tuple.types().map(value_type).collect()),
        Type::Variant(variant) => ValueType::Variant(
            variant
                .cases()
                .map(|case| (case.name.to_string(), case.ty.map(value_type)))
                .collect(),
        ),
        Type::Enum(enum_) => ValueType::Enum(enum_.names().map(str::to_string).collect()),
        Type::Option(option) => ValueType::Option(Box::new(value_type(option.ty()))),
        Type::Result(result) => ValueType::Result {
            ok: result.ok().map(value_type).map(Box::new),
            err: result.err().map(value_type).map(Box::new),
        },
        Type::Flags(flags) => ValueType::Flags(flags.names().map(str::to_string).collect()),
        Type::Own(_) | Type::Borrow(_) => ValueType::Unsupported("resource".into()),
        Type::Future(_) => ValueType::Unsupported("future".into()),
        Type::Stream(_) => ValueType::Unsupported("stream".into()),
        Type::ErrorContext => ValueType::Unsupported("error-context".into()),
    }
}

fn validate_imports(imports: &[String], permissions: &Permissions) -> Result<()> {
    for import in imports {
        let allowed = if import.starts_with("wasi:clocks/") {
            permissions.clocks
        } else if import.starts_with("wasi:random/") {
            true
        } else if import.starts_with("wasi:sockets/") {
            permissions.network
        } else if import.starts_with("wasi:filesystem/") {
            !permissions.preopens.is_empty()
        } else if import.starts_with("wasi:cli/environment")
            || import.starts_with("wasi:cli/exit")
            || import.starts_with("wasi:cli/std")
            || import.starts_with("wasi:cli/terminal")
            || import.starts_with("wasi:io/")
        {
            true
        } else if import.starts_with("rpp:host/process") {
            permissions.arbitrary_processes || !permissions.processes.is_empty()
        } else {
            false
        };
        if !allowed {
            return Err(Error::DeniedCapability(import.clone()));
        }
    }
    Ok(())
}

fn add_process_host(linker: &mut Linker<StoreData>) -> Result<()> {
    let mut root = linker.root();
    let mut process = root
        .instance("rpp:host/process@0.1.0")
        .map_err(Error::Engine)?;
    process
        .func_new("run", |store, _ty, params, results| {
            let request = parse_process_request(
                params
                    .first()
                    .ok_or_else(|| wasmtime::Error::msg("missing process request"))?,
            )?;
            let result = crate::process::run(&store.data().permissions, request);
            results[0] = match result {
                Ok(output) => Val::Result(Ok(Some(Box::new(Val::Record(vec![
                    ("status".into(), Val::S32(output.status)),
                    (
                        "stdout".into(),
                        Val::List(output.stdout.into_iter().map(Val::U8).collect()),
                    ),
                    (
                        "stderr".into(),
                        Val::List(output.stderr.into_iter().map(Val::U8).collect()),
                    ),
                ]))))),
                Err(error) => Val::Result(Err(Some(Box::new(Val::String(error))))),
            };
            Ok(())
        })
        .map_err(Error::Engine)
}

fn parse_process_request(value: &Val) -> wasmtime::Result<crate::ProcessRequest> {
    let Val::Record(fields) = value else {
        return Err(wasmtime::Error::msg("process request must be a record"));
    };
    let get = |name: &str| {
        fields
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value)
            .ok_or_else(|| wasmtime::Error::msg(format!("missing process field `{name}`")))
    };
    let Val::String(program) = get("program")? else {
        return Err(wasmtime::Error::msg("process program must be a string"));
    };
    let Val::List(args) = get("args")? else {
        return Err(wasmtime::Error::msg("process args must be a list"));
    };
    let args = args
        .iter()
        .map(|value| match value {
            Val::String(value) => Ok(value.clone()),
            _ => Err(wasmtime::Error::msg("process argument must be a string")),
        })
        .collect::<wasmtime::Result<Vec<_>>>()?;
    let cwd = match get("cwd")? {
        Val::Option(Some(value)) => match value.as_ref() {
            Val::String(value) => Some(PathBuf::from(value)),
            _ => return Err(wasmtime::Error::msg("process cwd must be a string")),
        },
        Val::Option(None) => None,
        _ => return Err(wasmtime::Error::msg("process cwd must be an option")),
    };
    let Val::List(environment) = get("environment")? else {
        return Err(wasmtime::Error::msg("process environment must be a list"));
    };
    let environment = environment
        .iter()
        .map(|value| match value {
            Val::Tuple(values) if values.len() == 2 => match (&values[0], &values[1]) {
                (Val::String(name), Val::String(value)) => Ok((name.clone(), value.clone())),
                _ => Err(wasmtime::Error::msg(
                    "process environment entries must contain strings",
                )),
            },
            _ => Err(wasmtime::Error::msg(
                "process environment entries must be tuples",
            )),
        })
        .collect::<wasmtime::Result<Vec<_>>>()?;
    let Val::List(stdin) = get("stdin")? else {
        return Err(wasmtime::Error::msg("process stdin must be a byte list"));
    };
    let stdin = stdin
        .iter()
        .map(|value| match value {
            Val::U8(value) => Ok(*value),
            _ => Err(wasmtime::Error::msg("process stdin must be a byte list")),
        })
        .collect::<wasmtime::Result<Vec<_>>>()?;
    let timeout = match get("timeout-ms")? {
        Val::Option(Some(value)) => match value.as_ref() {
            Val::U64(value) => Some(Duration::from_millis(*value)),
            _ => return Err(wasmtime::Error::msg("timeout-ms must be u64")),
        },
        Val::Option(None) => None,
        _ => return Err(wasmtime::Error::msg("timeout-ms must be an option")),
    };
    Ok(crate::ProcessRequest {
        program: program.clone(),
        args,
        cwd,
        environment,
        stdin,
        timeout,
    })
}
