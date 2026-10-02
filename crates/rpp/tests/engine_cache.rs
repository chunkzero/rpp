//! Incremental file cache and plugin cache keys.

mod common;

use std::sync::Arc;

use common::engine::{build, generator_factory, upper, upper_factory};
use common::mock::{cache_key, MockFactory};
use common::Project;
use rpp::model::{PluginFactory, ProcessOutcome};

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

#[test]
fn plugin_key_change_keeps_other_plugins_cached() {
    let project = Project::new();
    project.write_src("a.txt", "hello");

    let stable = Arc::new(generator_factory("stable", |host| {
        host.emit("stable.txt", b"s".to_vec())
    })) as Arc<dyn PluginFactory>;
    let other = |key: u64| {
        Arc::new(
            generator_factory("other", |host| host.emit("other.txt", b"o".to_vec())).with_key(key),
        ) as Arc<dyn PluginFactory>
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
            upper_factory("p", key)
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

    let keep = Arc::new(upper_factory("keep", 1).with_generator(|host| {
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
