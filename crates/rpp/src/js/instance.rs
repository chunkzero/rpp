//! [`JsPluginInstance`]: a live, per-worker TypeScript plugin.

use std::cell::RefCell;
use std::rc::Rc;

use rpp_js::{Call, Cancellation, Clock, Engine, LogLevel, Output, Runtime};
use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::host::log::{self, LogLevel as HostLevel};
use crate::host::{Phase, RuntimeAccess};
use crate::js::factory::JsPluginFactory;
use crate::js::host::JsHost;
use crate::model::{
    BuildStats, GeneratorHost, PackFile, PluginFactory, PluginInstance, ProcessOutcome,
};
use crate::util::hash::HashWriter;

thread_local! {
    static ENGINE: RefCell<Option<Rc<Engine>>> = const { RefCell::new(None) };
}

/// The current thread's engine, created on first use.
pub(super) fn engine() -> std::result::Result<Rc<Engine>, String> {
    ENGINE.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(engine) = slot.as_ref() {
            return Ok(Rc::clone(engine));
        }
        let engine = Rc::new(Engine::new().map_err(|e| e.to_string())?);
        *slot = Some(Rc::clone(&engine));
        Ok(engine)
    })
}

/// Why a runtime operation failed.
enum Failure {
    Load(String),
    Call(String),
}

fn render(error: &rpp_js::Error) -> String {
    error.to_string().trim_end().to_string()
}

pub(super) struct JsPluginInstance {
    factory: JsPluginFactory,
    engine: Rc<Engine>,
    access: RuntimeAccess,
    cancellation: Cancellation,
    /// The runtime for processors; module state persists between files.
    processor_runtime: Option<Runtime>,
}

impl JsPluginInstance {
    pub(super) fn new(factory: JsPluginFactory) -> Result<Self> {
        let id = factory.shared().id.clone();
        let engine = engine().map_err(|message| Error::PluginLoad {
            plugin: id.clone(),
            message,
        })?;
        let access = factory.access();
        let mut instance = Self {
            factory,
            engine,
            access,
            cancellation: Cancellation::new(),
            processor_runtime: None,
        };
        if instance.factory.processors().is_empty() {
            return Ok(instance);
        }
        match instance.load_runtime(instance.clock(&[&id])) {
            Ok(runtime) => instance.processor_runtime = Some(runtime),
            Err(Failure::Load(message) | Failure::Call(message)) => {
                return Err(Error::PluginLoad {
                    plugin: id,
                    message,
                })
            }
        }
        Ok(instance)
    }

    fn id(&self) -> &str {
        &self.factory.shared().id
    }

    fn clock(&self, parts: &[&str]) -> Clock {
        if !self.access.is_deterministic() {
            return Clock::Real;
        }
        let mut writer = HashWriter::new();
        for part in parts {
            writer.write_str(part);
        }
        Clock::Fixed {
            timestamp_ms: 0,
            seed: writer.finish(),
        }
    }

    fn emit_logs(&self, output: &Output) {
        for entry in &output.logs {
            let level = match entry.level {
                LogLevel::Debug => HostLevel::Debug,
                LogLevel::Info => HostLevel::Info,
                LogLevel::Warn => HostLevel::Warn,
                LogLevel::Error => HostLevel::Error,
            };
            log::emit(self.id(), level, &entry.message);
        }
    }

    /// Evaluate the bundle in a new runtime and call `init`.
    fn load_runtime(&self, clock: Clock) -> std::result::Result<Runtime, Failure> {
        let shared = self.factory.shared();
        let (mut runtime, logs) = self
            .engine
            .load(
                &shared.id,
                &shared.bundle,
                shared.limits,
                clock,
                &self.cancellation,
            )
            .map_err(|e| Failure::Load(render(&e)))?;
        let output = Output {
            value: Value::Null,
            bytes: None,
            logs,
        };
        self.emit_logs(&output);
        let mut host = JsHost::new(&self.access, shared.limits.time, None);
        self.call(
            &mut runtime,
            "init",
            shared.init.clone(),
            None,
            clock,
            &mut host,
        )
        .map_err(|failure| match failure {
            Failure::Call(message) => Failure::Load(message),
            load => load,
        })?;
        Ok(runtime)
    }

