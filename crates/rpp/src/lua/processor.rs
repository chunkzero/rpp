use std::sync::Arc;

use crate::compile::processor::Processor;

pub struct LuaProcessor {
    lua: mlua::Lua,
}

impl Processor for LuaProcessor {
    fn description(&self) -> String {
        todo!()
    }

    fn process(
        &self,
        context: &mut crate::compile::processor::FileProcessContext,
    ) -> crate::Result<()> {
        todo!()
    }
}

impl Processor for Arc<LuaProcessor> {
    fn description(&self) -> String {
        LuaProcessor::description(self)
    }

    fn process(
        &self,
        context: &mut crate::compile::processor::FileProcessContext,
    ) -> crate::Result<()> {
        LuaProcessor::process(self, context)
    }
}
