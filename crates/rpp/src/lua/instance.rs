//! [`LuaPluginInstance`]: a live, per-worker Lua plugin (spec §4).

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use mlua::{Function, Lua, Table, Value};
use parking_lot::Mutex;

use crate::error::{Error, Result};
use crate::lua::ctx::{base_ctx, PackInfo};
use crate::lua::factory::LuaPluginFactory;
use crate::lua::file::{FileHandle, FileState};
use crate::lua::plugin_builder::PluginBuilder;
use crate::lua::sandbox::{install_limits, run_limited, Deadline, Sandbox};
use crate::lua::traceback;
use crate::model::{BuildStats, GeneratorHost, PackFile, PluginFactory, ProcessOutcome};
use crate::util::path::validate_relative;

/// A live Lua plugin instance bound to a single thread.
///
/// Holds its own [`Lua`] state with a sandboxed environment. The registered
/// processor/generator/hook handlers are retained for the lifetime of the
/// instance.
pub struct LuaPluginInstance {
    lua: Lua,
    plugin_id: String,
    pack: PackInfo,
    options: toml::Value,
    processors: HashMap<String, Function>,
    generator: Option<Function>,
    on_start: Option<Function>,
    on_finish: Option<Function>,
    deadline: Deadline,
    // Kept alive so the sandbox environment (and its closures) live as long as
    // the registered functions.
    _sandbox_env: Table,
}

impl LuaPluginInstance {
    /// Build a fresh instance from a factory.
    pub(crate) fn new(factory: LuaPluginFactory) -> Result<Self> {
        let lua = Lua::new();
        let deadline =
            install_limits(&lua, factory.memory_limit()).map_err(|e| Error::PluginLoad {
                plugin: factory.id().to_string(),
                message: traceback::render(&e),
            })?;

        let plugin_id = factory.id().to_string();
        let (entry_name, entry_source) = factory.entry();

        let sandbox =
            Sandbox::new(&lua, &plugin_id, factory.root()).map_err(|e| Error::PluginLoad {
                plugin: plugin_id.clone(),
                message: traceback::render(&e),
            })?;

        let value = run_limited(&deadline, || {
            sandbox.exec(&lua, &format!("@{entry_name}"), entry_source)
        })
        .map_err(|e| Error::PluginLoad {
            plugin: plugin_id.clone(),
            message: traceback::render(&e),
        })?;

        let builder = extract_builder(&plugin_id, value)?;
        let inner = builder.inner.lock();

        let mut processors = HashMap::new();
        for p in &inner.processors {
            processors.insert(p.def.name.clone(), p.handler.clone());
        }
        let generator = inner.generator.as_ref().map(|g| g.handler.clone());
        let on_start = inner.on_start.clone();
        let on_finish = inner.on_finish.clone();
        drop(inner);

        Ok(LuaPluginInstance {
            lua,
            plugin_id,
            pack: factory.pack().clone(),
            options: factory.options().clone(),
            processors,
            generator,
            on_start,
            on_finish,
            deadline,
            _sandbox_env: sandbox.env,
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

        let call: mlua::Result<()> = run_limited(&self.deadline, || handler.call((ctx, handle)));
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
        let Some(handler) = self.generator.clone() else {
            return Ok(());
        };

        let ctx = self.processor_ctx()?;

        // Bridge GeneratorHost into Lua via a scope so the borrow is bounded by
        // the call. The host RefCell lives outside the scope so the scoped
        // functions can borrow it for the scope's lifetime.
        let plugin_id = self.plugin_id.clone();
        let host = RefCell::new(host);
        let result = run_limited(&self.deadline, || {
            self.lua.scope(|scope| {
                let host_files = &host;
                ctx.set(
                    "files",
                    scope.create_function_mut(
                        move |lua, (_this, glob): (Value, Option<mlua::String>)| {
                            let glob = match glob {
                                Some(s) => Some(s.to_str()?.to_string()),
                                None => None,
                            };
                            let files = host_files.borrow_mut().list_files(glob.as_deref());
                            let t = lua.create_table()?;
                            for (i, f) in files.into_iter().enumerate() {
                                t.raw_set(i + 1, f)?;
                            }
                            Ok(t)
                        },
                    )?,
                )?;

                let host_read = &host;
                ctx.set(
                    "read",
                    scope.create_function_mut(
                        move |lua, (_this, path): (Value, mlua::String)| {
                            let path = path.to_str()?.to_string();
                            validate_relative(&path).map_err(mlua::Error::external)?;
                            match host_read.borrow_mut().read_file(&path) {
                                Some(bytes) => Ok(Value::String(lua.create_string(&bytes)?)),
                                None => Ok(Value::Nil),
                            }
                        },
                    )?,
                )?;

                let host_read_src = &host;
                ctx.set(
                    "read_source",
                    scope.create_function_mut(
                        move |lua, (_this, path): (Value, mlua::String)| {
                            let path = path.to_str()?.to_string();
                            validate_relative(&path).map_err(mlua::Error::external)?;
                            match host_read_src.borrow_mut().read_source(&path) {
                                Some(bytes) => Ok(Value::String(lua.create_string(&bytes)?)),
                                None => Ok(Value::Nil),
                            }
                        },
                    )?,
                )?;

                let host_emit = &host;
                ctx.set(
                    "emit",
                    scope.create_function_mut(
                        move |_, (_this, path, contents): (Value, mlua::String, mlua::String)| {
                            let path = path.to_str()?;
                            validate_relative(&path).map_err(mlua::Error::external)?;
                            host_emit
                                .borrow_mut()
                                .emit(&path, contents.as_bytes().to_vec());
                            Ok(())
                        },
                    )?,
                )?;

                let host_remove = &host;
                ctx.set(
                    "remove",
                    scope.create_function_mut(move |_, (_this, path): (Value, mlua::String)| {
                        let path = path.to_str()?;
                        validate_relative(&path).map_err(mlua::Error::external)?;
                        host_remove.borrow_mut().remove(&path);
                        Ok(())
                    })?,
                )?;

                handler.call::<()>(ctx)
            })
        });

        result.map_err(|e| Error::Generator {
            plugin: plugin_id,
            message: traceback::render(&e),
        })
    }

    fn on_build_start(&mut self) -> Result<()> {
        let Some(handler) = self.on_start.clone() else {
            return Ok(());
        };
        let ctx = self.processor_ctx()?;
        run_limited(&self.deadline, || handler.call::<()>(ctx)).map_err(|e| Error::Hook {
            plugin: self.plugin_id.clone(),
            hook: "on_start".into(),
            message: traceback::render(&e),
        })
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

        run_limited(&self.deadline, || handler.call::<()>((ctx, stats_t))).map_err(|e| {
            Error::Hook {
                plugin: self.plugin_id.clone(),
                hook: "on_finish".into(),
                message: traceback::render(&e),
            }
        })
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