    fn call(
        &self,
        runtime: &mut Runtime,
        export: &str,
        args: Value,
        bytes: Option<Vec<u8>>,
        clock: Clock,
        host: &mut JsHost<'_>,
    ) -> std::result::Result<Output, Failure> {
        let call = Call {
            export,
            args,
            bytes,
            clock,
        };
        let output = runtime
            .call(&self.engine, call, host, &self.cancellation)
            .map_err(|e| Failure::Call(render(&e)))?;
        self.emit_logs(&output);
        Ok(output)
    }

    /// Run `export` in a fresh runtime.
    fn run_fresh<'a>(
        &'a self,
        phase: Phase,
        export: &str,
        args: Value,
        generator: Option<&'a mut dyn GeneratorHost>,
    ) -> std::result::Result<(), Failure> {
        let clock = self.clock(&[self.id()]);
        let mut runtime = self.load_runtime(clock)?;
        let mut host = JsHost::new(&self.access, self.factory.shared().limits.time, generator);
        self.access.phase.set(phase);
        let result = self.call(&mut runtime, export, args, None, clock, &mut host);
        self.access.phase.set(Phase::Load);
        result.map(|_| ())
    }

    fn load_error(&self, message: String) -> Error {
        Error::PluginLoad {
            plugin: self.id().to_string(),
            message,
        }
    }
}

impl PluginInstance for JsPluginInstance {
    fn process(&mut self, processor: &str, file: &mut PackFile) -> Result<ProcessOutcome> {
        let (plugin, path) = (self.id().to_string(), file.path.clone());
        let processor_error = |message: String| Error::Processor {
            plugin: plugin.clone(),
            processor: processor.to_string(),
            file: path.clone(),
            message,
        };
        let clock = self.clock(&[self.id(), processor, &file.path]);
        let mut runtime = match self.processor_runtime.take() {
            Some(runtime) => runtime,
            None => self.load_runtime(clock).map_err(|failure| match failure {
                Failure::Load(message) | Failure::Call(message) => self.load_error(message),
            })?,
        };

        let original = std::mem::take(&mut file.contents);
        let mut host = JsHost::new(&self.access, self.factory.shared().limits.time, None);
        self.access.phase.set(Phase::Processor);
        let result = self.call(
            &mut runtime,
            "process",
            json!({ "processor": processor, "path": path }),
            Some(original.clone()),
            clock,
            &mut host,
        );
        self.access.phase.set(Phase::Load);
        if !runtime.is_terminated() {
            self.processor_runtime = Some(runtime);
        }
        let output = match result {
            Ok(output) => output,
            Err(Failure::Load(message) | Failure::Call(message)) => {
                file.contents = original;
                return Err(processor_error(message));
            }
        };
        let (Some(contents), Some((new_path, dropped))) = (output.bytes, host.file.take()) else {
            file.contents = original;
            return Err(processor_error("processor did not return the file".into()));
        };

        let modified = new_path != path || contents != original;
        file.path = new_path;
        file.contents = contents;
        Ok(if dropped {
            ProcessOutcome::Dropped
        } else if modified {
            ProcessOutcome::Modified
        } else {
            ProcessOutcome::Unchanged
        })
    }

    fn generate(&mut self, host: &mut dyn GeneratorHost) -> Result<()> {
        if !self.factory.shared().handlers.generator {
            return Ok(());
        }
        self.run_fresh(Phase::Generator, "generate", Value::Null, Some(host))
            .map_err(|failure| match failure {
                Failure::Load(message) => self.load_error(message),
                Failure::Call(message) => Error::Generator {
                    plugin: self.id().to_string(),
                    message,
                },
            })
    }

    fn on_build_start(&mut self) -> Result<()> {
        if !self.factory.shared().handlers.on_start {
            return Ok(());
        }
        self.hook("on_start", "onStart", Value::Null)
    }

    fn on_build_finish(&mut self, stats: &BuildStats) -> Result<()> {
        if !self.factory.shared().handlers.on_finish {
            return Ok(());
        }
        let args = json!({
            "processed": stats.processed,
            "cached": stats.cached,
            "generated": stats.generated,
            "dropped": stats.dropped,
        });
        self.hook("on_finish", "onFinish", args)
    }
}

impl JsPluginInstance {
    fn hook(&self, hook: &str, export: &str, args: Value) -> Result<()> {
        self.run_fresh(Phase::Hook, export, args, None)
            .map_err(|failure| match failure {
                Failure::Load(message) => self.load_error(message),
                Failure::Call(message) => Error::Hook {
                    plugin: self.id().to_string(),
                    hook: hook.to_string(),
                    message,
                },
            })
    }
}
