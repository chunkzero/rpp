//! Generator reads, emits, and cache replay.

mod common;

use std::sync::Arc;

use common::engine::{build, generator};
use common::mock::{cache_key, MockFactory};
use common::Project;
use rpp::model::{PluginFactory, ProcessOutcome};

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
    assert_eq!(
        project.read_out("observed.txt").as_deref(),
        Some("a.txt\npack.mcmeta")
    );
    let second = build(&project, vec![plugin()]);
    assert_eq!(second.generated, 0);
    assert_eq!(
        project.read_out("observed.txt").as_deref(),
        Some("a.txt\npack.mcmeta")
    );
    assert!(second.changes.written.is_empty());
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
