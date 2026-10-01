//! End-to-end build engine tests with mock plugins: chain ordering, generators,
//! incremental cache, lifecycle hooks, and path boundaries.

mod common;

use std::sync::{Arc, Mutex};

use common::mock::{cache_key, MockFactory};
use common::Project;
use rpp::engine::Engine;
use rpp::model::{PackFile, PluginFactory, ProcessOutcome};

fn build(project: &Project, plugins: Vec<Arc<dyn PluginFactory>>) -> rpp::engine::BuildResult {
    engine(project, plugins).build().unwrap()
}

fn engine(project: &Project, plugins: Vec<Arc<dyn PluginFactory>>) -> Engine {
    Engine::builder(project.config())
        .project_root(project.root())
        .plugins(plugins)
        .build_engine()
        .unwrap()
}

fn upper_case(file: &mut PackFile) -> ProcessOutcome {
    file.contents = file.contents.to_ascii_uppercase();
    ProcessOutcome::Modified
}

fn upper(key: u64) -> Arc<dyn PluginFactory> {
    Arc::new(MockFactory::new("upper", key, |_, file| upper_case(file)))
}

fn noop() -> Arc<dyn PluginFactory> {
    Arc::new(MockFactory::new("noop", 1, |_, _| {
        ProcessOutcome::Unchanged
    }))
}

fn appender(id: &str, mark: &'static str) -> MockFactory {
    MockFactory::new(id, cache_key(id), move |_, file| {
        file.contents.extend_from_slice(mark.as_bytes());
        ProcessOutcome::Modified
    })
}

fn generator(
    id: &str,
    run: impl Fn(&mut dyn rpp::model::GeneratorHost) + Send + Sync + 'static,
) -> Arc<dyn PluginFactory> {
    Arc::new(
        MockFactory::new(id, cache_key(id), |_, _| ProcessOutcome::Unchanged).with_generator(
            move |host| {
                run(host);
                Ok(())
            },
        ),
    )
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
fn generator_emit_and_read() {
    let project = Project::new();
    project.write_src("assets/a.txt", "one");
    project.write_src("assets/b.txt", "two");

    let plugin = generator("indexer", |host| {
        let mut names = Vec::new();
        for path in host.list_files(Some("assets/*.txt")) {
            let body = String::from_utf8(host.read_file(&path).unwrap()).unwrap();
            names.push(format!("{path}={body}"));
        }
        names.sort();
        host.emit("index.txt", names.join("\n").into_bytes());
    });

    let result = build(&project, vec![plugin]);
    assert_eq!(result.generated, 1);
    assert_eq!(
        project.read_out("index.txt").as_deref(),
        Some("assets/a.txt=one\nassets/b.txt=two")
    );
}

#[test]
fn generator_read_source_sees_the_unprocessed_file() {
    let project = Project::new();
    project.write_src("raw.txt", "RAWBODY");

    let plugin = MockFactory::new("src-reader", 1, |_, file| {
        file.contents = b"PROCESSED".to_vec();
        ProcessOutcome::Modified
    })
    .with_patterns(&["raw.txt"])
    .with_generator(|host| {
        let raw = host.read_source("raw.txt").unwrap();
        let processed = host.read_file("raw.txt").unwrap();
        host.emit("report.txt", [raw, b"|".to_vec(), processed].concat());
        Ok(())
    });

    build(&project, vec![Arc::new(plugin)]);
    assert_eq!(
        project.read_out("report.txt").as_deref(),
        Some("RAWBODY|PROCESSED")
    );
}

#[test]
fn incremental_touch_one_file_reprocesses_only_it() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    project.write_src("b.txt", "b");

    build(&project, vec![upper(1)]);
    project.write_src("a.txt", "changed");

    let second = build(&project, vec![upper(1)]);
    assert_eq!(second.processed, 1);
    assert_eq!(second.cached, 1);
    assert_eq!(project.read_out("a.txt").as_deref(), Some("CHANGED"));
}

#[test]
fn changing_cache_key_reprocesses() {
    let project = Project::new();
    project.write_src("a.txt", "x");

    let versioned = |key: u64, tag: &'static str| -> Arc<dyn PluginFactory> {
        Arc::new(MockFactory::new("p", key, move |_, file| {
            file.contents = [tag.as_bytes(), &file.contents].concat();
            ProcessOutcome::Modified
        }))
    };
    let first = build(&project, vec![versioned(1, "A")]);
    assert_eq!(first.processed, 1);
    assert_eq!(project.read_out("a.txt").as_deref(), Some("Ax"));

    let second = build(&project, vec![versioned(2, "B")]);
    assert_eq!((second.processed, second.cached), (1, 0));
    assert_eq!(project.read_out("a.txt").as_deref(), Some("Bx"));
}

