use crate::build::processor::Processor;

pub struct LuaProcessor {
    lua: mlua::Lua,
}

impl Processor for LuaProcessor {
    fn description(&self) -> String {
        todo!()
    }

    fn process(
        &self,
        context: &mut crate::build::processor::FileProcessContext,
    ) -> crate::Result<()> {
        todo!()
    }
}
