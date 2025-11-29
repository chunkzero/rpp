use std::path::PathBuf;

use crate::compile::{
    worker::{self, EventHandlerProvider},
    PackCompiler,
};

pub struct PackCompilerBuilder {
    event_handler_providers: Vec<Box<dyn worker::EventHandlerProvider>>,
    pack: Option<PathBuf>,
    cache_dir: Option<PathBuf>,
}

impl Default for PackCompilerBuilder {
    fn default() -> Self {
        Self {
            event_handler_providers: Vec::new(),
            pack: None,
            cache_dir: None,
        }
    }
}

impl PackCompilerBuilder {
    pub fn with_event_handler<F>(mut self, provider: F) -> Self
    where
        F: EventHandlerProvider + 'static,
    {
        self.event_handler_providers.push(Box::new(provider));
        self
    }

    pub fn with_pack<P: Into<PathBuf>>(mut self, pack: P) -> Self {
        self.pack = Some(pack.into());
        self
    }

    pub fn with_cache_dir<P: Into<PathBuf>>(mut self, cache_dir: P) -> Self {
        self.cache_dir = Some(cache_dir.into());
        self
    }

    pub fn build(self) -> Result<PackCompiler, super::CompileError> {
        let pack = self
            .pack
            .ok_or_else(|| super::CompileError::Builder("pack path must be set".into()))?;
        let cache_dir = self
            .cache_dir
            .ok_or_else(|| super::CompileError::Builder("cache_dir must be set".into()))?;

        PackCompiler::new(self.event_handler_providers, pack, cache_dir)
    }
}
