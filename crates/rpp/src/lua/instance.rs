//! [`LuaPluginInstance`]: a live, per-worker Lua plugin (spec §4).

use std::collections::HashMap;
use std::sync::Arc;

use mlua::{Function, Lua, Table, Value};
use parking_lot::Mutex;

use crate::error::{Error, Result};
use crate::lua::bootstrap::eval_entry;
use crate::lua::ctx::{base_ctx, PackInfo};
use crate::lua::factory::LuaPluginFactory;
use crate::lua::file::{FileHandle, FileState};
use crate::lua::generator_ctx::call_generator;
use crate::lua::plugin_builder::PluginBuilder;
use crate::lua::runtime::{Phase, RuntimeAccess};
use crate::lua::sandbox::{run_limited, Deadline};
use crate::lua::traceback;
use crate::model::{BuildStats, GeneratorHost, PackFile, PluginFactory, ProcessOutcome};

/// A live Lua plugin instance bound to a single thread.
///
/// Holds its own [`Lua`] state with a sandboxed environment. The registered
/// processor/generator/hook handlers are retained for the lifetime of the
/// instance.
pub(crate) struct LuaPluginInstance {
    lua: Lua,
    plugin_id: String,
    pack: PackInfo,
    options: toml::Value,
    processors: HashMap<String, Function>,
    generator: Option<(String, Function)>,
    on_start: Option<Function>,
    on_finish: Option<Function>,
    deadline: Deadline,
    execution_limit: std::time::Duration,
    access: RuntimeAccess,
    // Kept alive so the sandbox environment (and its closures) live as long as
    // the registered functions.
    _sandbox_env: Table,
}

impl LuaPluginInstance {
    /// Build a fresh instance from a factory.
    pub(crate) fn new(factory: LuaPluginFactory) -> Result<Self> {
        let plugin_id = factory.id().to_string();
        let (entry_name, entry_source) = factory.entry();
        let eval = eval_entry(
            &plugin_id,
            factory.root(),
            entry_name,
            entry_source,
            factory.memory_limit(),
            factory.execution_limit(),
            factory.access(),
        )?;

        let builder = extract_builder(&plugin_id, eval.value)?;
        let inner = builder.inner.lock();

        let mut processors = HashMap::new();
        for p in &inner.processors {
            processors.insert(p.def.name.clone(), p.handler.clone());
        }
        let generator = inner
            .generator
            .as_ref()
            .map(|g| (g.name.clone(), g.handler.clone()));
        let on_start = inner.on_start.clone();
        let on_finish = inner.on_finish.clone();
        drop(inner);

        Ok(LuaPluginInstance {
            lua: eval.lua,
            plugin_id,
            pack: factory.pack().clone(),
            options: factory.options().clone(),
            processors,
            generator,
            on_start,
            on_finish,
            deadline: eval.deadline,
            execution_limit: factory.execution_limit(),
            access: factory.access(),
            _sandbox_env: eval.sandbox.env,
        })
    }

    fn processor_ctx(&self) -> Result<Table> {
        base_ctx(&self.lua, &self.plugin_id, &self.options, &self.pack).map_err(|e| {
            Error::PluginLoad {
                plugin: self.plugin_id.clone(),
                message: traceback::render(&e),
            }
        })
    }
}

