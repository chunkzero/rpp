//! Processing chain: ordering, drops, ignore rules, errors, and lifecycle hooks.

mod common;

use std::sync::{Arc, Mutex};

use common::engine::{build, engine, noop};
use common::mock::{cache_key, MockFactory};
use common::Project;
use rpp::model::ProcessOutcome;

fn appender(id: &str, mark: &'static str) -> MockFactory {
    MockFactory::new(id, cache_key(id), move |_, file| {
        file.contents.extend_from_slice(mark.as_bytes());
        ProcessOutcome::Modified
    })
}

#[test]
fn authoring_sources_are_excluded_from_output_and_processors() {
    let project = Project::new();
    project.write_src("authoring.json", "{}");
    project.write_src("keep.json", "{}");

    let plugin = MockFactory::new("authoring", 1, |_, file| {
        file.contents = b"processed".to_vec();
        ProcessOutcome::Modified
    })
    .with_patterns(&["**/*.json"])
    .with_authoring_source("authoring.json")
    .with_generator(|host| {
        let files = host.list_source_files(None).join(",");
        host.emit("sources.txt", files.into_bytes());
        Ok(())
    });

    let result = build(&project, vec![Arc::new(plugin)]);
    assert_eq!(result.processed, 1);
    assert!(!project.out_exists("authoring.json"));
    assert_eq!(project.read_out("keep.json").as_deref(), Some("processed"));
    assert_eq!(
        project.read_out("sources.txt").as_deref(),
        Some("keep.json")
    );
}

#[test]
fn chain_ordering_across_plugins() {
    let project = Project::new();
    project.write_src("x.txt", "");

    // `late` has the higher priority number and runs after `early`, even though it is listed first.
    let early = Arc::new(appender("early", "A").with_priority(10));
    let late = Arc::new(appender("late", "B").with_priority(20));
    let result = build(&project, vec![late, early]);
    assert_eq!(result.processed, 1);
    assert_eq!(project.read_out("x.txt").as_deref(), Some("AB"));
}

#[test]
fn chain_tie_broken_by_plugin_order() {
    let project = Project::new();
    project.write_src("x.txt", "");

    let result = build(
        &project,
        vec![
            Arc::new(appender("first", "1")),
            Arc::new(appender("second", "2")),
        ],
    );
    assert_eq!(result.processed, 1);
    assert_eq!(project.read_out("x.txt").as_deref(), Some("12"));
}

#[test]
fn dropped_file_excluded_from_output() {
    let project = Project::new();
    project.write_src("keep.txt", "k");
    project.write_src("drop.txt", "d");

    let dropper =
        MockFactory::new("dropper", 1, |_, _| ProcessOutcome::Dropped).with_patterns(&["drop.txt"]);
    let result = build(&project, vec![Arc::new(dropper)]);
    assert_eq!(result.dropped, 1);
    assert!(project.out_exists("keep.txt"));
    assert!(!project.out_exists("drop.txt"));
}

#[test]
fn rppignore_excludes_files() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    project.write_src("ignored.tmp", "x");
    std::fs::write(project.src().join(".rppignore"), "*.tmp\n").unwrap();

    build(&project, vec![noop()]);
    assert!(project.out_exists("a.txt"));
    assert!(!project.out_exists("ignored.tmp"));
}

#[test]
fn processor_error_publishes_nothing() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    project.write_src("bad.txt", "bad");
    let plugin = MockFactory::fallible("failing", 1, |id, file| {
        if file.path == "bad.txt" {
            return Err(rpp::Error::Processor {
                plugin: id.into(),
                processor: "p".into(),
                file: file.path.clone(),
                message: "boom".into(),
            });
        }
        file.contents = b"processed".to_vec();
        Ok(ProcessOutcome::Modified)
    });
    let error = engine(&project, vec![Arc::new(plugin)])
        .build()
        .unwrap_err()
        .to_string();
    assert!(error.contains("boom"), "{error}");
    assert!(!project.root().join("dist").exists());
    assert!(!project.root().join(".rpp/cache/manifest.bin").exists());
}

#[test]
fn colliding_processor_outputs_fail_deterministically() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    project.write_src("b.txt", "b");
    let plugin = MockFactory::new("rename", 1, |_, file| {
        file.path = "same.txt".into();
        ProcessOutcome::Modified
    });
    let error = engine(&project, vec![Arc::new(plugin)])
        .build()
        .unwrap_err()
        .to_string();
    assert!(error.contains("both produce `same.txt`"), "{error}");
}

#[test]
fn lifecycle_hooks_run() {
    let project = Project::new();
    project.write_src("a.txt", "a");

    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let (on_process, on_generate, on_start, on_finish) = (
        events.clone(),
        events.clone(),
        events.clone(),
        events.clone(),
    );
    let plugin = MockFactory::new("hooks", 1, move |_, file| {
        on_process.lock().unwrap().push("process".into());
        file.contents.extend_from_slice(b" processed");
        ProcessOutcome::Modified
    })
    .with_generator(move |host| {
        on_generate.lock().unwrap().push("generate".into());
        let body = host.read_file("a.txt").unwrap();
        host.emit("generated.txt", body);
        Ok(())
    })
    .with_hooks(
        move || {
            on_start.lock().unwrap().push("start".into());
            Ok(())
        },
        move |stats| {
            on_finish
                .lock()
                .unwrap()
                .push(format!("finish {} {}", stats.processed, stats.generated));
            Ok(())
        },
    );

    let result = build(&project, vec![Arc::new(plugin)]);
    assert_eq!((result.processed, result.generated), (1, 1));
    assert_eq!(
        project.read_out("generated.txt").as_deref(),
        Some("a processed")
    );
    assert_eq!(
        *events.lock().unwrap(),
        ["start", "process", "generate", "finish 1 1"]
    );
}

#[test]
fn failing_finish_hook_does_not_commit_outputs_or_manifest() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    let plugin = MockFactory::new("finish-error", 1, |_, _| ProcessOutcome::Unchanged)
        .with_generator(|host| {
            host.emit("generated.txt", b"generated".to_vec());
            Ok(())
        })
        .with_hooks(
            || Ok(()),
            |_| {
                Err(rpp::Error::Hook {
                    plugin: "finish-error".into(),
                    hook: "onFinish".into(),
                    message: "finish failed".into(),
                })
            },
        );
    let error = engine(&project, vec![Arc::new(plugin)])
        .build()
        .unwrap_err()
        .to_string();
    assert!(error.contains("finish failed"), "{error}");
    assert!(!project.root().join("dist/generated.txt").exists());
    assert!(!project.root().join(".rpp/cache/manifest.bin").exists());
}
