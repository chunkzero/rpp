use mlua::Lua;

use crate::compile::event::EventHandler;

pub struct LuaProcessor {
    lua: Lua,
}

impl LuaProcessor {
    pub fn new() -> LuaProcessor {
        Self {
            lua: mlua::Lua::new(),
        }
    }
}

impl EventHandler for LuaProcessor {
    fn id(&self) -> String {
        todo!()
    }

    fn handle_event(
        &self,
        thread_id: usize,
        event: crate::compile::event::BuildEvent,
    ) -> crate::Result<()> {
        todo!()
    }
}
