use std::path::PathBuf;

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
        }
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
        // Create runtime per call (will be cached by worker pool thread-local storage)
        let runtime = LuaRuntime::from_source(&self.name, &self.source)?;

        let sandbox = SandboxContext::new(
            ctx.source_path
                .parent()
                .unwrap_or(ctx.source_path)
                .to_path_buf(),
            PathBuf::from("."),
        );

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
