//! Output ownership, collisions, overrides, and path boundaries for processors and
//! generators.

mod common;

use std::sync::Arc;

use common::engine::{build, build_error, engine, generator, generator_factory};
use common::mock::{cache_key, MockFactory};
use common::Project;
use rpp::model::{PluginFactory, ProcessOutcome};

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

    let err = build_error(&project, vec![plugin]);
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

    let err = build_error(
        &project,
        vec![
            make("first", cache_key("first")) as Arc<dyn PluginFactory>,
            make("second", cache_key("second")) as Arc<dyn PluginFactory>,
        ],
    );
    assert!(
        err.contains("`second`") && err.contains("plugin `first`"),
        "{err}"
    );
    assert!(err.contains("same.txt"), "{err}");
    assert!(!project.out_exists("same.txt"));
}

#[test]
fn generator_cannot_overwrite_source_output_without_override() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    let plugin = generator_factory("gen", |host| host.emit("a.txt", b"x".to_vec()));
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
    let plugin = generator_factory("gen", |host| {
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
    let first = generator_factory("first", |host| host.emit("x.txt", b"x".to_vec()));
    let second = generator_factory("second", |host| host.remove("x.txt"));
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
    let plugin = generator_factory("gen", |host| {
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
    let plugin: Arc<dyn PluginFactory> = Arc::new(generator_factory("gen", |host| {
        host.emit("out.txt", b"g".to_vec())
    }));
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
fn processor_cannot_escape_output_directory() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    let plugin = MockFactory::new("escape", 1, |_, file| {
        file.path = "../escaped.txt".into();
        ProcessOutcome::Modified
    });
    assert!(engine(&project, vec![Arc::new(plugin)]).build().is_err());
    assert!(!project.root().join("escaped.txt").exists());
}

#[test]
fn generator_cannot_escape_source_or_output_directories() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    std::fs::write(project.root().join("secret.txt"), "secret").unwrap();
    let plugin = generator("escape", |host| {
        let secret = host.read_source("../secret.txt");
        assert!(secret.is_none());
        host.emit("../escaped.txt", b"x".to_vec());
    });
    assert!(engine(&project, vec![plugin]).build().is_err());
    assert!(!project.root().join("escaped.txt").exists());
}
