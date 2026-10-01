//! Engine behavior tests using mock plugins.

mod common;

use std::sync::Arc;

use common::mock::{cache_key, MockFactory};
use common::Project;
use rpp::engine::Engine;
use rpp::model::{GeneratorHost, PluginFactory, ProcessOutcome};

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
    assert!(
        err.contains("`second`") && err.contains("plugin `first`"),
        "{err}"
    );
    assert!(err.contains("same.txt"), "{err}");
    assert!(!project.out_exists("same.txt"));
}

fn generator(
    id: &str,
    run: impl Fn(&mut dyn GeneratorHost) + Send + Sync + 'static,
) -> MockFactory {
    MockFactory::new(id, cache_key(id), |_, _file| ProcessOutcome::Unchanged).with_generator(
        move |host| {
            run(host);
            Ok(())
        },
    )
}

fn build_error(project: &Project, plugins: Vec<Arc<dyn PluginFactory>>) -> String {
    Engine::builder(project.config())
        .project_root(project.root())
        .plugins(plugins)
        .build_engine()
        .unwrap()
        .build()
        .unwrap_err()
        .to_string()
}

#[test]
fn generator_cannot_overwrite_source_output_without_override() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    let plugin = generator("gen", |host| host.emit("a.txt", b"x".to_vec()));
    let err = build_error(&project, vec![Arc::new(plugin)]);
    assert!(
        err.contains("cannot emit `a.txt`: owned by source `a.txt`"),
        "{err}"
    );
    assert!(err.contains("overrides"), "{err}");
    assert!(!project.out_exists("a.txt"));
}

#[test]
fn generator_override_glob_allows_overwrite_and_remove() {
    let project = Project::new();
    project.write_src("assets/a.txt", "a");
    project.write_src("assets/b.txt", "b");
    let plugin = generator("gen", |host| {
        host.emit("assets/a.txt", b"x".to_vec());
        host.remove("assets/b.txt");
    })
    .with_overrides(&["assets/*.txt"]);
    build(&project, vec![Arc::new(plugin)]);
    assert_eq!(project.read_out("assets/a.txt").as_deref(), Some("x"));
    assert!(!project.out_exists("assets/b.txt"));
}

#[test]
fn generator_cannot_remove_other_plugins_emit() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    let first = generator("first", |host| host.emit("x.txt", b"x".to_vec()));
    let second = generator("second", |host| host.remove("x.txt"));
    let err = build_error(&project, vec![Arc::new(first), Arc::new(second)]);
    assert!(
        err.contains("cannot remove `x.txt`: owned by plugin `first`"),
        "{err}"
    );
}

#[test]
fn generator_may_reemit_and_remove_own_outputs() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    let plugin = generator("gen", |host| {
        host.emit("x.txt", b"1".to_vec());
        host.emit("x.txt", b"2".to_vec());
        host.emit("y.txt", b"y".to_vec());
        host.remove("x.txt");
        host.remove("missing.txt");
    });
    build(&project, vec![Arc::new(plugin)]);
    assert!(!project.out_exists("x.txt"));
    assert_eq!(project.read_out("y.txt").as_deref(), Some("y"));
}

#[test]
fn replayed_generator_rechecks_ownership_against_new_source() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    let plugin: Arc<dyn PluginFactory> =
        Arc::new(generator("gen", |host| host.emit("out.txt", b"g".to_vec())));
    build(&project, vec![plugin.clone()]);
    assert_eq!(project.read_out("out.txt").as_deref(), Some("g"));

    project.write_src("out.txt", "handwritten");
    let err = build_error(&project, vec![plugin]);
    assert!(
        err.contains("cannot emit `out.txt`: owned by source `out.txt`"),
        "{err}"
    );
    assert_eq!(project.read_out("out.txt").as_deref(), Some("g"));
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
    project.write_src("window/z.txt", "z");
    project.write_src("window/a.txt", "a");

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
        Some("window/a.txt\nwindow/z.txt")
    );
    assert!(!project.out_exists("window/a.txt"));

    let unchanged = build(&project, vec![plugin.clone()]);
    assert_eq!(unchanged.generated, 0);

    project.write_src("window/m.txt", "m");
    let added = build(&project, vec![plugin]);
    assert_eq!(added.generated, 1);
    assert_eq!(
        project.read_out("sources.txt").as_deref(),
        Some("window/a.txt\nwindow/m.txt\nwindow/z.txt")
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

fn upper(id: &str, key: u64) -> MockFactory {
    MockFactory::new(id, key, |_, file| {
        file.contents = file.contents.to_ascii_uppercase();
        ProcessOutcome::Modified
    })
}

#[test]
fn plugin_key_change_keeps_other_plugins_cached() {
    let project = Project::new();
    project.write_src("a.txt", "hello");

    let stable = Arc::new(generator("stable", |host| {
        host.emit("stable.txt", b"s".to_vec())
    })) as Arc<dyn PluginFactory>;
    let other = |key: u64| {
        Arc::new(generator("other", |host| host.emit("other.txt", b"o".to_vec())).with_key(key))
            as Arc<dyn PluginFactory>
    };
    let first = build(&project, vec![stable.clone(), other(1)]);
    assert_eq!(first.generated, 2);

    let second = build(&project, vec![stable, other(2)]);
    assert_eq!(second.generated, 1);
    assert_eq!(project.read_out("stable.txt").as_deref(), Some("s"));
}

#[test]
fn processor_key_stable_reuses_files_but_reruns_generator() {
    let project = Project::new();
    project.write_src("a.txt", "hello");

    let plugin = |key: u64| {
        Arc::new(
            upper("p", key)
                .with_processor_key(7)
                .with_generator(|host| {
                    host.emit("out.txt", b"1".to_vec());
                    Ok(())
                }),
        ) as Arc<dyn PluginFactory>
    };
    let first = build(&project, vec![plugin(1)]);
    assert_eq!((first.processed, first.generated), (1, 1));

    let second = build(&project, vec![plugin(2)]);
    assert_eq!(
        (second.processed, second.cached, second.generated),
        (0, 1, 1)
    );
    assert_eq!(project.read_out("a.txt").as_deref(), Some("HELLO"));
}

#[test]
fn removing_plugin_keeps_remaining_cache() {
    let project = Project::new();
    project.write_src("a.txt", "hello");

    let keep = Arc::new(upper("keep", 1).with_generator(|host| {
        host.emit("keep.txt", b"k".to_vec());
        Ok(())
    })) as Arc<dyn PluginFactory>;
    let mut gone =
        MockFactory::new("gone", 5, |_, _| ProcessOutcome::Unchanged).with_generator(|host| {
            host.emit("gone.txt", b"g".to_vec());
            Ok(())
        });
    gone.processors.clear();
    let gone = Arc::new(gone) as Arc<dyn PluginFactory>;
    build(&project, vec![keep.clone(), gone]);
    assert!(project.out_exists("gone.txt"));

    let second = build(&project, vec![keep]);
    assert_eq!(
        (second.processed, second.cached, second.generated),
        (0, 1, 0)
    );
    assert!(!project.out_exists("gone.txt"));
    assert_eq!(project.read_out("keep.txt").as_deref(), Some("k"));
}
