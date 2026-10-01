//! One `JsRuntime`: setup, module evaluation and export invocation.

use std::time::Duration;

use deno_core::error::{CoreError, CoreErrorKind};
use deno_core::{v8, JsRuntime, ModuleSpecifier, PollEventLoopOptions, RuntimeOptions};
use serde_json::Value;

use crate::deadline::Deadline;
use crate::error::{Error, Result};
use crate::model::{Call, Cancellation, Clock, Host, Limits, Log, Output};
use crate::termination::{Reason, Termination};

mod snapshot_sources {
    include!(concat!(env!("OUT_DIR"), "/snapshot_sources.rs"));
}

const EMERGENCY_HEAP_BYTES: usize = 8 * 1024 * 1024;
const INVALID_RESULT: &str = "RppInvalidResult";

pub(crate) struct State {
    // Persistent handles must drop before their isolate.
    run: Option<v8::Global<v8::Function>>,
    namespace: Option<v8::Global<v8::Object>>,
    pub(crate) runtime: JsRuntime,
    termination: Termination,
    pub(crate) terminated: bool,
    module: String,
    source_map: String,
}

impl State {
    pub(crate) fn new(limits: Limits, name: &str, source_map: &str) -> Self {
        let termination = Termination::default();
        let mut extensions = crate::extensions::web();
        extensions.extend([
            crate::profile::rpp_profile::init(),
            crate::host::rpp_host::init(),
        ]);
        let mut runtime = JsRuntime::new(RuntimeOptions {
            extensions,
            startup_snapshot: Some(include_bytes!(concat!(env!("OUT_DIR"), "/snapshot.bin"))),
            residual_lazy_js_sources: snapshot_sources::JS,
            residual_lazy_esm_sources: snapshot_sources::ESM,
            create_params: Some(
                v8::Isolate::create_params()
                    .heap_limits(0, limits.heap_bytes)
                    .array_buffer_allocator(crate::allocator::bounded(
                        limits.heap_bytes,
                        termination.clone(),
                    )),
            ),
            ..Default::default()
        });
        crate::profile::initialize(&mut runtime);
        crate::host::initialize(&mut runtime.op_state().borrow_mut());
        let heap_signal = termination.clone();
        let handle = runtime.v8_isolate().thread_safe_handle();
        runtime.add_near_heap_limit_callback(move |limit, _| {
            heap_signal.record(Reason::Heap);
            handle.terminate_execution();
            limit + EMERGENCY_HEAP_BYTES
        });
        Self {
            run: None,
            namespace: None,
            runtime,
            termination,
            terminated: false,
            module: module_specifier(name).to_string(),
            source_map: source_map.to_owned(),
        }
    }

    pub(crate) fn initialize_on(
        &mut self,
        executor: &tokio::runtime::Runtime,
        deadline: &Deadline,
        code: &str,
        limits: Limits,
        clock: Clock,
        cancellation: &Cancellation,
    ) -> Result<Vec<Log>> {
        crate::profile::begin(&mut self.runtime, clock)?;
        let result = self.guarded(deadline, limits.time, cancellation, |state| {
            executor.block_on(state.initialize(code))
        });
        let logs = crate::profile::end(&mut self.runtime);
        result.map(|()| logs)
    }

    pub(crate) fn call(
        &mut self,
        executor: &tokio::runtime::Runtime,
        deadline: &Deadline,
        call: Call<'_>,
        host: &mut dyn Host,
        limits: Limits,
        cancellation: &Cancellation,
    ) -> Result<Output> {
        if self.terminated {
            return Err(Error::Terminated);
        }
        let args = serde_json::to_string(&call.args)
            .map_err(|error| Error::Invalid(format!("arguments are not JSON: {error}")))?;
        crate::profile::begin(&mut self.runtime, call.clock)?;
        let state = self.runtime.op_state();
        crate::host::set_pending(&mut state.borrow_mut(), call.bytes);
        let installed = crate::host::Installed::new(state, host);
        let result = self.guarded(deadline, limits.time, cancellation, |state| {
            executor.block_on(state.invoke(call.export, &args))
        });
        drop(installed);
        let logs = crate::profile::end(&mut self.runtime);
        let (value, bytes) = result?;
        Ok(Output { value, bytes, logs })
    }