impl crate::model::PluginInstance for LuaPluginInstance {
    fn process(&mut self, processor: &str, file: &mut PackFile) -> Result<ProcessOutcome> {
        let handler = self
            .processors
            .get(processor)
            .ok_or_else(|| Error::Processor {
                plugin: self.plugin_id.clone(),
                processor: processor.to_string(),
                file: file.path.clone(),
                message: "no such processor".into(),
            })?;
        let handler = handler.clone();

        let ctx = self.processor_ctx()?;

        let state = Arc::new(Mutex::new(FileState::new(
            file.path.clone(),
            std::mem::take(&mut file.contents),
        )));
        let handle = FileHandle(state.clone());

        self.access.phase.set(Phase::Processor);
        let call: mlua::Result<()> = run_limited(&self.deadline, self.execution_limit, || {
            handler.call((ctx, handle))
        });
        self.access.phase.set(Phase::Load);
        if let Err(e) = call {
            return Err(Error::Processor {
                plugin: self.plugin_id.clone(),
                processor: processor.to_string(),
                file: file.path.clone(),
                message: traceback::render(&e),
            });
        }

        let final_state = Arc::try_unwrap(state)
            .map(Mutex::into_inner)
            .unwrap_or_else(|arc| arc.lock().clone());

        file.path = final_state.path;
        file.contents = final_state.contents;

        Ok(if final_state.dropped {
            ProcessOutcome::Dropped
        } else if final_state.modified {
            ProcessOutcome::Modified
        } else {
            ProcessOutcome::Unchanged
        })
    }

    fn generate(&mut self, host: &mut dyn GeneratorHost) -> Result<()> {
        let Some((name, handler)) = self.generator.clone() else {
            return Ok(());
        };

        let ctx = self.processor_ctx()?;
        let plugin_id = self.plugin_id.clone();

        self.access.phase.set(Phase::Generator);
        let result = run_limited(&self.deadline, self.execution_limit, || {
            call_generator(&self.lua, &handler, ctx, host, self._sandbox_env.clone())
        })
        .map_err(|e| Error::Generator {
            plugin: plugin_id,
            message: format!("generator `{name}`: {}", traceback::render(&e)),
        });
        self.access.phase.set(Phase::Load);
        result
    }

    fn on_build_start(&mut self) -> Result<()> {
        let Some(handler) = self.on_start.clone() else {
            return Ok(());
        };
        let ctx = self.processor_ctx()?;
        self.access.phase.set(Phase::Hook);
        let result = run_limited(&self.deadline, self.execution_limit, || {
            handler.call::<()>(ctx)
        })
        .map_err(|e| Error::Hook {
            plugin: self.plugin_id.clone(),
            hook: "on_start".into(),
            message: traceback::render(&e),
        });
        self.access.phase.set(Phase::Load);
        result
    }

    fn on_build_finish(&mut self, stats: &BuildStats) -> Result<()> {
        let Some(handler) = self.on_finish.clone() else {
            return Ok(());
        };
        let ctx = self.processor_ctx()?;
        let stats_t = self.lua.create_table().map_err(|e| Error::Hook {
            plugin: self.plugin_id.clone(),
            hook: "on_finish".into(),
            message: traceback::render(&e),
        })?;
        let _ = stats_t.set("processed", stats.processed);
        let _ = stats_t.set("cached", stats.cached);
        let _ = stats_t.set("generated", stats.generated);
        let _ = stats_t.set("dropped", stats.dropped);

        self.access.phase.set(Phase::Hook);
        let result = run_limited(&self.deadline, self.execution_limit, || {
            handler.call::<()>((ctx, stats_t))
        })
        .map_err(|e| Error::Hook {
            plugin: self.plugin_id.clone(),
            hook: "on_finish".into(),
            message: traceback::render(&e),
        });
        self.access.phase.set(Phase::Load);
        result
    }
}

/// Extract a [`PluginBuilder`] from the value returned by `init.lua`.
pub(crate) fn extract_builder(plugin_id: &str, value: Value) -> Result<PluginBuilder> {
    match value {
        Value::UserData(ud) => {
            let builder = ud
                .borrow::<PluginBuilder>()
                .map_err(|_| Error::PluginLoad {
                    plugin: plugin_id.to_string(),
                    message: "entry script must `return rpp.plugin()`".into(),
                })?;
            Ok(builder.clone())
        }
        _ => Err(Error::PluginLoad {
            plugin: plugin_id.to_string(),
            message: "entry script must return a plugin (got non-plugin value)".into(),
        }),
    }
}
