use super::Plugin;
use crate::build::BuildError;
use std::path::{Path, PathBuf};

/// Context passed to processor plugins during file processing.
pub struct ProcessingContext<'a> {
    /// Relative path of the file being processed
    pub path: &'a Path,
    /// Current file content (may have been modified by previous processors)
    pub content: &'a [u8],
    /// Original source path
    pub source_path: &'a Path,
    /// Plugin configuration from rpp.toml
    pub config: &'a toml::Value,
}

/// Result returned by a processor after processing a file.
#[derive(Debug)]
pub enum ProcessResult {
    /// Continue processing with new content.
    /// Optionally change the output path.
    Continue {
        content: Vec<u8>,
        output_path: Option<PathBuf>,
    },

    /// Skip this processor, continue with next in chain.
    /// Content remains unchanged.
    Skip,

    /// Cancel processing this file entirely.
    /// File will not appear in output.
    Cancel,
}

impl ProcessResult {
    /// Create a Continue result with just new content.
    pub fn continue_with(content: Vec<u8>) -> Self {
        Self::Continue {
            content,
            output_path: None,
        }
    }

    /// Create a Continue result that also changes the output path.
    pub fn continue_with_path(content: Vec<u8>, path: PathBuf) -> Self {
        Self::Continue {
            content,
            output_path: Some(path),
        }
    }
}

/// Trait for processor plugins that transform file content.
pub trait ProcessorPlugin: Plugin {
    /// Glob patterns this processor handles (e.g., ["*.json", "**/*.mcmeta"]).
    fn patterns(&self) -> &[String];

    /// Priority for ordering in the processor chain.
    /// Lower values run first. Default is 100.
    fn priority(&self) -> i32 {
        100
    }

    /// Process a file and return the result.
    fn process(&self, ctx: &ProcessingContext) -> Result<ProcessResult, BuildError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_result_constructors() {
        let content = vec![1, 2, 3];
        let result = ProcessResult::continue_with(content.clone());
        match result {
            ProcessResult::Continue {
                content: c,
                output_path,
            } => {
                assert_eq!(c, content);
                assert!(output_path.is_none());
            }
            _ => panic!("Expected Continue"),
        }
    }
}
