//! The `rpp.plugin()` builder userdata and its registration API (spec §4).

use std::sync::Arc;

use mlua::{AnyUserData, Function, Lua, Table, UserData, UserDataMethods, Value};
use parking_lot::Mutex;

use crate::model::ProcessorDef;

/// A registered processor: its definition plus the Lua handler.
pub(crate) struct RegisteredProcessor {
    pub(crate) def: ProcessorDef,
    pub(crate) handler: Function,
}

/// A registered generator: its name plus the Lua handler.
pub(crate) struct RegisteredGenerator {
    #[allow(dead_code)]
    pub(crate) name: String,
    pub(crate) handler: Function,
}

/// The collected registrations from a plugin's `init.lua`.
#[derive(Default)]
pub(crate) struct Registrations {
    pub(crate) processors: Vec<RegisteredProcessor>,
    pub(crate) generator: Option<RegisteredGenerator>,
    pub(crate) on_start: Option<Function>,
    pub(crate) on_finish: Option<Function>,
}

/// The plugin builder exposed to Lua as the value returned by `rpp.plugin()`.
#[derive(Clone)]
pub(crate) struct PluginBuilder {
    pub(crate) inner: Arc<Mutex<Registrations>>,
}

impl PluginBuilder {
    /// Create a builder userdata wrapping fresh registrations.
    pub(crate) fn create_userdata(lua: &Lua) -> mlua::Result<AnyUserData> {
        lua.create_userdata(PluginBuilder {
            inner: Arc::new(Mutex::new(Registrations::default())),
        })
    }
}

impl UserData for PluginBuilder {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // plugin:processor(name, opts, fn)
        methods.add_method(
            "processor",
            |_, this, (name, opts, handler): (mlua::String, Table, Function)| {
                let name = name.to_str()?.to_string();

                let files: Vec<String> = match opts.get::<Value>("files")? {
                    Value::Table(t) => {
                        let mut v = Vec::new();
                        for item in t.sequence_values::<mlua::String>() {
                            v.push(item?.to_str()?.to_string());
                        }
                        v
                    }
                    Value::String(s) => vec![s.to_str()?.to_string()],
                    Value::Nil => {
                        return Err(mlua::Error::external(
                            "processor options require a `files` glob list",
                        ));
                    }
                    other => {
                        return Err(mlua::Error::external(format!(
                            "`files` must be a string or list, got {}",
                            other.type_name()
                        )));
                    }
                };

                if files.is_empty() {
                    return Err(mlua::Error::external(
                        "processor `files` must contain at least one glob",
                    ));
                }

                let priority = opts.get::<Option<i32>>("priority")?.unwrap_or(0);

                let mut inner = this.inner.lock();
                if inner.processors.iter().any(|p| p.def.name == name) {
                    return Err(mlua::Error::external(format!(
                        "processor `{name}` is already registered"
                    )));
                }
                inner.processors.push(RegisteredProcessor {
                    def: ProcessorDef {
                        name,
                        patterns: files,
                        priority,
                    },
                    handler,
                });
                drop(inner);
                Ok(())
            },
        );

        // plugin:generator(name, fn)
        methods.add_method(
            "generator",
            |_, this, (name, handler): (mlua::String, Function)| {
                let name = name.to_str()?.to_string();
                let mut inner = this.inner.lock();
                if inner.generator.is_some() {
                    return Err(mlua::Error::external(
                        "a plugin may declare at most one generator",
                    ));
                }
                inner.generator = Some(RegisteredGenerator { name, handler });
                Ok(())
            },
        );

        // plugin:on_start(fn)
        methods.add_method("on_start", |_, this, handler: Function| {
            this.inner.lock().on_start = Some(handler);
            Ok(())
        });

        // plugin:on_finish(fn)
        methods.add_method("on_finish", |_, this, handler: Function| {
            this.inner.lock().on_finish = Some(handler);
            Ok(())
        });
    }
}
