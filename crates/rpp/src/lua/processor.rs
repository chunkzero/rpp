use std::collections::HashMap;

use crate::compile::event::EventHandler;

pub struct LuaProcessor {
    lua: parking_lot::RwLock<HashMap<usize, mlua::Lua>>,
}

unsafe impl Sync for LuaProcessor {}
unsafe impl Send for LuaProcessor {}

impl EventHandler for LuaProcessor {
    fn id(&self) -> String {
        todo!()
    }

    fn handle_event(
        &self,
        thread_id: usize,
        event: crate::compile::event::BuildEvent,
    ) -> crate::Result<()> {
        let lua = match self.lua.read().get(&thread_id) {
            Some(lua) => lua.clone(),
            None => {
                let lua = mlua::Lua::new();
                self.lua.write().insert(thread_id, lua.clone());
                lua
            }
        };

        todo!()
    }
}