#[test]
fn output_sync_removes_stale_files() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    project.write_src("b.txt", "b");

    build(&project, vec![noop()]);
    assert!(project.out_exists("a.txt"));
    assert!(project.out_exists("b.txt"));

    std::fs::remove_file(project.src().join("b.txt")).unwrap();
    let report = build(&project, vec![noop()]);
    assert!(project.out_exists("a.txt"));
    assert!(!project.out_exists("b.txt"));
    assert!(report.changes.removed.contains(&"b.txt".to_string()));
}

#[test]
fn generator_incremental_reuses_when_inputs_unchanged() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    let count = || {
        generator("g", |host| {
            let n = host.list_files(Some("**/*.txt")).len();
            host.emit("count.txt", n.to_string().into_bytes());
        })
    };

    let first = build(&project, vec![count()]);
    assert_eq!(first.generated, 1);
    assert_eq!(project.read_out("count.txt").as_deref(), Some("1"));

    let second = build(&project, vec![count()]);
    assert_eq!(second.generated, 0);
    assert_eq!(project.read_out("count.txt").as_deref(), Some("1"));
    assert!(second.changes.written.is_empty());

    // A new file changes the generator's file-list read set, so it reruns.
    project.write_src("b.txt", "b");
    build(&project, vec![count()]);
    assert_eq!(project.read_out("count.txt").as_deref(), Some("2"));
}

#[test]
fn generator_source_reads_track_content_and_removal_of_dropped_sources() {
    let project = Project::new();
    project.write_src("window/z.txt", "z");
    project.write_src("window/a.txt", "a");

    let plugin = || -> Arc<dyn PluginFactory> {
        Arc::new(
            MockFactory::new("source-generator", 1, |_, _| ProcessOutcome::Dropped)
                .with_patterns(&["window/**"])
                .with_generator(|host| {
                    let mut documents = Vec::new();
                    for path in host.list_source_files(Some("window/**")) {
                        let body = String::from_utf8(host.read_source(&path).unwrap()).unwrap();
                        documents.push(format!("{path}={body}"));
                    }
                    host.emit("documents.txt", documents.join("\n").into_bytes());
                    Ok(())
                }),
        )
    };

    let first = build(&project, vec![plugin()]);
    assert_eq!(first.generated, 1);
    assert_eq!(
        project.read_out("documents.txt").as_deref(),
        Some("window/a.txt=a\nwindow/z.txt=z")
    );
    assert!(!project.out_exists("window/a.txt"));

    assert_eq!(build(&project, vec![plugin()]).generated, 0);

    project.write_src("window/a.txt", "changed");
    assert_eq!(build(&project, vec![plugin()]).generated, 1);
    assert_eq!(
        project.read_out("documents.txt").as_deref(),
        Some("window/a.txt=changed\nwindow/z.txt=z")
    );

    std::fs::remove_file(project.src().join("window/z.txt")).unwrap();
    assert_eq!(build(&project, vec![plugin()]).generated, 1);
    assert_eq!(
        project.read_out("documents.txt").as_deref(),
        Some("window/a.txt=changed")
    );
}