    fn guarded<T>(
        &mut self,
        deadline: &Deadline,
        time: Duration,
        cancellation: &Cancellation,
        run: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let handle = self.runtime.v8_isolate().thread_safe_handle();
        let guard = deadline.arm(handle, cancellation.clone(), time, self.termination.clone());
        let result = run(self);
        drop(guard);
        if let Some(error) = self.termination.take() {
            self.terminated = true;
            return Err(error);
        }
        result.map_err(|error| match error {
            Error::JavaScript { message, stack } => Error::JavaScript {
                message,
                stack: crate::sourcemap::map_stack(&stack, &self.module, &self.source_map),
            },
            other => other,
        })
    }

    async fn initialize(&mut self, code: &str) -> Result<()> {
        self.runtime
            .execute_script("rpp:web", include_str!("web.js"))
            .map_err(js_error)?;
        let run = self
            .runtime
            .execute_script("rpp:bootstrap", include_str!("bootstrap.js"))
            .map_err(js_error)?;
        {
            deno_core::scope!(scope, &mut self.runtime);
            let run = v8::Local::new(scope, run);
            let run = v8::Local::<v8::Function>::try_from(run).map_err(js_error)?;
            self.run = Some(v8::Global::new(scope, run));
        }
        let specifier = ModuleSpecifier::parse(&self.module).map_err(js_error)?;
        let module = self
            .runtime
            .load_main_es_module_from_code(&specifier, code.to_owned())
            .await
            .map_err(js_error)?;
        let evaluation = self.runtime.mod_evaluate(module);
        self.runtime
            .with_event_loop_promise(evaluation, PollEventLoopOptions::default())
            .await
            .map_err(js_error)?;
        self.namespace = Some(
            self.runtime
                .get_module_namespace(module)
                .map_err(js_error)?,
        );
        Ok(())
    }

    async fn invoke(&mut self, export: &str, args: &str) -> Result<(Value, Option<Vec<u8>>)> {
        let arguments = {
            deno_core::scope!(scope, &mut self.runtime);
            let namespace = v8::Local::new(scope, self.namespace.as_ref().expect("initialized"));
            let missing = || Error::MissingExport(export.to_owned());
            let key = v8::String::new(scope, export).ok_or(Error::Heap)?;
            let function = namespace.get(scope, key.into()).ok_or_else(missing)?;
            if !function.is_function() {
                return Err(missing());
            }
            let args = v8::String::new(scope, args).ok_or(Error::Heap)?;
            [
                v8::Global::new(scope, function),
                v8::Global::new(scope, v8::Local::<v8::Value>::from(args)),
            ]
        };
        let call = self
            .runtime
            .call_with_args(self.run.as_ref().expect("initialized"), &arguments);
        let output = self
            .runtime
            .with_event_loop_promise(call, PollEventLoopOptions::default())
            .await
            .map_err(js_error)?;
        deno_core::scope!(scope, &mut self.runtime);
        let output = v8::Local::new(scope, output);
        if output.is_undefined() {
            return Ok((Value::Null, None));
        }
        if let Ok(array) = v8::Local::<v8::Uint8Array>::try_from(output) {
            let mut bytes = vec![0; array.byte_length()];
            array.copy_contents(&mut bytes);
            return Ok((Value::Null, Some(bytes)));
        }
        let json = v8::Local::<v8::String>::try_from(output).map_err(js_error)?;
        let value = serde_json::from_str(&json.to_rust_string_lossy(scope))
            .map_err(|error| Error::Invalid(format!("result is not JSON: {error}")))?;
        Ok((value, None))
    }
}

/// The module specifier V8 prints in stacks for a module called `name`.
fn module_specifier(name: &str) -> ModuleSpecifier {
    ModuleSpecifier::parse(name)
        .or_else(|_| ModuleSpecifier::parse(&format!("rpp:{name}")))
        .unwrap_or_else(|_| ModuleSpecifier::parse("rpp:plugin").expect("valid specifier"))
}

fn js_error(error: impl Into<CoreError>) -> Error {
    let error = error.into();
    let CoreErrorKind::Js(js) = *error.0 else {
        return Error::JavaScript {
            message: error.to_string(),
            stack: String::new(),
        };
    };
    let message = match (&js.name, &js.message) {
        (Some(name), Some(message)) if name == INVALID_RESULT => {
            return Error::Invalid(message.clone());
        }
        (Some(name), Some(message)) => format!("{name}: {message}"),
        (None, Some(message)) => message.clone(),
        _ => js.exception_message.clone(),
    };
    let stack = js
        .stack
        .iter()
        .flat_map(|stack| stack.lines())
        .filter(|line| line.trim_start().starts_with("at "))
        .collect::<Vec<_>>()
        .join("\n");
    Error::JavaScript { message, stack }
}
