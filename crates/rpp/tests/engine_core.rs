//! Engine behavior tests using mock plugins (no Lua runtime).

mod common;

use std::sync::Arc;

use common::Project;
use rpp::engine::Engine;
use rpp::model::{
    BuildStats, GeneratorHost, PackFile, PluginFactory, PluginInstance, ProcessOutcome,
    ProcessorDef,
};

type ProcessFn = dyn Fn(&str, &mut PackFile) -> ProcessOutcome + Send + Sync;
type GenerateFn = dyn Fn(&mut dyn GeneratorHost) -> rpp::Result<()> + Send + Sync;

struct MockFactory {
    id: String,
    key: u64,
    processors: Vec<ProcessorDef>,
    has_generator: bool,
    behavior: Arc<ProcessFn>,
    generator: Option<Arc<GenerateFn>>,
}

impl MockFactory {
    fn new(
        id: &str,
        key: u64,
        behavior: impl Fn(&str, &mut PackFile) -> ProcessOutcome + Send + Sync + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            key,
            processors: vec![ProcessorDef {
                name: "p".into(),
                patterns: vec!["**/*".into()],
                priority: 0,
            }],
            has_generator: false,
            behavior: Arc::new(behavior),
            generator: None,
        }
    }

    fn with_generator(
        mut self,
        generator: impl Fn(&mut dyn GeneratorHost) -> rpp::Result<()> + Send + Sync + 'static,
    ) -> Self {
        self.has_generator = true;
        self.generator = Some(Arc::new(generator));
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

    fn processors(&self) -> &[ProcessorDef] {
        &self.processors
    }

    fn has_generator(&self) -> bool {
        self.has_generator
    }

    fn instantiate(&self) -> rpp::Result<Box<dyn PluginInstance>> {
        Ok(Box::new(MockInstance {
            id: self.id.clone(),
            behavior: Arc::clone(&self.behavior),
            generator: self.generator.clone(),
        }))
    }
}

struct MockInstance {
    id: String,
    behavior: Arc<ProcessFn>,
    generator: Option<Arc<GenerateFn>>,
}

impl PluginInstance for MockInstance {
    fn process(&mut self, _processor: &str, file: &mut PackFile) -> rpp::Result<ProcessOutcome> {
        Ok((self.behavior)(&self.id, file))
    }

    fn generate(&mut self, host: &mut dyn GeneratorHost) -> rpp::Result<()> {
        if let Some(generator) = &self.generator {
            generator(host)
        } else {
            Ok(())
        }
    }

    fn on_build_start(&mut self) -> rpp::Result<()> {
        Ok(())
    }

    fn on_build_finish(&mut self, _stats: &BuildStats) -> rpp::Result<()> {
        Ok(())
    }
}

fn cache_key(label: &str) -> u64 {
    label.len() as u64 + label.bytes().map(u64::from).sum::<u64>()
}

fn build(project: &Project, plugins: Vec<Arc<dyn PluginFactory>>) -> rpp::engine::BuildResult {
    let engine = Engine::builder(project.config())
        .project_root(project.root())
        .plugins(plugins)
        .build_engine()
        .unwrap();
    engine.build().unwrap()
}

#[test]
fn generator_invalid_emit_path_fails() {
    let project = Project::new();
    project.write_src("a.txt", "a");

    let plugin = Arc::new(
        MockFactory::new("escape", cache_key("escape"), |_, _file| {
            ProcessOutcome::Unchanged
        })
        .with_generator(|host| {
            host.emit("../escaped.txt", b"nope".to_vec());
            Ok(())
        }),
    );

    let engine = Engine::builder(project.config())
        .project_root(project.root())
        .plugin(plugin)
        .build_engine()
        .unwrap();
    let err = engine.build().unwrap_err().to_string();
    assert!(err.contains("invalid emit path"), "{err}");
}

#[test]
fn colliding_generator_outputs_fail() {
    let project = Project::new();
    project.write_src("a.txt", "a");

    let make = |id: &str, key: u64| {
        let contents = id.as_bytes().to_vec();
        Arc::new(
            MockFactory::new(id, key, |_, _file| ProcessOutcome::Unchanged).with_generator(
                move |host| {
                    host.emit("same.txt", contents.clone());
                    Ok(())
                },
            ),
        )
    };

    let engine = Engine::builder(project.config())
        .project_root(project.root())
        .plugins(vec![
            make("first", cache_key("first")) as Arc<dyn PluginFactory>,
            make("second", cache_key("second")) as Arc<dyn PluginFactory>,
        ])
        .build_engine()
        .unwrap();
    let err = engine.build().unwrap_err().to_string();
    assert!(err.contains("already claimed"), "{err}");
}

#[test]
fn generator_cache_replay_does_not_increment_generated() {
    let project = Project::new();
    project.write_src("a.txt", "a");

    let plugin = Arc::new(
        MockFactory::new("gen", cache_key("gen"), |_, _file| {
            ProcessOutcome::Unchanged
        })
        .with_generator(|host| {
            host.emit("out.txt", b"1".to_vec());
            Ok(())
        }),
    );

    let first = build(&project, vec![plugin.clone()]);
    assert_eq!(first.generated, 1);

    let second = build(&project, vec![plugin]);
    assert_eq!(second.generated, 0);
    assert_eq!(project.read_out("out.txt").as_deref(), Some("1"));
}

#[test]
fn raw_source_list_is_sorted_and_invalidates_generator_cache() {
    let project = Project::new();
    project.write_src("window/z.lua", "z");
    project.write_src("window/a.lua", "a");

    let plugin = Arc::new(
        MockFactory::new("sources", cache_key("sources"), |_, _file| {
            ProcessOutcome::Dropped
        })
        .with_generator(|host| {
            let files = host.list_source_files(Some("window/**"));
            host.emit("sources.txt", files.join("\n").into_bytes());
            Ok(())
        }),
    );

    let first = build(&project, vec![plugin.clone()]);
    assert_eq!(first.generated, 1);
    assert_eq!(
        project.read_out("sources.txt").as_deref(),
        Some("window/a.lua\nwindow/z.lua")
    );
    assert!(!project.out_exists("window/a.lua"));

    let unchanged = build(&project, vec![plugin.clone()]);
    assert_eq!(unchanged.generated, 0);

    project.write_src("window/m.lua", "m");
    let added = build(&project, vec![plugin]);
    assert_eq!(added.generated, 1);
    assert_eq!(
        project.read_out("sources.txt").as_deref(),
        Some("window/a.lua\nwindow/m.lua\nwindow/z.lua")
    );
}

#[test]
fn incremental_file_cache_uses_cas_without_reprocessing() {
    let project = Project::new();
    project.write_src("a.txt", "hello");

    let plugin = Arc::new(MockFactory::new("upper", cache_key("upper"), |_, file| {
        file.contents = file.contents.to_ascii_uppercase();
        ProcessOutcome::Modified
    }));

    let first = build(&project, vec![plugin.clone()]);
    assert_eq!(first.processed, 1);

    let second = build(&project, vec![plugin]);
    assert_eq!(second.processed, 0);
    assert_eq!(second.cached, 1);
    assert_eq!(project.read_out("a.txt").as_deref(), Some("HELLO"));
}
