//! A configurable in-memory plugin for engine tests.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use rpp::model::{
    BuildStats, GeneratorHost, PackFile, PluginFactory, PluginInstance, ProcessOutcome,
    ProcessorDef,
};

type ProcessFn = dyn Fn(&str, &mut PackFile) -> rpp::Result<ProcessOutcome> + Send + Sync;
type GenerateFn = dyn Fn(&mut dyn GeneratorHost) -> rpp::Result<()> + Send + Sync;
type StartFn = dyn Fn() -> rpp::Result<()> + Send + Sync;
type FinishFn = dyn Fn(&BuildStats) -> rpp::Result<()> + Send + Sync;

pub struct MockFactory {
    id: String,
    key: u64,
    processor_key: Option<u64>,
    pub processors: Vec<ProcessorDef>,
    has_generator: bool,
    behavior: Arc<ProcessFn>,
    generator: Option<Arc<GenerateFn>>,
    on_start: Option<Arc<StartFn>>,
    on_finish: Option<Arc<FinishFn>>,
    overrides: Vec<String>,
    output_roots: BTreeMap<String, PathBuf>,
    authoring_source: Option<String>,
    instantiations: Arc<AtomicUsize>,
}

impl MockFactory {
    /// A plugin with one processor `p` matching every file.
    pub fn new(
        id: &str,
        key: u64,
        behavior: impl Fn(&str, &mut PackFile) -> ProcessOutcome + Send + Sync + 'static,
    ) -> Self {
        Self::fallible(id, key, move |id, file| Ok(behavior(id, file)))
    }

    /// Like [`MockFactory::new`], for processors that can fail.
    pub fn fallible(
        id: &str,
        key: u64,
        behavior: impl Fn(&str, &mut PackFile) -> rpp::Result<ProcessOutcome> + Send + Sync + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            key,
            processor_key: None,
            processors: vec![ProcessorDef {
                name: "p".into(),
                patterns: vec!["**/*".into()],
                priority: 0,
            }],
            has_generator: false,
            behavior: Arc::new(behavior),
            generator: None,
            on_start: None,
            on_finish: None,
            overrides: Vec::new(),
            output_roots: BTreeMap::new(),
            authoring_source: None,
            instantiations: Arc::default(),
        }
    }

    /// Count every `instantiate` call in `counter`.
    pub fn with_instantiations(mut self, counter: Arc<AtomicUsize>) -> Self {
        self.instantiations = counter;
        self
    }

    pub fn with_overrides(mut self, globs: &[&str]) -> Self {
        self.overrides = globs.iter().map(|glob| glob.to_string()).collect();
        self
    }

    pub fn with_processor_key(mut self, key: u64) -> Self {
        self.processor_key = Some(key);
        self
    }

    pub fn with_key(mut self, key: u64) -> Self {
        self.key = key;
        self
    }

    pub fn with_patterns(mut self, patterns: &[&str]) -> Self {
        self.processors[0].patterns = patterns.iter().map(|p| p.to_string()).collect();
        self
    }

    pub fn with_priority(mut self, priority: i32) -> Self {
        self.processors[0].priority = priority;
        self
    }

    pub fn with_output_root(mut self, name: &str, path: &str) -> Self {
        self.output_roots.insert(name.into(), path.into());
        self
    }

    pub fn with_authoring_source(mut self, rel: &str) -> Self {
        self.authoring_source = Some(rel.into());
        self
    }

    pub fn with_generator(
        mut self,
        generator: impl Fn(&mut dyn GeneratorHost) -> rpp::Result<()> + Send + Sync + 'static,
    ) -> Self {
        self.has_generator = true;
        self.generator = Some(Arc::new(generator));
        self
    }

    pub fn with_hooks(
        mut self,
        on_start: impl Fn() -> rpp::Result<()> + Send + Sync + 'static,
        on_finish: impl Fn(&BuildStats) -> rpp::Result<()> + Send + Sync + 'static,
    ) -> Self {
        self.on_start = Some(Arc::new(on_start));
        self.on_finish = Some(Arc::new(on_finish));
        self
    }
}

impl PluginFactory for MockFactory {
    fn id(&self) -> &str {
        &self.id
    }

    fn cache_key(&self) -> u64 {
        self.key
    }

    fn processor_key(&self) -> u64 {
        self.processor_key.unwrap_or(self.key)
    }

    fn processors(&self) -> &[ProcessorDef] {
        &self.processors
    }

    fn has_generator(&self) -> bool {
        self.has_generator
    }

    fn overrides(&self) -> &[String] {
        &self.overrides
    }

    fn output_roots(&self) -> BTreeMap<String, PathBuf> {
        self.output_roots.clone()
    }

    fn is_authoring_source(&self, rel: &str) -> bool {
        self.authoring_source.as_deref() == Some(rel)
    }

    fn instantiate(&self) -> rpp::Result<Box<dyn PluginInstance>> {
        self.instantiations.fetch_add(1, Ordering::Relaxed);
        Ok(Box::new(MockInstance {
            id: self.id.clone(),
            behavior: Arc::clone(&self.behavior),
            generator: self.generator.clone(),
            on_start: self.on_start.clone(),
            on_finish: self.on_finish.clone(),
        }))
    }
}

struct MockInstance {
    id: String,
    behavior: Arc<ProcessFn>,
    generator: Option<Arc<GenerateFn>>,
    on_start: Option<Arc<StartFn>>,
    on_finish: Option<Arc<FinishFn>>,
}

impl PluginInstance for MockInstance {
    fn process(&mut self, _processor: &str, file: &mut PackFile) -> rpp::Result<ProcessOutcome> {
        (self.behavior)(&self.id, file)
    }

    fn generate(&mut self, host: &mut dyn GeneratorHost) -> rpp::Result<()> {
        self.generator.as_ref().map_or(Ok(()), |run| run(host))
    }

    fn on_build_start(&mut self) -> rpp::Result<()> {
        self.on_start.as_ref().map_or(Ok(()), |run| run())
    }

    fn on_build_finish(&mut self, stats: &BuildStats) -> rpp::Result<()> {
        self.on_finish.as_ref().map_or(Ok(()), |run| run(stats))
    }
}

/// A cache key derived from `label`, distinct per label.
pub fn cache_key(label: &str) -> u64 {
    label.len() as u64 + label.bytes().map(u64::from).sum::<u64>()
}
