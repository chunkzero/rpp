//! Helpers for running builds with mock plugins.

use std::sync::Arc;

use rpp::engine::{BuildResult, Engine};
use rpp::model::{GeneratorHost, PluginFactory, ProcessOutcome};

use super::mock::{cache_key, MockFactory};
use super::Project;

pub fn engine(project: &Project, plugins: Vec<Arc<dyn PluginFactory>>) -> Engine {
    Engine::builder(project.config())
        .project_root(project.root())
        .plugins(plugins)
        .build_engine()
        .unwrap()
}

pub fn build(project: &Project, plugins: Vec<Arc<dyn PluginFactory>>) -> BuildResult {
    engine(project, plugins).build().unwrap()
}

/// Run a build that must fail and return the error message.
pub fn build_error(project: &Project, plugins: Vec<Arc<dyn PluginFactory>>) -> String {
    engine(project, plugins).build().unwrap_err().to_string()
}

/// A plugin with no effective processing that runs `run` as its generator.
pub fn generator_factory(
    id: &str,
    run: impl Fn(&mut dyn GeneratorHost) + Send + Sync + 'static,
) -> MockFactory {
    MockFactory::new(id, cache_key(id), |_, _| ProcessOutcome::Unchanged).with_generator(
        move |host| {
            run(host);
            Ok(())
        },
    )
}

pub fn generator(
    id: &str,
    run: impl Fn(&mut dyn GeneratorHost) + Send + Sync + 'static,
) -> Arc<dyn PluginFactory> {
    Arc::new(generator_factory(id, run))
}

/// A processor that upper-cases every file.
pub fn upper_factory(id: &str, key: u64) -> MockFactory {
    MockFactory::new(id, key, |_, file| {
        file.contents = file.contents.to_ascii_uppercase();
        ProcessOutcome::Modified
    })
}

pub fn upper(key: u64) -> Arc<dyn PluginFactory> {
    Arc::new(upper_factory("upper", key))
}

pub fn noop() -> Arc<dyn PluginFactory> {
    Arc::new(MockFactory::new("noop", 1, |_, _| {
        ProcessOutcome::Unchanged
    }))
}
