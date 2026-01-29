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

    pub fn source(&self) -> &str {
        &self.source
    }
}

impl Plugin for LuaProcessor {
    fn name(&self) -> &str {
        &self.name
    }
    fn version(&self) -> &str {
        &self.version
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
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
        // Fallback for non-chain processing (kept for compatibility)
        // This won't be called in the optimized path
        let mut runtime = LuaRuntime::new()?;
        runtime.load_plugin_versioned(&self.name, &self.version, &self.source)?;

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