#[test]
fn clean_removes_output_and_cache() {
    let project = Project::new();
    project.write_src("a.txt", "a");

    let engine = engine(&project, vec![noop()]);
    engine.build().unwrap();
    assert!(project.out_exists("a.txt"));

    engine.clean().unwrap();
    assert!(!project.root().join("dist").exists());
    assert!(!project.root().join(".rpp").exists());
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

#[test]
fn cached_generator_replays_removals() {
    let project = Project::new();
    project.write_src("remove.txt", "remove me");
    let plugin: Arc<dyn PluginFactory> = Arc::new(
        MockFactory::new("remover", 1, |_, _| ProcessOutcome::Unchanged)
            .with_overrides(&["remove.txt"])
            .with_generator(|host| {
                host.remove("remove.txt");
                Ok(())
            }),
    );

    build(&project, vec![plugin.clone()]);
    assert!(!project.out_exists("remove.txt"));
    let second = build(&project, vec![plugin]);
    assert!(!project.out_exists("remove.txt"));
    assert!(second.changes.written.is_empty());
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
fn generator_reads_use_the_pre_generator_snapshot() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    let plugin = || {
        generator("snapshot", |host| {
            host.emit("marker.txt", b"marker".to_vec());
            let observed = host.list_files(None).join("\n");
            host.emit("observed.txt", observed.into_bytes());
        })
    };

    let first = build(&project, vec![plugin()]);
    assert_eq!(first.generated, 1);
    assert_eq!(project.read_out("observed.txt").as_deref(), Some("a.txt"));
    let second = build(&project, vec![plugin()]);
    assert_eq!(second.generated, 0);
    assert_eq!(project.read_out("observed.txt").as_deref(), Some("a.txt"));
    assert!(second.changes.written.is_empty());
}

#[test]
fn corrupt_cache_object_is_rebuilt() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    build(&project, Vec::new());

    let objects = project.root().join(".rpp/cache/objects");
    let object = std::fs::read_dir(&objects)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    std::fs::write(object, "corrupt").unwrap();

    // The output directory still holds the right bytes, so this is a hit.
    let second = build(&project, Vec::new());
    assert_eq!(second.cached, 1);
    assert_eq!(project.read_out("a.txt").as_deref(), Some("a"));

    // Once the output is gone the corrupt object cannot be materialized and
    // the file is reprocessed.
    std::fs::remove_dir_all(project.root().join("dist")).unwrap();
    let third = build(&project, Vec::new());
    assert_eq!(third.processed, 1);
    assert_eq!(project.read_out("a.txt").as_deref(), Some("a"));
}

#[test]
fn generator_reads_linked_output_when_cache_object_is_corrupt() {
    let project = Project::new();
    project.write_src("a.txt", "a");
    project.write_src("trigger.txt", "first");
    let plugin = || {
        generator("reader", |host| {
            let output = host.read_file("a.txt").unwrap();
            let trigger = host.read_source("trigger.txt").unwrap();
            host.emit("report.txt", [output, b":".to_vec(), trigger].concat());
        })
    };

    build(&project, vec![plugin()]);
    for entry in std::fs::read_dir(project.root().join(".rpp/cache/objects")).unwrap() {
        std::fs::write(entry.unwrap().path(), "corrupt").unwrap();
    }
    project.write_src("trigger.txt", "second");

    build(&project, vec![plugin()]);
    assert_eq!(project.read_out("report.txt").as_deref(), Some("a:second"));
}

#[test]
fn same_size_source_change_with_preserved_mtime_is_rebuilt() {
    let project = Project::new();
    project.write_src("a.txt", "aa");
    build(&project, Vec::new());

    let source = project.src().join("a.txt");
    let modified = std::fs::metadata(&source).unwrap().modified().unwrap();
    std::fs::write(&source, "bb").unwrap();
    std::fs::File::open(&source)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();

    let second = build(&project, Vec::new());
    assert_eq!(second.processed, 1);
    assert_eq!(project.read_out("a.txt").as_deref(), Some("bb"));
}

#[test]
fn materialized_outputs_do_not_share_writable_cache_inodes() {
    let project = Project::new();
    project.write_src("a.txt", "same");
    project.write_src("b.txt", "same");
    build(&project, Vec::new());

    std::fs::remove_dir_all(project.root().join("dist")).unwrap();
    let warm = build(&project, Vec::new());
    assert_eq!(warm.cached, 2);

    std::fs::write(project.root().join("dist/a.txt"), "changed").unwrap();
    assert_eq!(project.read_out("b.txt").as_deref(), Some("same"));
}

#[cfg(unix)]
#[test]
fn cached_symlink_output_is_replaced_even_without_cas() {
    for remove_cas in [false, true] {
        let project = Project::new();
        project.write_src("a.txt", "same");
        build(&project, Vec::new());
        let output = project.root().join("dist/a.txt");
        std::fs::remove_file(&output).unwrap();
        std::os::unix::fs::symlink(project.src().join("a.txt"), &output).unwrap();
        if remove_cas {
            std::fs::remove_dir_all(project.root().join(".rpp/cache/objects")).unwrap();
        }
        build(&project, Vec::new());
        assert_eq!(project.read_out("a.txt").as_deref(), Some("same"));
        assert!(!std::fs::symlink_metadata(output)
            .unwrap()
            .file_type()
            .is_symlink());
    }
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
