//! Output sync, cleaning, and recovery from stale or damaged cache state.

mod common;

use common::engine::{build, engine, generator, noop};
use common::Project;

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
fn clean_removes_output_and_cache() {
    let project = Project::new();
    project.write_src("a.txt", "a");

    let engine = engine(&project, vec![noop()]);
    engine.build().unwrap();
    assert!(project.out_exists("a.txt"));

    rpp::engine::clean_project_artifacts(&project.config(), project.root()).unwrap();
    assert!(!project.root().join("dist").exists());
    assert!(!project.root().join(".rpp").exists());
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

#[cfg(unix)]
#[test]
fn symlinked_output_subdirectory_is_rejected() {
    let project = Project::new();
    project.write_src("a/b/x.txt", "x");
    project.write_src("a/c/y.txt", "y");
    build(&project, Vec::new());

    let outside = tempfile::tempdir().unwrap();
    let nested = project.root().join("dist/a/c");
    std::fs::remove_dir_all(&nested).unwrap();
    std::os::unix::fs::symlink(outside.path(), &nested).unwrap();
    project.write_src("a/c/y.txt", "changed");

    let error = engine(&project, Vec::new())
        .build()
        .unwrap_err()
        .to_string();
    assert!(error.contains("must not contain symlinks"), "{error}");
    assert!(std::fs::read_dir(outside.path()).unwrap().next().is_none());
}
