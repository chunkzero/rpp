use std::path::PathBuf;
use std::sync::Mutex;

use crate::build::BuildError;
use crate::lua::{LuaProcessResult, LuaRuntime};
use crate::sandbox::SandboxContext;

use super::{Plugin, ProcessResult, ProcessingContext, ProcessorPlugin};

/// A processor plugin loaded from Lua source.
pub struct LuaProcessor {
    name: String,
    version: String,
    patterns: Vec<String>,
    priority: i32,
    source: String,
    runtime: Mutex<Option<LuaRuntime>>,
}

impl LuaProcessor {
    pub fn new(
        name: String,
        version: String,
        patterns: Vec<String>,
        priority: i32,
        source: String,
    ) -> Self {
        Self {
            name,
            version,
            patterns,
            priority,
            source,
            runtime: Mutex::new(None),
        }
    }

    fn ensure_runtime(&self) -> Result<(), BuildError> {
        let mut guard = self.runtime.lock().unwrap();
        if guard.is_none() {
            let mut rt = LuaRuntime::new()?;
            rt.load_plugin(&self.name, &self.source)?;
            *guard = Some(rt);
        }
        Ok(())
    }
}

impl Plugin for LuaProcessor {
    fn name(&self) -> &str {
        &self.name
    }
    fn version(&self) -> &str {
        &self.version
    }
}

impl ProcessorPlugin for LuaProcessor {
    fn patterns(&self) -> &[String] {
        &self.patterns
    }
    fn priority(&self) -> i32 {
        self.priority
    }

    fn process(&self, ctx: &ProcessingContext) -> Result<ProcessResult, BuildError> {
        self.ensure_runtime()?;

        let sandbox = SandboxContext::new(
            ctx.source_path
                .parent()
                .unwrap_or(ctx.source_path)
                .to_path_buf(),
            PathBuf::from("."),
        );

        let guard = self.runtime.lock().unwrap();
        let runtime = guard.as_ref().unwrap();

        let result = runtime.call_processor(
            &self.name,
            &ctx.path.to_string_lossy(),
            ctx.content,
            &sandbox,
        )?;

        match result {
            LuaProcessResult::Continue { content, path } => Ok(ProcessResult::Continue {
                content,
                output_path: path.map(PathBuf::from),
            }),
            LuaProcessResult::Skip => Ok(ProcessResult::Skip),
            LuaProcessResult::Cancel => Ok(ProcessResult::Cancel),
        }
    }
}
